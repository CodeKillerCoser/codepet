use crate::{
    runtime_gateway::tauri_bridge::ProviderHostState,
    settings::{configured_app_data_dir, load_app_settings},
};
use codepet_task_lineage::watch::{self, WatchConfig};
use codepet_task_lineage::{
    domain::{Message, Task},
    extraction::ClaudeExtractor,
    git::{self, WorkspaceFacts},
    service::{self, Snapshot},
    sources::codex::{self, CodexSource},
    stable_id,
    store::Store,
};
use serde_json::{json, Value};
use std::{path::PathBuf, time::Duration};
use tauri::{Emitter, Manager};

fn directory(provider_id: &str) -> Result<PathBuf, String> {
    let settings = load_app_settings().map_err(|e| e.to_string())?;
    Ok(configured_app_data_dir(&settings)
        .join("task-lineage/v1")
        .join(stable_id(provider_id)))
}
async fn context(host: &ProviderHostState, provider_id: &str) -> Result<Value, String> {
    host.instance_data_contexts("codex")
        .await
        .into_iter()
        .find(|(id, _, _)| id == provider_id)
        .map(|(_, _, settings)| settings)
        .ok_or("Codex instance is unavailable".into())
}
#[tauri::command]
pub(crate) async fn task_lineage_options(
    host: tauri::State<'_, ProviderHostState>,
) -> Result<Value, String> {
    let contexts = host.instance_data_contexts("codex").await;
    let sources=tauri::async_runtime::spawn_blocking(move||->Result<Vec<Value>,String>{contexts.into_iter().map(|(id,name,settings)|{
        let root=directory(&id)?;let config=watch::read(&Store::read_only(&root)?)?;
        Ok(json!({"id":id,"name":name,"directory":codex::data_directory(&settings).ok(),"watch":config}))
    }).collect()}).await.map_err(|e|e.to_string())??;
    let claude = host.runtime_view("claude").await.ok();
    Ok(
        json!({"sources":sources,"claudeExecutable":claude.and_then(|runtime|runtime.resolved_executable),"defaultModel":"haiku","budgetUsd":0.10}),
    )
}
#[tauri::command]
pub(crate) async fn task_lineage_snapshot(provider_id: String) -> Result<Snapshot, String> {
    let root = directory(&provider_id)?;
    tauri::async_runtime::spawn_blocking(move || service::snapshot(&Store::read_only(&root)?))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub(crate) async fn task_lineage_scan(
    host: tauri::State<'_, ProviderHostState>,
    provider_id: String,
) -> Result<Snapshot, String> {
    let source = CodexSource {
        home: codex::data_directory(&context(&host, &provider_id).await?)?,
    };
    let root = directory(&provider_id)?;
    tauri::async_runtime::spawn_blocking(move || service::scan(&Store::open(&root)?, &source))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub(crate) async fn task_lineage_extract(
    host: tauri::State<'_, ProviderHostState>,
    provider_id: String,
    thread_id: Option<String>,
    model: String,
) -> Result<Snapshot, String> {
    context(&host, &provider_id).await?;
    let extractor = make_extractor(&host, model).await?;
    let root = directory(&provider_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        service::extract_next(&Store::open(&root)?, &extractor, thread_id.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
}
async fn make_extractor(
    host: &ProviderHostState,
    model: String,
) -> Result<ClaudeExtractor, String> {
    if model.trim().is_empty() || model.len() > 120 {
        return Err("Model is required".into());
    }
    let runtime = host.runtime_view("claude").await?;
    let executable = runtime
        .resolved_executable
        .ok_or("请先在连接中选择可用的 Claude 运行时")?;
    let contexts = host.instance_data_contexts("claude").await;
    if contexts.len() > 1 {
        return Err("存在多个 Claude 实例，首期抽取需要唯一的 Claude 数据上下文".into());
    }
    let config = contexts
        .first()
        .and_then(|(_, _, settings)| {
            settings
                .get("dataDirectory")
                .or_else(|| settings.get("claudeConfigDir"))
        })
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    Ok(ClaudeExtractor {
        executable: PathBuf::from(executable),
        config_directory: config,
        model,
        budget_usd: 0.10,
        timeout: Duration::from_secs(90),
    })
}
#[tauri::command]
pub(crate) async fn task_lineage_watch(
    host: tauri::State<'_, ProviderHostState>,
    provider_id: String,
    config: WatchConfig,
) -> Result<WatchConfig, String> {
    context(&host, &provider_id).await?;
    let root = directory(&provider_id)?;
    tauri::async_runtime::spawn_blocking(move || watch::configure(&Store::open(&root)?, config))
        .await
        .map_err(|e| e.to_string())?
}
pub(crate) fn start_background(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(15)).await;
            let host = app.state::<ProviderHostState>().inner().clone();
            for (id, _, settings) in host.instance_data_contexts("codex").await {
                let Ok(root) = directory(&id) else {
                    continue;
                };
                let config_root = root.clone();
                let Ok(Ok(config)) = tauri::async_runtime::spawn_blocking(move || {
                    watch::read(&Store::read_only(&config_root)?)
                })
                .await
                else {
                    continue;
                };
                if !config.enabled {
                    continue;
                }
                let Ok(home) = codex::data_directory(&settings) else {
                    continue;
                };
                let extractor = if config.extract && config.remaining_jobs > 0 {
                    make_extractor(&host, config.model.clone()).await.ok()
                } else {
                    None
                };
                let result = tauri::async_runtime::spawn_blocking(move || {
                    watch::tick(
                        &Store::open(&root)?,
                        &CodexSource { home },
                        extractor
                            .as_ref()
                            .map(|e| e as &dyn codepet_task_lineage::extraction::TaskExtractor),
                    )
                })
                .await;
                let error = match result {
                    Ok(Ok(_)) => None,
                    Ok(Err(error)) => Some(error),
                    Err(error) => Some(error.to_string()),
                };
                let _ = app.emit(
                    "task-lineage-updated",
                    json!({"providerId":id,"error":error}),
                );
            }
        }
    });
}
#[tauri::command]
pub(crate) async fn task_lineage_messages(
    provider_id: String,
    thread_id: String,
) -> Result<Vec<Message>, String> {
    let root = directory(&provider_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        service::conversation_messages(&Store::read_only(&root)?, &thread_id)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub(crate) async fn task_lineage_complete(
    provider_id: String,
    task_id: String,
    revision: u64,
    completed: bool,
) -> Result<Task, String> {
    let root = directory(&provider_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        service::set_completion(&Store::open(&root)?, &task_id, revision, completed)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub(crate) async fn task_lineage_workspace(
    provider_id: String,
    thread_id: String,
) -> Result<WorkspaceFacts, String> {
    let root = directory(&provider_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let store = Store::read_only(&root)?;
        let view = service::snapshot(&store)?;
        let thread = view
            .threads
            .iter()
            .find(|t| t.id == thread_id)
            .ok_or("Thread not found")?;
        Ok(git::inspect(&thread.workspace))
    })
    .await
    .map_err(|e| e.to_string())?
}
