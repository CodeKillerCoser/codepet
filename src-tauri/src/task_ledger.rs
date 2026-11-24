//! Human actions use the same canonical ledger; Agent authority stays in the HTTP adapter.
use codepet_task_lineage::tasks::{
    Actor, CreateTaskInput, CreateTaskRunInput, EditTaskInput, UpdateTaskRunInput,
};
use serde_json::Value;
#[tauri::command]
pub(crate) async fn task_ledger_request(
    window: tauri::WebviewWindow,
    request: Value,
    runtime: tauri::State<'_, codepet_task_mcp::http::HttpRuntime>,
) -> Result<Value, String> {
    if window.label() != "main" {
        return Err("任务操作仅允许主窗口调用".into());
    }
    if request["op"].as_str() == Some("setup") {
        return runtime.configuration().await;
    }
    let ledger = runtime.ledger();
    tauri::async_runtime::spawn_blocking(move || {
        let id = || request["taskId"].as_str().ok_or("缺少任务 ID".to_owned());
        let value = |receipt| serde_json::to_value(receipt).map_err(|e| e.to_string());
        match request["op"].as_str().ok_or("缺少任务操作")? {
            "list" => Ok(Value::Array(
                ledger
                    .list()?
                    .iter()
                    .map(|t| ledger.view(t))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            "get" => ledger.view(&ledger.get(id()?)?),
            "create-task" => {
                let input: CreateTaskInput =
                    serde_json::from_value(request["input"].clone()).map_err(|e| e.to_string())?;
                value(ledger.create_task(input, Actor::human())?)
            }
            "create-task-run" => {
                let input: CreateTaskRunInput =
                    serde_json::from_value(request["input"].clone()).map_err(|e| e.to_string())?;
                value(ledger.create_task_run(input, Actor::human())?)
            }
            "update-task-run" => {
                let input: UpdateTaskRunInput =
                    serde_json::from_value(request["input"].clone()).map_err(|e| e.to_string())?;
                value(ledger.update_task_run(input, Actor::human())?)
            }
            "edit-task" => {
                let input: EditTaskInput =
                    serde_json::from_value(request["input"].clone()).map_err(|e| e.to_string())?;
                value(ledger.edit_task(input)?)
            }
            "close-run" => value(ledger.close(
                id()?,
                request["runId"].as_str().ok_or("缺少执行 ID")?,
                request["expectedRevision"].as_u64().ok_or("缺少修订号")?,
                request["accepted"].as_bool().unwrap_or(false),
                request["reason"].as_str().ok_or("缺少结束说明")?,
            )?),
            "openChat" => {
                let task = ledger.get(id()?)?;
                let conversation = request["conversationId"].as_str().ok_or("缺少对话 ID")?;
                uuid::Uuid::parse_str(conversation).map_err(|_| "对话 ID 无效")?;
                if !task.runs.iter().any(|r| {
                    r.agent_conversation.iter().any(|c| {
                        c.agent == "codex" && c.conversation_id == conversation && c.verified
                    })
                }) {
                    return Err("Codex 对话关联尚未核实".into());
                }
                open::that(format!("codex://threads/{conversation}")).map_err(|e| e.to_string())?;
                Ok(Value::Null)
            }
            _ => Err("未知任务操作".into()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
