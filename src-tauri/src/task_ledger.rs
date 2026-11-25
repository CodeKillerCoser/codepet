//! Human actions use the same canonical ledger; MCP transports retain Agent authority.
use codepet_task_lineage::tasks::{
    Actor, CreateTaskInput, CreateTaskRunInput, EditTaskInput, UpdateTaskRunInput,
};
use serde_json::Value;
#[tauri::command]
pub(crate) async fn task_ledger_request(
    window: tauri::WebviewWindow,
    request: Value,
    runtime: tauri::State<'_, codepet_task_mcp::http::HttpRuntime>,
    app: tauri::AppHandle,
) -> Result<Value, String> {
    if window.label() != "main" {
        return Err("任务操作仅允许主窗口调用".into());
    }
    if request["op"] == "model-catalog" {
        let profile = codepet_host::model_gateway::catalog::profile(
            runtime.ledger().data_directory(),
            request["purpose"].as_str().ok_or("缺少模型用途")?,
            request["profile"].clone(),
        )?;
        return serde_json::to_value(codepet_host::model_gateway::catalog::models(profile).await?)
            .map_err(|error| error.to_string());
    }
    if request["op"] == "local-model-runtime" {
        let root = codepet_paths::PathManager::discover()
            .map_err(|error| error.to_string())?
            .workspace
            .join("tools/local-models");
        return codepet_host::local_models::request(
            root,
            request["action"].as_str().unwrap_or("status"),
            request["id"].as_str(),
        )
        .await;
    }
    if matches!(
        request["op"].as_str(),
        Some("model-settings" | "save-model-settings")
    ) {
        let data = runtime.ledger().data_directory().to_owned();
        let save = request["op"] == "save-model-settings";
        let value = request["settings"].clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            use codepet_host::model_gateway::{settings, usage};
            let settings = if save {
                settings::save(&data, value)?
            } else {
                settings::public(&settings::read(&data)?)
            };
            Ok::<_, String>(serde_json::json!({"settings":settings,"usage":usage::summary(&data)?}))
        })
        .await
        .map_err(|e| e.to_string())??;
        if save {
            runtime.recall().stop().await;
            runtime.recall().changed();
        }
        return Ok(result);
    }
    if request["op"].as_str() == Some("recall-graph") {
        return runtime.recall().graph().await;
    }
    if request["op"].as_str() == Some("recall-history") {
        return runtime.recall().history(request.clone()).await;
    }
    if request["op"].as_str() == Some("setup") {
        let mut config = crate::task_mcp_setup::configuration(
            &app,
            runtime.ledger().data_directory().to_owned(),
        )
        .await?;
        config
            .as_object_mut()
            .unwrap()
            .extend(runtime.instructions().await?.as_object().unwrap().clone());
        return Ok(config);
    }
    if request["op"].as_str() == Some("save-instructions") {
        let revision = request["expectedRevision"]
            .as_u64()
            .ok_or("缺少提示词修订号")?;
        let text = if request["text"].is_null() {
            None
        } else {
            Some(request["text"].as_str().ok_or("提示词格式无效")?.to_owned())
        };
        return runtime.save_instructions(revision, text).await;
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
            "commit-files" => serde_json::to_value(
                ledger.commit_changes(id()?, request["runId"].as_str().ok_or("缺少执行 ID")?)?,
            )
            .map_err(|e| e.to_string()),
            "change-diffs" => {
                ledger.change_diffs(id()?, request["runId"].as_str().ok_or("缺少执行 ID")?)
            }
            "file-diff" => ledger.file_diff(
                id()?,
                request["runId"].as_str().ok_or("缺少执行 ID")?,
                request["version"].as_str().ok_or("缺少差异版本")?,
                request["path"].as_str().ok_or("缺少文件路径")?,
            ),
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
            "complete-task" => value(ledger.complete_task(
                id()?,
                request["expectedRevision"].as_u64().ok_or("缺少修订号")?,
                request["reason"].as_str().ok_or("缺少完成说明")?,
            )?),
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
