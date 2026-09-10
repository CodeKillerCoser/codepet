//! Exercise installed user skills and a persistent workspace through the real CLI.
//! Synthetic text only; no native conversation is read.
use codepet_task_lineage::{
    domain::{Evidence, Message},
    extraction::{ClaudeExtractor, TaskExtractor},
    management::{ExtractionSettings, Layout, ManagedExtractor},
};
use std::{path::PathBuf, time::Duration};
fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(
        args.get(1)
            .ok_or("Expected absolute application data directory")?,
    );
    let layout = Layout::initialize(&root)?;
    if args.len() == 2 {
        println!("{}", serde_json::to_string(&layout).unwrap());
        return Ok(());
    }
    let config = ExtractionSettings {
        prompt: "将登录表单的实现与测试归入同一个任务。".into(),
        ..Default::default()
    };
    let extractor = ManagedExtractor {
        inner: ClaudeExtractor {
            executable: PathBuf::from(&args[2]),
            config_directory: None,
            model: config.model.clone(),
            budget_usd: config.budget_usd,
            timeout: Duration::from_secs(config.timeout_seconds),
        },
        layout,
        config,
    };
    let input: Vec<_> = [
        ("user", "修复登录表单键盘焦点。"),
        ("assistant", "登录框现在可以用 Tab 聚焦，测试通过。"),
    ]
    .iter()
    .enumerate()
    .map(|(index, (role, text))| Message {
        evidence: Evidence {
            event_id: format!("synthetic-{index}"),
            file: "synthetic".into(),
            byte_offset: index as u64,
            generation: 0,
        },
        thread_id: "synthetic".into(),
        role: role.to_string(),
        text: text.to_string(),
        timestamp: Some(chrono::Utc::now().to_rfc3339()),
        turn_id: None,
    })
    .collect();
    let result = extractor.extract(&input, &[])?;
    if result.extraction.tasks.len() != 1 {
        return Err("Expected exactly one synthetic task".into());
    }
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
    Ok(())
}
