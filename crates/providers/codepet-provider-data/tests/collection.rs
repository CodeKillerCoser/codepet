use codepet_provider_data::*;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct Sink(AtomicUsize);
impl ProviderEventSink for Sink {
    fn publish(&self, _: ProtocolEvent) -> Result<(), ProtocolError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
fn setup() -> (tempfile::TempDir, Arc<ProviderData>) {
    let dir = tempfile::tempdir().unwrap();
    let data = Arc::new(ProviderData::default());
    data.initialize(Some(&ProviderDirectories {
        data: dir.path().to_str().unwrap().into(),
        logs: dir.path().join("logs").to_str().unwrap().into(),
        database_path: dir.path().join("provider.sqlite").to_str().unwrap().into(),
    }))
    .unwrap();
    (dir, data)
}
fn event(raw: Value, id: &str) -> ProtocolEvent {
    ProtocolEvent::EventNotification {
        jsonrpc: "2.0".into(),
        params: ProviderNotificationEvent {
            subscription_id: "pet".into(),
            event_id: id.into(),
            received_at: 1000,
            payload: json!({"codepet_observation":{"raw":raw}})
                .as_object()
                .unwrap()
                .clone()
                .into_iter()
                .collect(),
        },
    }
}
fn total(data: &ProviderData) -> Value {
    let q=serde_json::from_value(json!({"datasetId":HOOK_DATASET,"filter":{"time":{"kind":"all"}},"aggregation":{"timeBucket":"halfHour","timeZone":"UTC","groupBy":["model"]},"metrics":["totalTokens","inputTokens","outputTokens","cacheReadTokens","cacheWriteTokens"],"summaries":["totals"]})).unwrap();
    serde_json::to_value(data.query_observed("instance", q).unwrap()).unwrap()
}

#[test]
fn stop_registers_transcript_and_incremental_reads_are_replay_safe() {
    let (dir, data) = setup();
    let path = dir.path().join("transcript.jsonl");
    let record = |id: &str, n: u64| {
        json!({"timestamp":"2026-09-01T01:02:00Z","sessionId":"session","message":{"id":id,"role":"assistant","model":"claude-test","usage":{"input_tokens":n,"output_tokens":5,"cache_read_input_tokens":3,"cache_creation_input_tokens":2}}}).to_string()+"\n"
    };
    std::fs::write(&path, record("first", 10)).unwrap();
    let downstream = Arc::new(Sink(AtomicUsize::new(0)));
    let sink = UsageSink {
        data: data.clone(),
        downstream: downstream.clone(),
        provider: "claude",
    };
    sink.publish(event(
        json!({"hook_event_name":"PreToolUse","transcript_path":path}),
        "tool",
    ))
    .unwrap();
    data.collect().unwrap();
    assert!(total(&data)["rows"].as_array().unwrap().is_empty());
    sink.publish(event(
        json!({"hook_event_name":"Stop","transcript_path":path}),
        "stop",
    ))
    .unwrap();
    data.collect().unwrap();
    data.collect().unwrap();
    assert_eq!(
        total(&data)["summaries"]["totals"]["totalTokens"]["value"],
        20
    );
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(record("second", 20).as_bytes())
        .unwrap();
    data.collect().unwrap();
    assert_eq!(
        total(&data)["summaries"]["totals"]["totalTokens"]["value"],
        50
    );
    assert_eq!(downstream.0.load(Ordering::SeqCst), 2);
}

#[test]
fn incomplete_tail_is_retried_after_append_without_losing_or_duplicating_usage() {
    let (dir, data) = setup();
    let path = dir.path().join("tail.jsonl");
    let record=json!({"timestamp":"2026-09-01T00:00:00Z","sessionId":"session","message":{"id":"same","role":"assistant","model":"m","usage":{"input_tokens":1,"output_tokens":2,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}).to_string();
    std::fs::write(&path, &record[..record.len() / 2]).unwrap();
    let sink = UsageSink {
        data: data.clone(),
        downstream: Arc::new(Sink(AtomicUsize::new(0))),
        provider: "claude",
    };
    sink.publish(event(
        json!({"hook_event_name":"Stop","transcript_path":path}),
        "stop",
    ))
    .unwrap();
    data.collect().unwrap();
    assert!(total(&data)["rows"].as_array().unwrap().is_empty());
    std::fs::write(&path, record + "\n").unwrap();
    data.collect().unwrap();
    data.collect().unwrap();
    assert_eq!(
        total(&data)["summaries"]["totals"]["totalTokens"]["value"],
        3
    );
}

#[test]
fn opencode_completed_message_survives_restart_and_duplicate_delivery() {
    let (dir, data) = setup();
    let raw = json!({"type":"message.updated","properties":{"info":{"role":"assistant","id":"m1","sessionID":"s1","providerID":"vendor","modelID":"model","time":{"completed":1800000},"tokens":{"input":10,"output":5,"reasoning":7,"cache":{"read":3,"write":2}}}}});
    let sink = UsageSink {
        data: data.clone(),
        downstream: Arc::new(Sink(AtomicUsize::new(0))),
        provider: "opencode",
    };
    sink.publish(event(raw.clone(), "first")).unwrap();
    drop(sink);
    drop(data);
    let data = Arc::new(ProviderData::default());
    data.initialize(Some(&ProviderDirectories {
        data: dir.path().to_str().unwrap().into(),
        logs: dir.path().join("logs").to_str().unwrap().into(),
        database_path: dir.path().join("provider.sqlite").to_str().unwrap().into(),
    }))
    .unwrap();
    data.collect().unwrap();
    UsageSink {
        data: data.clone(),
        downstream: Arc::new(Sink(AtomicUsize::new(0))),
        provider: "opencode",
    }
    .publish(event(raw, "duplicate-delivery"))
    .unwrap();
    data.collect().unwrap();
    let result = total(&data);
    assert_eq!(result["rows"][0]["modelId"], "vendor/model");
    assert_eq!(result["summaries"]["totals"]["totalTokens"]["value"], 27);
}

#[test]
fn codex_native_lifetime_is_not_recomputed_from_returned_days() {
    let (_dir, data) = setup();
    data.import_codex_daily("account",&json!({"summary":{"lifetimeTokens":10000,"peakDailyTokens":1000},"dailyUsageBuckets":[{"startDate":"2026-09-01","tokens":30}]})).unwrap();
    let q=serde_json::from_value(json!({"datasetId":CODEX_DATASET,"filter":{"time":{"kind":"all"}},"aggregation":{"timeBucket":"day","timeZone":"UTC","groupBy":[]},"metrics":["totalTokens"],"summaries":["totals"]})).unwrap();
    let result = serde_json::to_value(data.query("account", q, true).unwrap()).unwrap();
    assert_eq!(result["summaries"]["totals"]["totalTokens"]["value"], 30);
    assert_eq!(result["nativeAccountSummary"]["lifetimeTokens"], 10000);
}

#[test]
fn collection_wakes_immediately_and_uses_clock_aligned_half_hours() {
    tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
        let (dir,data)=setup();data.start_collection().unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let path=dir.path().join("boundaries.jsonl");
        let lines=[("before","2026-09-01T10:29:59+08:00"),("after","2026-09-01T10:30:00+08:00")].map(|(id,timestamp)|{
            json!({"timestamp":timestamp,"sessionId":"s","message":{"id":id,"role":"assistant","model":"m","usage":{"input_tokens":1,"output_tokens":2,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}).to_string()+"\n"
        }).concat();
        std::fs::write(&path,lines).unwrap();
        UsageSink{data:data.clone(),downstream:Arc::new(Sink(AtomicUsize::new(0))),provider:"claude"}.publish(event(json!({"hook_event_name":"SessionEnd","transcript_path":path}),"end")).unwrap();
        let result=tokio::time::timeout(std::time::Duration::from_secs(5),async {
            loop {let result=total(&data);if result["rows"].as_array().unwrap().len()==2{break result;}tokio::time::sleep(std::time::Duration::from_millis(20)).await;}
        }).await.expect("Hook should wake collection without a polling interval");
        assert_eq!(result["rows"][0]["bucket"]["from"],"2026-09-01T02:00:00+00:00");
        assert_eq!(result["rows"][1]["bucket"]["from"],"2026-09-01T02:30:00+00:00");
        assert_eq!(result["summaries"]["totals"]["totalTokens"]["value"],6);
    });
}
