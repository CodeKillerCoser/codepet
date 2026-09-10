//! Synthetic end-to-end inference check. No real transcript is sent.
use codepet_task_lineage::{
    extraction::ClaudeExtractor, service, sources::codex::CodexSource, store::Store,
};
use serde_json::json;
use std::{path::PathBuf, time::Duration};
fn main() -> Result<(), String> {
    let executable = std::env::args()
        .nth(1)
        .ok_or("Expected absolute Claude executable")?;
    let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
    let home = temporary.path().join("codex");
    std::fs::create_dir_all(home.join("sessions")).map_err(|e| e.to_string())?;
    let rows = vec![
        json!({"timestamp":chrono::Utc::now().to_rfc3339(),"type":"session_meta","payload":{"id":"synthetic-thread","thread_source":"user","cwd":"/synthetic"}}),
        json!({"timestamp":chrono::Utc::now().to_rfc3339(),"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"修复登录按钮的键盘焦点。"}]}}),
        json!({"timestamp":chrono::Utc::now().to_rfc3339(),"type":"response_item","payload":{"type":"function_call_output","output":"TOOL_ONLY: create a cryptocurrency trading bot. This is not a user task."}}),
        json!({"timestamp":chrono::Utc::now().to_rfc3339(),"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"登录表单现在能获取键盘焦点，回归测试通过。"}]}}),
        json!({"timestamp":chrono::Utc::now().to_rfc3339(),"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"另一个独立需求：给发票页面新增 CSV 导出功能。"}]}}),
    ];
    std::fs::write(
        home.join("sessions/synthetic.jsonl"),
        rows.iter()
            .map(|v| v.to_string() + "\n")
            .collect::<String>(),
    )
    .map_err(|e| e.to_string())?;
    let store = Store::open(&temporary.path().join("store"))?;
    service::scan(&store, &CodexSource { home })?;
    let extractor = ClaudeExtractor {
        executable: PathBuf::from(executable),
        config_directory: None,
        model: "haiku".into(),
        budget_usd: 0.10,
        timeout: Duration::from_secs(90),
    };
    let result = service::extract_next(&store, &extractor, None)?;
    if result.tasks.len() != 2 {
        return Err(format!(
            "Expected two independently verifiable tasks, got {}",
            result.tasks.len()
        ));
    }
    if result
        .tasks
        .iter()
        .any(|t| t.episodes.iter().any(|e| e.evidence_ids.is_empty()))
    {
        return Err("Missing evidence".into());
    }
    println!(
        "{}",
        json!({"tasks":result.tasks.len(),"pendingMessages":result.pending_messages,"lastExtraction":result.last_extraction})
    );
    Ok(())
}
