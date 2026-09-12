//! cargo run -p codepet-task-lineage --example probe -- <codex-home> <isolated-store>
use codepet_task_lineage::{service, sources::codex::CodexSource, store::Store};
use std::path::PathBuf;
fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("Expected absolute Codex home and isolated store directory".into());
    }
    let source = CodexSource {
        home: PathBuf::from(&args[0]),
    };
    let store = Store::open(&PathBuf::from(&args[1]))?;
    let view = service::scan(&store, &source)?;
    println!(
        "{}",
        serde_json::json!({"threads":view.threads.len(),"linkedThreads":view.threads.iter().filter(|t|t.parent_id.is_some()).count(),"pendingMessages":view.pending_messages,"diagnostics":view.diagnostics})
    );
    Ok(())
}
