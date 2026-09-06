//! Multiplexed message admission and stream lifetimes, independent of Provider RPC types.
mod budget;
mod io;

pub use budget::Class;
use budget::Resources;
use io::{Pipe, FrameBoundedIo, timed_read, timed_write};
use crate::generated::ProviderTransportLimits;
use super::{error::TransportError, handshake};
use futures::{future::poll_fn, io::{AsyncReadExt, AsyncWriteExt}};
use std::{pin::Pin, sync::Arc, task::{Context, Poll}, time::Duration};
use tokio::{io::{AsyncRead, AsyncWrite}, sync::{mpsc, oneshot, OwnedSemaphorePermit}, task::JoinHandle};
use tokio_util::compat::TokioAsyncReadCompatExt;

const HEADER_BYTES: usize = 10;

/// The upper message layer supplies encoding and lane policy. Encoding/decoding run under
/// transport-owned worker and byte permits; adapters must honor the supplied byte bounds.
pub trait MessageCodec: Send + Sync + 'static {
    type Message: Send + 'static;
    type Error: From<TransportError> + Send + 'static;
    const MIN_ENCODED_BYTES: usize;

    fn classify(message: &Self::Message, small_message_bytes: usize) -> Result<Class, Self::Error>;
    fn encode(message: &Self::Message, encoded_cap: usize, decoded_cap: usize) -> Result<(Vec<u8>, usize), Self::Error>;
    fn decode(frame: &[u8], decoded: usize, class: Class) -> Result<Self::Message, Self::Error>;
    /// Return Control to keep the reply on the control reserve, or None for normal classification.
    fn response_class(message: &Self::Message) -> Option<Class>;
}

fn error<C: MessageCodec>(message: impl ToString) -> C::Error { TransportError::new(message).into() }

/// Keeps receive bytes and a lane slot charged until the caller releases the decoded message.
#[derive(Debug)]
pub struct Received<M> { pub message: M, _bytes: OwnedSemaphorePermit, _slot: OwnedSemaphorePermit }
pub struct Incoming<C: MessageCodec> { stream: yamux::Stream, peer: Connection<C>, control: bool }
/// Aborting the owner also stops connection I/O; no detached transport driver survives process teardown.
pub struct Driver<E> { task: JoinHandle<Result<(), E>> }
impl<E> Driver<E> {
    pub fn abort(&self) { self.task.abort(); }
    pub fn is_finished(&self) -> bool { self.task.is_finished() }
}
impl<E> Drop for Driver<E> { fn drop(&mut self) { self.task.abort(); } }
impl<E> std::future::Future for Driver<E> {
    type Output = Result<Result<(), E>, tokio::task::JoinError>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> { Pin::new(&mut self.task).poll(cx) }
}
pub struct Connection<C: MessageCodec> {
    open: mpsc::Sender<oneshot::Sender<Result<yamux::Stream, C::Error>>>,
    send: Arc<Resources>, receive: Arc<Resources>,
    stop: tokio::sync::watch::Sender<bool>,
    codec: std::marker::PhantomData<C>,
}

impl<C: MessageCodec> Clone for Connection<C> {
    fn clone(&self) -> Self {
        Self { open: self.open.clone(), send: self.send.clone(), receive: self.receive.clone(),
            stop: self.stop.clone(), codec: std::marker::PhantomData }
    }
}

impl<C: MessageCodec> Connection<C> {
    /// Completes the bounded bootstrap before returning. The driver must live as long as the peer.
    pub async fn connect<R, W>(reader: R, writer: W, host: bool, limits: ProviderTransportLimits)
        -> Result<(Self, mpsc::Receiver<Incoming<C>>, Driver<C::Error>), C::Error>
    where R: AsyncRead + Unpin + Send + 'static, W: AsyncWrite + Unpin + Send + 'static {
        let mut io = Pipe { reader, writer }.compat();
        let remote = handshake::negotiate(&mut io, host, &limits).await?;
        let mut config = yamux::Config::default();
        // Both directions and reply streams share the connection window. Upper protocol bound is 112.
        config.set_max_num_streams(128);
        config.set_max_connection_receive_window(Some(limits.connection_window_bytes as usize));
        config.set_split_send_size(remote.max_frame_payload_bytes.min(limits.max_frame_payload_bytes) as usize);
        // A peer may send GOAWAY/EOF immediately after its final response. Drain bytes already
        // received by a stream before reporting closure, including during provider.shutdown.
        config.set_read_after_close(true);
        let io = FrameBoundedIo::new(io, limits.max_frame_payload_bytes as usize, Duration::from_millis(limits.idle_timeout_ms));
        let mut connection = yamux::Connection::new(io, config, if host { yamux::Mode::Client } else { yamux::Mode::Server });
        let (open, mut requests) = mpsc::channel::<oneshot::Sender<Result<yamux::Stream, C::Error>>>(64);
        let (incoming, receiver) = mpsc::channel(64);
        let (stop, mut stopped) = tokio::sync::watch::channel(false);
        let peer = Self { open, send: Arc::new(Resources::new(remote)), receive: Arc::new(Resources::new(limits)), stop, codec: std::marker::PhantomData };
        let driver_peer = peer.clone();
        let driver = tokio::spawn(async move {
            let mut pending = None;
            let progress = poll_fn(|cx| {
                match connection.poll_next_inbound(cx) {
                    Poll::Ready(Some(Ok(stream))) => {
                        if incoming.try_send(Incoming { stream, peer: driver_peer.clone(), control: false }).is_err() { return Poll::Ready(Err(error::<C>("incoming stream capacity exceeded"))); }
                        cx.waker().wake_by_ref();
                    }
                    Poll::Ready(Some(Err(e))) => return Poll::Ready(Err(error::<C>(e))),
                    Poll::Ready(None) => return Poll::Ready(Ok(())),
                    Poll::Pending => {}
                }
                if pending.is_none() { if let Poll::Ready(Some(sender)) = requests.poll_recv(cx) { pending = Some(sender); } }
                if pending.as_ref().is_some_and(|s| s.is_closed()) { pending.take(); cx.waker().wake_by_ref(); }
                if pending.is_some() {
                    if let Poll::Ready(result) = connection.poll_new_outbound(cx) {
                        let _ = pending.take().unwrap().send(result.map_err(error::<C>));
                        cx.waker().wake_by_ref();
                    }
                }
                Poll::Pending
            });
            let result = tokio::select! {
                result = progress => result,
                _ = stopped.changed() => {
                    tokio::time::timeout(Duration::from_secs(2), poll_fn(|cx| connection.poll_close(cx))).await.map_err(|_| error::<C>("connection close timeout"))?.map_err(error::<C>)
                }
            };
            driver_peer.stop.send_replace(true);
            result
        });
        Ok((peer, receiver, Driver { task: driver }))
    }

    pub fn close(&self) { self.stop.send_replace(true); }
    pub fn stream_timeout(&self) -> Duration { Duration::from_millis(self.send.limits.stream_timeout_ms.min(self.receive.limits.stream_timeout_ms)) }

    async fn open_stream(&self) -> Result<yamux::Stream, C::Error> {
        let (sender, receiver) = oneshot::channel();
        self.open.send(sender).await.map_err(error::<C>)?;
        receiver.await.map_err(error::<C>)?
    }

    pub async fn exchange(&self, message: C::Message) -> Result<Received<C::Message>, C::Error> {
        tokio::time::timeout(self.stream_timeout(), async {
            let class = C::classify(&message, self.send.limits.small_message_bytes as usize)?;
            let _slot = self.send.lanes[class as usize].slots.clone().acquire_owned().await.map_err(error::<C>)?;
            let mut stream = self.open_stream().await?;
            self.write_message(&mut stream, message, class).await?;
            let response = self.read_message(&mut stream).await?;
            stream.close().await.map_err(error::<C>)?;
            Ok(response)
        }).await.map_err(|_| error::<C>("stream lifetime exceeded"))?
    }

    /// Awaiting the acknowledgement preserves event delivery order without blocking RPC streams.
    pub async fn send_event(&self, message: C::Message) -> Result<(), C::Error> {
        tokio::time::timeout(self.stream_timeout(), async {
            let class = C::classify(&message, self.send.limits.small_message_bytes as usize)?;
            let _slot = self.send.lanes[class as usize].slots.clone().acquire_owned().await.map_err(error::<C>)?;
            let mut stream = self.open_stream().await?;
            self.write_message(&mut stream, message, class).await?;
            let mut ack = [0]; timed_read(&mut stream, &mut ack, self.send.idle()).await?;
            if ack != [2] { return Err(error::<C>("invalid event acknowledgement")); }
            stream.close().await.map_err(error::<C>)
        }).await.map_err(|_| error::<C>("event lifetime exceeded"))?
    }

    async fn write_message(&self, stream: &mut yamux::Stream, message: C::Message, class: Class) -> Result<(), C::Error> {
        let lane = &self.send.lanes[class as usize];
        let (encoded_cap, decoded_cap) = self.send.caps(class);
        // Charge the maximum possible transient serialized buffers before encoding, including in a cancelled worker.
        let permit = lane.bytes.clone().acquire_many_owned((encoded_cap + decoded_cap) as u32).await.map_err(error::<C>)?;
        let worker = lane.workers.clone().acquire_owned().await.map_err(error::<C>)?;
        let (frame, decoded, mut permit) = tokio::task::spawn_blocking(move || {
            let _worker = worker;
            let (frame, decoded) = C::encode(&message, encoded_cap, decoded_cap)?;
            Ok::<_, C::Error>((frame, decoded, permit))
        }).await.map_err(error::<C>)??;
        // Serialization temporaries are gone. Retain only the encoded frame charge so another
        // bulk encoder can run while this stream is waiting for network/pipe progress.
        let surplus = permit.num_permits() - frame.len();
        drop(permit.split(surplus));
        let mut header = [0; HEADER_BYTES]; header[0] = 1; header[1] = class as u8;
        header[2..6].copy_from_slice(&(frame.len() as u32).to_be_bytes());
        header[6..10].copy_from_slice(&(decoded as u32).to_be_bytes());
        timed_write(stream, &header, self.send.idle()).await?;
        let mut grant = [0]; timed_read(stream, &mut grant, self.send.idle()).await?;
        if grant != [1] { return Err(error::<C>("message admission rejected")); }
        timed_write(stream, &frame, self.send.idle()).await?;
        drop(permit);
        Ok(())
    }

    async fn read_message(&self, stream: &mut yamux::Stream) -> Result<Received<C::Message>, C::Error> {
        let mut header = [0; HEADER_BYTES];
        // Waiting for an application reply is governed by the RPC/stream deadline, not transfer idle time.
        timed_read(stream, &mut header[..1], self.stream_timeout()).await?;
        timed_read(stream, &mut header[1..], self.receive.idle()).await?;
        if header[0] != 1 { return Err(error::<C>("invalid message envelope version")); }
        let class = Class::parse(header[1])?;
        let encoded = u32::from_be_bytes(header[2..6].try_into().unwrap()) as usize;
        let decoded = u32::from_be_bytes(header[6..10].try_into().unwrap()) as usize;
        let (encoded_cap, decoded_cap) = self.receive.caps(class);
        if encoded < C::MIN_ENCODED_BYTES || encoded > encoded_cap || decoded == 0 || decoded > decoded_cap { return Err(error::<C>("declared message size exceeds receive limits")); }
        let lane = &self.receive.lanes[class as usize];
        // Fail excess streams immediately: no unbounded queues retaining decoded objects.
        let slot = lane.slots.clone().try_acquire_owned().map_err(|_| error::<C>("receive lane is full"))?;
        let bytes = tokio::time::timeout(self.receive.idle(), lane.bytes.clone().acquire_many_owned((encoded + decoded) as u32)).await.map_err(|_| error::<C>("receive budget admission timed out"))?.map_err(error::<C>)?;
        timed_write(stream, &[1], self.receive.idle()).await?;
        let mut frame = vec![0; encoded]; timed_read(stream, &mut frame, self.receive.idle()).await?;
        let worker = lane.workers.clone().acquire_owned().await.map_err(error::<C>)?;
        tokio::task::spawn_blocking(move || {
            let _worker = worker;
            let message = C::decode(&frame, decoded, class)?;
            Ok(Received { message, _bytes: bytes, _slot: slot })
        }).await.map_err(error::<C>)?
    }
}

impl<C: MessageCodec> Incoming<C> {
    pub async fn receive(&mut self) -> Result<Received<C::Message>, C::Error> {
        let message = self.peer.read_message(&mut self.stream).await?;
        self.control = C::response_class(&message.message) == Some(Class::Control);
        Ok(message)
    }
    pub async fn respond(mut self, message: C::Message) -> Result<(), C::Error> {
        self.write_response(message, None).await?;
        self.finish().await
    }

    // The message adapter decides whether an encoding failure has a protocol-specific fallback.
    pub(crate) async fn write_response(&mut self, message: C::Message, class: Option<Class>) -> Result<(), C::Error> {
        let class = if self.control { Class::Control } else {
            match class {
                Some(class) => class,
                None => C::classify(&message, self.peer.send.limits.small_message_bytes as usize)?,
            }
        };
        self.peer.write_message(&mut self.stream, message, class).await
    }

    pub(crate) async fn finish(mut self) -> Result<(), C::Error> {
        self.stream.close().await.map_err(error::<C>)
    }

    pub async fn acknowledge(mut self) -> Result<(), C::Error> {
        timed_write(&mut self.stream, &[2], self.peer.send.idle()).await?;
        self.stream.close().await.map_err(error::<C>)
    }
    /// While a request is executing, any additional inbound data or EOF cancels its waiter.
    pub async fn cancelled(&mut self) {
        let mut byte = [0]; let _ = self.stream.read(&mut byte).await;
    }
}

#[cfg(test)]
mod tests;
