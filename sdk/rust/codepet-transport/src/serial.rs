//! Per-connection serial transport: reusable framing -> payload -> injected message codec.
//! This is the Host byte pipeline, not a Provider server runtime.
use crate::protocol::{MAX_ENCODED_MESSAGE_BYTES, MAX_DECODED_MESSAGE_BYTES, FRAME_HEADER_BYTES};
use crate::{
    generated::ProviderTransportLimits,
    payload,
    transport::frame::{ProviderFrameEncoding, ProviderFrameHeader},
    Driver, MessageCodec, Profile, Received, TransportError,
};
use std::{marker::PhantomData, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    sync::{mpsc, oneshot, watch, OwnedSemaphorePermit, Semaphore},
    task::JoinSet,
};

fn error<C: MessageCodec>(e: impl ToString) -> C::Error {
    TransportError::new(e).into()
}
struct Outgoing<C: MessageCodec> {
    message: C::Message,
    done: oneshot::Sender<Result<(), C::Error>>,
    bytes: OwnedSemaphorePermit,
}
pub struct Sender<C: MessageCodec> {
    outgoing: mpsc::Sender<Outgoing<C>>,
    stop: watch::Sender<bool>,
    codec: PhantomData<C>,
    bytes: Arc<Semaphore>,
    reservation: u32,
}
impl<C: MessageCodec> Clone for Sender<C> {
    fn clone(&self) -> Self {
        Self {
            outgoing: self.outgoing.clone(),
            stop: self.stop.clone(),
            codec: PhantomData,
            bytes: self.bytes.clone(),
            reservation: self.reservation,
        }
    }
}
impl<C: MessageCodec> Sender<C> {
    pub async fn send(&self, message: C::Message) -> Result<(), C::Error> {
        if *self.stop.borrow() {
            return Err(error::<C>("transport closed"));
        }
        let (done, result) = oneshot::channel();
        // Admission happens before queue ownership, including when the caller is cancelled.
        let bytes = self
            .bytes
            .clone()
            .acquire_many_owned(self.reservation)
            .await
            .map_err(error::<C>)?;
        self.outgoing
            .send(Outgoing {
                message,
                done,
                bytes,
            })
            .await
            .map_err(error::<C>)?;
        result.await.map_err(error::<C>)?
    }
    pub fn close(&self) {
        self.stop.send_replace(true);
    }
}

pub fn connect<C, R, W>(
    reader: R,
    mut writer: W,
    profile: Profile,
    limits: ProviderTransportLimits,
) -> Result<
    (
        Sender<C>,
        mpsc::Receiver<Received<C::Message>>,
        Driver<C::Error>,
    ),
    C::Error,
>
where
    C: MessageCodec,
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    if profile == Profile::Mux {
        return Err(error::<C>("serial pipeline cannot assemble mux profile"));
    }
    let encoded_cap = limits.max_encoded_message_bytes as usize;
    let decoded_cap = if profile == Profile::JsonRpc {
        encoded_cap.saturating_sub(1)
    } else {
        limits.max_decoded_message_bytes as usize
    };
    let reservation = encoded_cap
        .checked_add(decoded_cap)
        .ok_or_else(|| error::<C>("invalid byte budget"))?;
    if encoded_cap < 1024
        || encoded_cap > MAX_ENCODED_MESSAGE_BYTES
        || decoded_cap > MAX_DECODED_MESSAGE_BYTES
        || reservation > limits.receive_budget_bytes as usize
        || limits.receive_budget_bytes > u32::MAX as u64
    {
        return Err(error::<C>("invalid serial transport limits"));
    }
    let idle = Duration::from_millis(limits.idle_timeout_ms);
    let receive_bytes = Arc::new(Semaphore::new(limits.receive_budget_bytes as usize));
    let send_bytes = Arc::new(Semaphore::new(limits.receive_budget_bytes as usize));
    let receive_slots = Arc::new(Semaphore::new(64));
    let (outgoing, mut messages) = mpsc::channel::<Outgoing<C>>(64);
    let (incoming, receiver) = mpsc::channel(64);
    let (stop, mut stopped) = watch::channel(false);
    let sender = Sender {
        outgoing,
        stop: stop.clone(),
        codec: PhantomData,
        bytes: send_bytes,
        reservation: reservation as u32,
    };
    let driver = tokio::spawn(async move {
        let mut tasks = JoinSet::<Result<(), C::Error>>::new();
        tasks.spawn(async move {
            let mut reader = BufReader::new(reader);
            loop {
                // Charge before reading or allocating. A queued decoded message retains its charge.
                let bytes = receive_bytes
                    .clone()
                    .acquire_many_owned(reservation as u32)
                    .await
                    .map_err(error::<C>)?;
                let slot = receive_slots
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(error::<C>)?;
                let record = read_record(&mut reader, profile, encoded_cap, idle)
                    .await
                    .map_err(error::<C>)?;
                let Some(record) = record else {
                    return Ok(());
                };
                let decoded = tokio::task::spawn_blocking(move || {
                    // The worker owns the budget even when its async waiter is cancelled.
                    let mut bytes = bytes;
                    let json = match profile {
                        Profile::JsonRpc => record,
                        Profile::Zstd => {
                            payload::decode(&record, encoded_cap, decoded_cap, None, true)
                                .map_err(C::Error::from)?
                        }
                        Profile::Mux => unreachable!(),
                    };
                    let count = json.len();
                    let message = C::decode_json(&json)?;
                    drop(json);
                    let surplus = bytes.num_permits().saturating_sub(count);
                    drop(bytes.split(surplus));
                    Ok::<_, C::Error>(Received::new(message, bytes, slot))
                })
                .await
                .map_err(error::<C>)??;
                incoming.send(decoded).await.map_err(error::<C>)?;
            }
        });
        tasks.spawn(async move {
            while let Some(outgoing) = messages.recv().await {
                let encoded = tokio::task::spawn_blocking(move || {
                    let _bytes = outgoing.bytes;
                    let json = C::encode_json(&outgoing.message, decoded_cap)?;
                    let record = match profile {
                        Profile::JsonRpc => {
                            let mut bytes = json;
                            bytes.push(b'\n');
                            bytes
                        }
                        Profile::Zstd => {
                            payload::encode(json, encoded_cap, decoded_cap, true)
                                .map_err(C::Error::from)?
                                .0
                        }
                        Profile::Mux => unreachable!(),
                    };
                    Ok::<_, C::Error>((record, _bytes, outgoing.done))
                })
                .await
                .map_err(error::<C>)?;
                match encoded {
                    Ok((record, _bytes, done)) => {
                        let result = tokio::time::timeout(idle, async {
                            writer.write_all(&record).await?;
                            writer.flush().await
                        })
                        .await
                        .map_err(error::<C>)?
                        .map_err(error::<C>);
                        match result {
                            Ok(()) => {
                                let _ = done.send(Ok(()));
                            }
                            Err(e) => {
                                let _ = done.send(Err(e));
                                return Err(error::<C>("output pipe failed"));
                            }
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
            writer.shutdown().await.map_err(error::<C>)
        });
        let result = tokio::select! {
            done = tasks.join_next() => match done { Some(Ok(v)) => v, Some(Err(e))=>Err(error::<C>(e)), None=>Ok(()) },
            _ = stopped.changed() => Ok(()),
        };
        stop.send_replace(true);
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        result
    });
    Ok((sender, receiver, Driver::new(driver)))
}

/// Only framing: no decompression, JSON or business types. Arbitrary OS chunk boundaries are valid.
pub async fn read_record<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    profile: Profile,
    cap: usize,
    idle: Duration,
) -> Result<Option<Vec<u8>>, TransportError> {
    let first = reader.read_u8().await;
    let first = match first {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(TransportError::new(e)),
    };
    if profile == Profile::JsonRpc {
        let mut record = vec![first];
        if first == b'\n' {
            record.pop();
            return Ok(Some(record));
        }
        loop {
            let available = tokio::time::timeout(idle, reader.fill_buf())
                .await
                .map_err(|_| TransportError::new("partial JSON line idle timeout"))?
                .map_err(TransportError::new)?;
            if available.is_empty() {
                return Err(TransportError::new("truncated JSON line"));
            }
            let count = available
                .iter()
                .position(|&b| b == b'\n')
                .map(|n| n + 1)
                .unwrap_or(available.len());
            if count > cap.saturating_sub(record.len()) {
                return Err(TransportError::new("JSON line exceeds byte limit"));
            }
            record.extend_from_slice(&available[..count]);
            reader.consume(count);
            if record.last() == Some(&b'\n') {
                record.pop();
                return Ok(Some(record));
            }
        }
    }
    if profile != Profile::Zstd {
        return Err(TransportError::new("invalid serial framer"));
    }
    let mut header = [0u8; FRAME_HEADER_BYTES];
    header[0] = first;
    tokio::time::timeout(idle, reader.read_exact(&mut header[1..]))
        .await
        .map_err(|_| TransportError::new("partial frame header idle timeout"))?
        .map_err(TransportError::new)?;
    let parsed = ProviderFrameHeader::decode(&header, cap)?;
    if parsed.encoding != ProviderFrameEncoding::ZstdJson {
        return Err(TransportError::new("profile requires zstd-json"));
    }
    let mut record = Vec::with_capacity(FRAME_HEADER_BYTES + parsed.payload_length);
    record.extend_from_slice(&header);
    record.resize(FRAME_HEADER_BYTES + parsed.payload_length, 0);
    tokio::time::timeout(idle, reader.read_exact(&mut record[FRAME_HEADER_BYTES..]))
        .await
        .map_err(|_| TransportError::new("partial frame body idle timeout"))?
        .map_err(TransportError::new)?;
    Ok(Some(record))
}
