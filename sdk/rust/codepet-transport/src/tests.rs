use super::*;
use serde_json::{json, Value};
use std::{
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::Duration,
};
use tokio::io::{AsyncWriteExt, BufReader};
struct JsonCodec;
impl MessageCodec for JsonCodec {
    type Message = Value;
    type Error = TransportError;
    const MIN_ENCODED_BYTES: usize = 10;
    fn classify(_: &Value, _: usize) -> Result<Class, TransportError> {
        Ok(Class::Normal)
    }
    fn encode_json(m: &Value, cap: usize) -> Result<Vec<u8>, TransportError> {
        let mut out = payload::BoundedBuffer::new(cap);
        serde_json::to_writer(&mut out, m).map_err(TransportError::new)?;
        Ok(out.bytes)
    }
    fn decode_json(bytes: &[u8]) -> Result<Value, TransportError> {
        serde_json::from_slice(bytes).map_err(TransportError::new)
    }
    fn encode(m: &Value, e: usize, d: usize) -> Result<(Vec<u8>, usize), TransportError> {
        payload::encode(Self::encode_json(m, d)?, e, d, false)
    }
    fn decode(frame: &[u8], d: usize, _: Class) -> Result<Value, TransportError> {
        Self::decode_json(&payload::decode(
            frame,
            16 * 1024 * 1024,
            d,
            Some(d),
            false,
        )?)
    }
    fn response_class(_: &Value) -> Option<Class> {
        None
    }
}
#[tokio::test]
async fn both_serial_profiles_reassemble_fragmented_and_coalesced_records() {
    for profile in [Profile::JsonRpc, Profile::Zstd] {
        let (pipe, mut peer) = tokio::io::duplex(16);
        let (read, write) = tokio::io::split(pipe);
        let (sender, mut incoming, _driver) =
            serial::connect::<JsonCodec, _, _>(read, write, profile, default_transport_limits())
                .unwrap();
        let expected = json!({"text":"中文\u{2028}\nUTF-8","id":"one"});
        let bytes = serde_json::to_vec(&expected).unwrap();
        let record = if profile == Profile::JsonRpc {
            let mut b = bytes;
            b.push(b'\n');
            b
        } else {
            payload::encode(bytes, 16 * 1024 * 1024, 128 * 1024 * 1024, true)
                .unwrap()
                .0
        };
        let copy = record.clone();
        let writer = tokio::spawn(async move {
            for chunk in copy.chunks(3) {
                peer.write_all(chunk).await.unwrap();
            }
            peer.write_all(&copy).await.unwrap();
            peer
        });
        assert_eq!(incoming.recv().await.unwrap().message, expected);
        assert_eq!(incoming.recv().await.unwrap().message, expected);
        let mut peer = BufReader::new(writer.await.unwrap());
        let output = expected.clone();
        let sending = tokio::spawn(async move {
            sender.send(output).await.unwrap();
        });
        let record =
            serial::read_record(&mut peer, profile, 16 * 1024 * 1024, Duration::from_secs(1))
                .await
                .unwrap()
                .unwrap();
        let decoded = if profile == Profile::JsonRpc {
            record
        } else {
            payload::decode(&record, 16 * 1024 * 1024, 128 * 1024 * 1024, None, true).unwrap()
        };
        assert_eq!(JsonCodec::decode_json(&decoded).unwrap(), expected);
        sending.await.unwrap();
    }
}
#[tokio::test]
async fn framing_rejects_oversize_truncated_and_wrong_encoding_before_decode() {
    let mut line = BufReader::new(&b"123456789\n"[..]);
    assert!(
        serial::read_record(&mut line, Profile::JsonRpc, 8, Duration::from_secs(1))
            .await
            .is_err()
    );
    let mut truncated = BufReader::new(&b"{\"id\":1}"[..]);
    assert!(serial::read_record(
        &mut truncated,
        Profile::JsonRpc,
        1024,
        Duration::from_secs(1)
    )
    .await
    .unwrap_err()
    .message
    .contains("truncated"));
    let header = transport::frame::encode_header(
        u32::MAX as usize,
        transport::frame::ProviderFrameEncoding::ZstdJson,
    );
    assert!(serial::read_record(
        &mut BufReader::new(&header[..]),
        Profile::Zstd,
        1024,
        Duration::from_secs(1)
    )
    .await
    .unwrap_err()
    .message
    .contains("exceeds"));
    let raw = transport::frame::encode_header(1, transport::frame::ProviderFrameEncoding::RawJson);
    assert!(serial::read_record(
        &mut BufReader::new(&raw[..]),
        Profile::Zstd,
        1024,
        Duration::from_secs(1)
    )
    .await
    .unwrap_err()
    .message
    .contains("requires zstd"));
}
#[test]
fn payload_limits_and_adaptive_encoding_are_independent_of_rpc_types() {
    let raw = vec![b'x'; 100];
    let (frame, n) = payload::encode(raw.clone(), 1024, 1024, false).unwrap();
    assert_eq!(frame[5], 0);
    assert_eq!(
        payload::decode(&frame, 1024, 1024, Some(n), false).unwrap(),
        raw
    );
    let large = vec![b'x'; 2 * 1024 * 1024];
    let (frame, n) =
        payload::encode(large.clone(), 16 * 1024 * 1024, 128 * 1024 * 1024, false).unwrap();
    assert_eq!(frame[5], 1);
    assert_eq!(
        payload::decode(&frame, 16 * 1024 * 1024, 128 * 1024 * 1024, Some(n), false).unwrap(),
        large
    );
    assert!(payload::decode(&frame, 16 * 1024 * 1024, 1024, None, true).is_err());
    assert!(payload::decode(
        &frame,
        16 * 1024 * 1024,
        128 * 1024 * 1024,
        Some(n - 1),
        true
    )
    .is_err());
    let (forced, _) = payload::encode(vec![b'x'; 100], 1024, 1024, true).unwrap();
    assert_eq!(forced[5], 1);
    assert!(payload::encode(vec![b'x'; 1025], 1024, 1024, true).is_err());
}
struct Gate {
    release: Mutex<bool>,
    changed: Condvar,
    entered: tokio::sync::Notify,
}
static GATE: OnceLock<Arc<Gate>> = OnceLock::new();
struct SlowCodec;
impl MessageCodec for SlowCodec {
    type Message = Value;
    type Error = TransportError;
    const MIN_ENCODED_BYTES: usize = 10;
    fn classify(m: &Value, n: usize) -> Result<Class, TransportError> {
        JsonCodec::classify(m, n)
    }
    fn encode_json(m: &Value, n: usize) -> Result<Vec<u8>, TransportError> {
        JsonCodec::encode_json(m, n)
    }
    fn decode_json(bytes: &[u8]) -> Result<Value, TransportError> {
        let m = JsonCodec::decode_json(bytes)?;
        if m["slow"] == true {
            let gate = GATE.get().unwrap();
            gate.entered.notify_one();
            let mut released = gate.release.lock().unwrap();
            while !*released {
                released = gate.changed.wait(released).unwrap();
            }
        }
        Ok(m)
    }
    fn encode(m: &Value, e: usize, d: usize) -> Result<(Vec<u8>, usize), TransportError> {
        JsonCodec::encode(m, e, d)
    }
    fn decode(f: &[u8], d: usize, c: Class) -> Result<Value, TransportError> {
        JsonCodec::decode(f, d, c)
    }
    fn response_class(_: &Value) -> Option<Class> {
        None
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_decode_in_provider_a_does_not_block_provider_b() {
    let gate = Arc::new(Gate {
        release: Mutex::new(false),
        changed: Condvar::new(),
        entered: tokio::sync::Notify::new(),
    });
    GATE.set(gate.clone()).ok().unwrap();
    struct Release(Arc<Gate>);
    impl Drop for Release {
        fn drop(&mut self) {
            *self.0.release.lock().unwrap() = true;
            self.0.changed.notify_all();
        }
    }
    let _release = Release(gate.clone());
    let (a, mut pa) = tokio::io::duplex(1024);
    let (ar, aw) = tokio::io::split(a);
    let (_a, mut events_a, _ad) =
        serial::connect::<SlowCodec, _, _>(ar, aw, Profile::JsonRpc, default_transport_limits())
            .unwrap();
    pa.write_all(b"{\"slow\":true}\n").await.unwrap();
    gate.entered.notified().await;
    let (b, mut pb) = tokio::io::duplex(1024);
    let (br, bw) = tokio::io::split(b);
    let (_b, mut events_b, _bd) =
        serial::connect::<SlowCodec, _, _>(br, bw, Profile::Zstd, default_transport_limits())
            .unwrap();
    let record = payload::encode(br#"{"id":"b"}"#.to_vec(), 1024, 1024, true)
        .unwrap()
        .0;
    pb.write_all(&record).await.unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), events_b.recv())
            .await
            .unwrap()
            .unwrap()
            .message,
        json!({"id":"b"})
    );
    assert!(events_a.try_recv().is_err());
    drop(_release);
    assert_eq!(events_a.recv().await.unwrap().message, json!({"slow":true}));
}
