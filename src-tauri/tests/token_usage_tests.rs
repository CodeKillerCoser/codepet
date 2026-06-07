use code_pet_lib::agents::AgentId;
use code_pet_lib::settings::AppSettings;
use code_pet_lib::token_usage::{refresh_transcript_usage, usage_store_path, TokenUsageStore};
use serde_json::json;

fn write_jsonl(path: &std::path::Path, rows: Vec<serde_json::Value>) {
    let text = rows
        .into_iter()
        .map(|row| serde_json::to_string(&row).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(path, format!("{text}\n")).unwrap();
}

#[test]
fn usage_store_path_follows_custom_app_data_directory() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = AppSettings::default();
    settings.data.data_directory = Some(temp.path().join("code-pet-data").to_string_lossy().to_string());

    assert_eq!(usage_store_path(&settings), temp.path().join("code-pet-data").join("token-usage.json"));
}

#[test]
fn claude_style_transcript_deduplicates_repeated_assistant_message_usage() {
    let temp = tempfile::tempdir().unwrap();
    let transcript = temp.path().join("session.jsonl");
    write_jsonl(
        &transcript,
        vec![
            json!({
                "type": "assistant",
                "sessionId": "claude-session",
                "message": {
                    "id": "msg-1",
                    "model": "claude-sonnet",
                    "usage": {
                        "input_tokens": 100,
                        "output_tokens": 20,
                        "cache_creation_input_tokens": 30,
                        "cache_read_input_tokens": 40
                    }
                }
            }),
            json!({
                "type": "assistant",
                "sessionId": "claude-session",
                "message": {
                    "id": "msg-1",
                    "model": "claude-sonnet",
                    "usage": {
                        "input_tokens": 100,
                        "output_tokens": 20,
                        "cache_creation_input_tokens": 30,
                        "cache_read_input_tokens": 40
                    }
                }
            }),
            json!({
                "type": "assistant",
                "sessionId": "claude-session",
                "message": {
                    "id": "msg-2",
                    "model": "claude-sonnet",
                    "usage": {
                        "input_tokens": 70,
                        "output_tokens": 8
                    }
                }
            }),
        ],
    );

    let mut store = TokenUsageStore::default();
    let changed = refresh_transcript_usage(&mut store, AgentId::Claude, &transcript, Some("claude-session")).unwrap();

    let summary = store.summary();
    assert!(changed);
    assert_eq!(summary.total.input_tokens, 170);
    assert_eq!(summary.total.output_tokens, 28);
    assert_eq!(summary.total.cache_creation_input_tokens, 30);
    assert_eq!(summary.total.cache_read_input_tokens, 40);
    assert_eq!(summary.sessions.len(), 1);
}

#[test]
fn codex_transcript_uses_latest_cumulative_total_without_readding_history() {
    let temp = tempfile::tempdir().unwrap();
    let transcript = temp.path().join("rollout-2026-05-27T20-57-14-019e6982-b3b9-7a93-8ac9-0bb871b552bf.jsonl");
    write_jsonl(
        &transcript,
        vec![
            json!({
                "type": "event_msg",
                "payload": {
                    "info": {
                        "total_token_usage": {
                            "input_tokens": 100,
                            "cached_input_tokens": 40,
                            "output_tokens": 10,
                            "reasoning_output_tokens": 3,
                            "total_tokens": 110
                        }
                    }
                }
            }),
            json!({
                "type": "event_msg",
                "payload": {
                    "info": {
                        "total_token_usage": {
                            "input_tokens": 250,
                            "cached_input_tokens": 100,
                            "output_tokens": 25,
                            "reasoning_output_tokens": 8,
                            "total_tokens": 275
                        }
                    }
                }
            }),
        ],
    );

    let mut store = TokenUsageStore::default();
    assert!(refresh_transcript_usage(&mut store, AgentId::Codex, &transcript, Some("codex-session")).unwrap());
    assert!(!refresh_transcript_usage(&mut store, AgentId::Codex, &transcript, Some("codex-session")).unwrap());

    let summary = store.summary();
    assert_eq!(summary.total.input_tokens, 250);
    assert_eq!(summary.total.cached_input_tokens, 100);
    assert_eq!(summary.total.output_tokens, 25);
    assert_eq!(summary.total.reasoning_output_tokens, 8);
    assert_eq!(summary.total.total_tokens, 275);
    assert_eq!(summary.sessions.len(), 1);
}

#[test]
fn codex_transcript_groups_incremental_usage_into_agent_30_minute_buckets() {
    let temp = tempfile::tempdir().unwrap();
    let transcript = temp.path().join("rollout-2026-05-27T20-57-14-019e6982-b3b9-7a93-8ac9-0bb871b552bf.jsonl");
    write_jsonl(
        &transcript,
        vec![
            json!({
                "timestamp": "2026-05-27T08:10:00Z",
                "type": "event_msg",
                "payload": {
                    "info": {
                        "total_token_usage": {
                            "input_tokens": 100,
                            "output_tokens": 20,
                            "total_tokens": 120
                        }
                    }
                }
            }),
            json!({
                "timestamp": "2026-05-27T08:35:00Z",
                "type": "event_msg",
                "payload": {
                    "info": {
                        "total_token_usage": {
                            "input_tokens": 160,
                            "output_tokens": 30,
                            "total_tokens": 190
                        }
                    }
                }
            }),
        ],
    );

    let mut store = TokenUsageStore::default();
    assert!(refresh_transcript_usage(&mut store, AgentId::Codex, &transcript, Some("codex-session")).unwrap());
    assert!(!refresh_transcript_usage(&mut store, AgentId::Codex, &transcript, Some("codex-session")).unwrap());

    let summary = store.summary();
    assert_eq!(summary.by_bucket.len(), 2);
    assert_eq!(summary.by_bucket[0].provider, AgentId::Codex);
    assert_eq!(summary.by_bucket[0].bucket_start, "2026-05-27T08:00:00+00:00");
    assert_eq!(summary.by_bucket[0].total.input_tokens, 100);
    assert_eq!(summary.by_bucket[0].total.output_tokens, 20);
    assert_eq!(summary.by_bucket[0].total.total_tokens, 120);
    assert_eq!(summary.by_bucket[1].bucket_start, "2026-05-27T08:30:00+00:00");
    assert_eq!(summary.by_bucket[1].total.input_tokens, 60);
    assert_eq!(summary.by_bucket[1].total.output_tokens, 10);
    assert_eq!(summary.by_bucket[1].total.total_tokens, 70);
}

#[test]
fn claude_style_usage_is_grouped_into_provider_30_minute_buckets() {
    let temp = tempfile::tempdir().unwrap();
    let transcript = temp.path().join("session.jsonl");
    write_jsonl(
        &transcript,
        vec![
            json!({
                "timestamp": "2026-05-27T09:12:00Z",
                "type": "assistant",
                "sessionId": "claude-session",
                "message": {
                    "id": "msg-1",
                    "usage": {
                        "input_tokens": 80,
                        "output_tokens": 12
                    }
                }
            }),
            json!({
                "timestamp": "2026-05-27T09:41:00Z",
                "type": "assistant",
                "sessionId": "claude-session",
                "message": {
                    "id": "msg-2",
                    "usage": {
                        "input_tokens": 20,
                        "output_tokens": 8
                    }
                }
            }),
        ],
    );

    let mut store = TokenUsageStore::default();
    assert!(refresh_transcript_usage(&mut store, AgentId::Claude, &transcript, Some("claude-session")).unwrap());

    let summary = store.summary();
    assert_eq!(summary.by_bucket.len(), 2);
    assert_eq!(summary.by_bucket[0].provider, AgentId::Claude);
    assert_eq!(summary.by_bucket[0].bucket_start, "2026-05-27T09:00:00+00:00");
    assert_eq!(summary.by_bucket[0].total.total_tokens, 92);
    assert_eq!(summary.by_bucket[1].bucket_start, "2026-05-27T09:30:00+00:00");
    assert_eq!(summary.by_bucket[1].total.total_tokens, 28);
}

#[test]
fn existing_session_without_buckets_is_refreshed_even_when_file_fingerprint_is_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let transcript = temp.path().join("session.jsonl");
    write_jsonl(
        &transcript,
        vec![json!({
            "timestamp": "2026-05-27T10:12:00Z",
            "type": "assistant",
            "sessionId": "claude-session",
            "message": {
                "id": "msg-1",
                "usage": {
                    "input_tokens": 40,
                    "output_tokens": 6
                }
            }
        })],
    );

    let mut store = TokenUsageStore::default();
    assert!(refresh_transcript_usage(&mut store, AgentId::Claude, &transcript, Some("claude-session")).unwrap());
    for session in store.sessions.values_mut() {
        session.buckets.clear();
    }

    assert!(refresh_transcript_usage(&mut store, AgentId::Claude, &transcript, Some("claude-session")).unwrap());

    let summary = store.summary();
    assert_eq!(summary.by_bucket.len(), 1);
    assert_eq!(summary.by_bucket[0].bucket_start, "2026-05-27T10:00:00+00:00");
    assert_eq!(summary.by_bucket[0].total.total_tokens, 46);
}
