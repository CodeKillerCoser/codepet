//! Human-action adapter. Agent ingress is confined to the independent stdio MCP.
use codepet_task_lineage::tasks::{task_view, Actor, Ledger, SaveInput};
use serde_json::{json, Value};

#[tauri::command]
pub(crate) async fn task_ledger_request(
    window: tauri::WebviewWindow,
    request: Value,
) -> Result<Value, String> {
    if window.label() != "main" {
        return Err("任务操作仅允许主窗口调用".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let ledger=Ledger::new(&crate::paths::current().map_err(|e|e.to_string())?.data);
        let op=request["op"].as_str().ok_or("缺少任务操作")?;
        match op {
            "list" => Ok(Value::Array(ledger.list()?.iter().map(task_view).collect::<Result<Vec<_>,_>>()?)),
            "get" => task_view(&ledger.get(request["taskId"].as_str().ok_or("缺少任务 ID")?)?),
            "save" => {
                let input:SaveInput=serde_json::from_value(request["input"].clone()).map_err(|e|e.to_string())?;
                serde_json::to_value(ledger.save(input,Actor::Human)?).map_err(|e|e.to_string())
            }
            "close" => serde_json::to_value(ledger.close(request["taskId"].as_str().ok_or("缺少任务 ID")?,request["expectedRevision"].as_u64().ok_or("缺少任务修订号")?,request["outcome"].as_str().ok_or("缺少结束结果")?,request["reason"].as_str().ok_or("缺少结束说明")?)?).map_err(|e|e.to_string()),
            "setup" => {
                let executable=std::env::current_exe().map_err(|e|e.to_string())?;
                let command=serde_json::to_string(&executable.to_string_lossy()).map_err(|e|e.to_string())?;
                Ok(json!({"config":format!("[mcp_servers.codepet_tasks]\ncommand = {command}\nargs = [\"--task-mcp\"]\n"),"instructions":include_str!("../../crates/codepet-task-mcp/instructions.md")}))
            }
            "openChat" => {
                let task=ledger.get(request["taskId"].as_str().ok_or("缺少任务 ID")?)?;
                let thread=request["threadId"].as_str().ok_or("缺少对话 ID")?;
                uuid::Uuid::parse_str(thread).map_err(|_|"对话 ID 无效")?;
                if !task.rounds.iter().any(|r|r.sources.iter().any(|s|s.thread_id==thread&&s.verified)) { return Err("对话尚未核实与此任务的关联".into()); }
                open::that(format!("codex://threads/{thread}")).map_err(|e|e.to_string())?;
                Ok(Value::Null)
            }
            _=>Err("未知任务操作".into()),
        }
    }).await.map_err(|e|e.to_string())?
}
