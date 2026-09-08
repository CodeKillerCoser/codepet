use codepet_provider_data::{
    ProviderData, ProviderDirectories, UsageQuery, CODEX_DATASET, HOOK_DATASET,
};
use serde_json::{json, Value};

fn setup() -> (tempfile::TempDir, ProviderData) {
    let root = tempfile::tempdir().unwrap();
    let data = ProviderData::default();
    data.initialize(Some(&ProviderDirectories {
        data: root.path().to_str().unwrap().into(),
        logs: root.path().join("logs").to_str().unwrap().into(),
        database_path: root.path().join("provider.sqlite").to_str().unwrap().into(),
    }))
    .unwrap();
    (root, data)
}
fn query() -> Value {
    json!({"datasetId":HOOK_DATASET,"filter":{"time":{"kind":"all"}},"aggregation":{"timeBucket":"day","timeZone":"UTC","groupBy":["model"]},"metrics":["totalTokens","inputTokens","outputTokens","cacheReadTokens","cacheWriteTokens"],"summaries":["totals","peakDaily","totalsByGroup","peakDailyByGroup"],"page":{"limit":1}})
}
fn run(data: &ProviderData, q: Value) -> Value {
    serde_json::to_value(
        data.query("test", serde_json::from_value(q).unwrap(), false)
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn corrections_are_idempotent_and_summaries_ignore_pagination() {
    let (_dir, data) = setup();
    for _ in 0..3 {
        data.record(
            "test",
            HOOK_DATASET,
            "a",
            1800,
            1800,
            Some("a"),
            [Some(15), Some(10), Some(5), Some(3), Some(2)],
        )
        .unwrap();
    }
    data.record(
        "test",
        HOOK_DATASET,
        "b",
        86400,
        1800,
        Some("b"),
        [Some(30), Some(20), Some(10), Some(0), Some(0)],
    )
    .unwrap();
    let first = run(&data, query());
    assert_eq!(first["rows"].as_array().unwrap().len(), 1);
    assert_eq!(first["summaries"]["totals"]["totalTokens"]["value"], 45);
    assert_eq!(first["summaries"]["peakDaily"]["date"], "1970-01-02");
    data.record(
        "test",
        HOOK_DATASET,
        "a",
        1800,
        1800,
        Some("a"),
        [Some(20), Some(15), Some(5), Some(3), Some(2)],
    )
    .unwrap();
    let mut second = query();
    second["page"]["cursor"] = first["nextCursor"].clone();
    let second = run(&data, second);
    assert_eq!(second["summaries"]["totals"]["totalTokens"]["value"], 45);
    assert_eq!(second["rows"][0]["modelId"], "b");
    assert_eq!(
        run(&data, query())["summaries"]["totals"]["totalTokens"]["value"],
        50
    );
}

#[test]
fn unknown_metrics_and_models_are_not_zero_and_model_filters_are_exact() {
    let (_dir, data) = setup();
    data.record(
        "test",
        HOOK_DATASET,
        "a",
        1800,
        1800,
        None,
        [None, None, Some(5), None, None],
    )
    .unwrap();
    data.record(
        "test",
        HOOK_DATASET,
        "b",
        1800,
        1800,
        Some("b"),
        [Some(30), Some(20), Some(10), Some(0), Some(0)],
    )
    .unwrap();
    let all = run(&data, query());
    assert_eq!(all["rows"][0].get("modelId"), Some(&Value::Null));
    assert_eq!(
        all["rows"][0]["values"]["inputTokens"]["value"],
        Value::Null
    );
    assert_eq!(
        all["summaries"]["totals"]["inputTokens"]["completeness"],
        "partial"
    );
    let mut q = query();
    q["filter"]["modelIds"] = json!(["b"]);
    assert_eq!(
        run(&data, q)["summaries"]["totals"]["totalTokens"]["value"],
        30
    );
}

#[test]
fn cursor_is_bound_to_filters_and_instance() {
    let (_dir, data) = setup();
    for i in 0..2 {
        data.record(
            "test",
            HOOK_DATASET,
            &i.to_string(),
            i * 86400,
            1800,
            None,
            [Some(2), Some(1), Some(1), Some(0), Some(0)],
        )
        .unwrap();
    }
    let first = run(&data, query());
    let mut q = query();
    q["page"]["cursor"] = first["nextCursor"].clone();
    let typed: UsageQuery = serde_json::from_value(q.clone()).unwrap();
    assert_eq!(
        data.query("different", typed, false).unwrap_err().code,
        "invalid_cursor"
    );
    q["aggregation"]["timeZone"] = json!("Asia/Shanghai");
    assert_eq!(
        data.query("test", serde_json::from_value(q).unwrap(), false)
            .unwrap_err()
            .code,
        "invalid_cursor"
    );
}

#[test]
fn timezone_daily_peak_sums_models_before_taking_peak() {
    let (_dir, data) = setup();
    data.record(
        "test",
        HOOK_DATASET,
        "a",
        16 * 3600,
        1800,
        Some("a"),
        [Some(10), Some(5), Some(5), Some(0), Some(0)],
    )
    .unwrap();
    data.record(
        "test",
        HOOK_DATASET,
        "b",
        17 * 3600,
        1800,
        Some("b"),
        [Some(20), Some(10), Some(10), Some(0), Some(0)],
    )
    .unwrap();
    let mut q = query();
    q["aggregation"]["timeZone"] = json!("Asia/Shanghai");
    let result = run(&data, q);
    assert_eq!(result["summaries"]["peakDaily"]["date"], "1970-01-02");
    assert_eq!(result["summaries"]["peakDaily"]["totalTokens"], 30);
}

#[test]
fn codex_rejects_fake_model_breakdowns_and_half_hour_buckets() {
    let (_dir, data) = setup();
    let mut q = query();
    q["datasetId"] = json!(CODEX_DATASET);
    let err = data
        .query("test", serde_json::from_value(q.clone()).unwrap(), true)
        .unwrap_err();
    assert_eq!(err.code, "unsupported_usage_query");
    q["aggregation"]["groupBy"] = json!([]);
    q["aggregation"]["timeBucket"] = json!("halfHour");
    q["metrics"] = json!(["totalTokens"]);
    q["summaries"] = json!([]);
    assert_eq!(
        data.query("test", serde_json::from_value(q).unwrap(), true)
            .unwrap_err()
            .code,
        "unsupported_usage_query"
    );
}

#[test]
fn reopening_preserves_usage_without_importing_legacy_json() {
    let (dir, data) = setup();
    data.record(
        "test",
        HOOK_DATASET,
        "a",
        1800,
        1800,
        None,
        [Some(2), Some(1), Some(1), Some(0), Some(0)],
    )
    .unwrap();
    std::fs::write(dir.path().join("token-usage.json"), "invalid old data").unwrap();
    drop(data);
    let reopened = ProviderData::default();
    reopened
        .initialize(Some(&ProviderDirectories {
            data: dir.path().to_str().unwrap().into(),
            logs: dir.path().join("logs").to_str().unwrap().into(),
            database_path: dir.path().join("provider.sqlite").to_str().unwrap().into(),
        }))
        .unwrap();
    assert_eq!(
        run(&reopened, query())["summaries"]["totals"]["totalTokens"]["value"],
        2
    );
}

#[test]
fn source_correction_can_move_between_model_buckets() {
    let (_dir, data) = setup();
    data.record(
        "test",
        HOOK_DATASET,
        "same",
        0,
        1800,
        Some("old"),
        [Some(2), Some(1), Some(1), Some(0), Some(0)],
    )
    .unwrap();
    data.record(
        "test",
        HOOK_DATASET,
        "same",
        86400,
        1800,
        Some("new"),
        [Some(4), Some(2), Some(2), None, None],
    )
    .unwrap();
    let r = run(&data, query());
    assert_eq!(r["rows"][0]["modelId"], "new");
    assert!(r["nextCursor"].is_null());
    assert_eq!(r["summaries"]["totals"]["totalTokens"]["value"], 4);
}

#[test]
fn requested_empty_peak_is_explicitly_null() {
    let (_dir, data) = setup();
    let result = run(&data, query());
    assert_eq!(result["summaries"].get("peakDaily"), Some(&Value::Null));
}
