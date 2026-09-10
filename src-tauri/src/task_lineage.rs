use crate::{
    runtime_gateway::tauri_bridge::ProviderHostState,
    settings::{configured_app_data_dir, load_app_settings},
};
use codepet_task_lineage::{
    domain::{Message, Task},
    extraction::ClaudeExtractor,
    git::{self, WorkspaceFacts},
    management::{self, ExtractionSettings, Job, Layout, ManagedExtractor},
    service::{self, Snapshot},
    sources::codex::{self, CodexSource},
    stable_id,
    store::Store,
    watch::{self, WatchConfig},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{LazyLock, Mutex},
    time::Duration,
};
use tauri::{Emitter, Manager};

static INITIALIZED: LazyLock<Mutex<HashSet<PathBuf>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
fn layout() -> Result<Layout, String> {
    Layout::initialize(&configured_app_data_dir(
        &load_app_settings().map_err(|e| e.to_string())?,
    ))
}
fn directory(provider_id: &str) -> Result<PathBuf, String> {
    if provider_id.is_empty() {
        return Err("请选择 Codex 来源".into());
    }
    let root = layout()?
        .root
        .join("task-lineage/v1")
        .join(stable_id(provider_id));
    let mut seen = INITIALIZED.lock().map_err(|e| e.to_string())?;
    if !seen.contains(&root) {
        let store = Store::open(&root)?;
        management::recover_jobs(&store)?;
        if store
            .read::<ExtractionSettings>("extraction-settings.json")?
            .is_none()
        {
            store.write("extraction-settings.json", &ExtractionSettings::default())?;
        }
        seen.insert(root.clone());
    }
    Ok(root)
}
async fn context(host: &ProviderHostState, provider_id: &str) -> Result<Value, String> {
    host.instance_data_contexts("codex")
        .await
        .into_iter()
        .find(|(id, _, _)| id == provider_id)
        .map(|(_, _, settings)| settings)
        .ok_or("Codex 实例不可用".into())
}
async fn make_extractor(
    host: &ProviderHostState,
    provider_id: &str,
) -> Result<ManagedExtractor, String> {
    let config = management::settings(&Store::read_only(&directory(provider_id)?)?)?;
    if config.harness != "claude" {
        return Err("当前任务抽取适配器只支持 Claude harness".into());
    }
    let runtime = host.runtime_view(&config.harness).await?;
    let executable = runtime
        .resolved_executable
        .ok_or("请先在连接中选择可用的 Claude 运行时")?;
    let contexts = host.instance_data_contexts(&config.harness).await;
    let selected = if let Some(id) = &config.harness_instance_id {
        Some(
            contexts
                .iter()
                .find(|(candidate, _, _)| candidate == id)
                .ok_or("配置的摘要实例已不可用")?,
        )
    } else if contexts.len() > 1 {
        return Err("请在抽取设置中选择摘要用的 Claude 实例".into());
    } else {
        contexts.first()
    };
    let data = selected
        .and_then(|(_, _, settings)| {
            settings
                .get("dataDirectory")
                .or_else(|| settings.get("claudeConfigDir"))
        })
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    Ok(ManagedExtractor {
        inner: ClaudeExtractor {
            executable: executable.into(),
            config_directory: data,
            model: config.model.clone(),
            budget_usd: config.budget_usd,
            timeout: Duration::from_secs(config.timeout_seconds),
        },
        layout: layout()?,
        config,
    })
}
#[tauri::command]
pub(crate) async fn task_lineage_options(
    host: tauri::State<'_, ProviderHostState>,
) -> Result<Value, String> {
    let contexts = host.instance_data_contexts("codex").await;
    let sources=tauri::async_runtime::spawn_blocking(move||->Result<Vec<Value>,String>{contexts.into_iter().map(|(id,name,settings)|{let root=directory(&id)?;let store=Store::read_only(&root)?;Ok(json!({"id":id,"name":name,"directory":codex::data_directory(&settings).ok(),"watch":watch::read(&store)?,"extraction":management::settings(&store)?}))}).collect()}).await.map_err(|e|e.to_string())??;
    let claude = host.runtime_view("claude").await.ok();
    let instances = host
        .instance_data_contexts("claude")
        .await
        .into_iter()
        .map(|(id, name, _)| json!({"id":id,"name":name,"harness":"claude"}))
        .collect::<Vec<_>>();
    Ok(
        json!({"sources":sources,"claudeExecutable":claude.and_then(|r|r.resolved_executable),"defaultModel":"haiku","budgetUsd":0.25,"instances":instances,"capabilities":management::capabilities(),"layout":layout()?}),
    )
}

/// Additive local extension contract. Extraction admission returns a persisted job.
#[derive(Deserialize)]
#[serde(tag = "method", deny_unknown_fields)]
pub(crate) enum TaskRequest {
    #[serde(rename = "tasks.capabilities")]
    Capabilities,
    #[serde(rename = "tasks.list")]
    List,
    #[serde(rename = "tasks.messages")]
    Messages {
        #[serde(rename = "threadId")]
        thread_id: String,
    },
    #[serde(rename = "tasks.settings.get")]
    SettingsGet,
    #[serde(rename = "tasks.settings.set")]
    SettingsSet { config: ExtractionSettings },
    #[serde(rename = "tasks.skills")]
    Skills,
    #[serde(rename = "tasks.skill.get")]
    SkillGet { name: String },
    #[serde(rename = "tasks.dirty")]
    Dirty,
    #[serde(rename = "tasks.jobs")]
    Jobs,
    #[serde(rename = "tasks.extract")]
    Extract {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "threadId")]
        thread_id: Option<String>,
    },
    #[serde(rename = "tasks.workspace.request")]
    Workspace {
        #[serde(rename = "taskId")]
        task_id: String,
    },
}
#[tauri::command]
pub(crate) async fn task_lineage_request(
    app: tauri::AppHandle,
    host: tauri::State<'_, ProviderHostState>,
    provider_id: String,
    request: TaskRequest,
) -> Result<Value, String> {
    if matches!(&request, TaskRequest::Capabilities) {
        return Ok(management::capabilities());
    }
    context(&host, &provider_id).await?;
    let root = directory(&provider_id)?;
    if let TaskRequest::Extract {
        request_id,
        thread_id,
    } = request
    {
        let queued = tauri::async_runtime::spawn_blocking(move || {
            management::enqueue(&Store::open(&root)?, &request_id, thread_id)
        })
        .await
        .map_err(|e| e.to_string())??;
        if queued.1 {
            let host = host.inner().clone();
            let job = queued.0.clone();
            tauri::async_runtime::spawn(async move {
                run_job(app, host, provider_id, job).await;
            });
        }
        return Ok(json!(queued.0));
    }
    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        match request {
            TaskRequest::List => Ok(json!(service::snapshot(&Store::read_only(&root)?)?)),
            TaskRequest::Messages { thread_id } => Ok(json!(service::conversation_messages(
                &Store::read_only(&root)?,
                &thread_id
            )?)),
            TaskRequest::SettingsGet => Ok(json!(management::settings(&Store::read_only(&root)?)?)),
            TaskRequest::SettingsSet { config } => Ok(json!(management::save_settings(
                &Store::open(&root)?,
                config,
                &layout()?
            )?)),
            TaskRequest::Skills => Ok(json!(layout()?.list_skills()?)),
            TaskRequest::SkillGet { name } => {
                Ok(json!({"name":name,"text":layout()?.read_skill(&name)?}))
            }
            TaskRequest::Dirty => Ok(json!(service::dirty_states(&Store::read_only(&root)?)?)),
            TaskRequest::Jobs => Ok(json!(management::jobs(&Store::read_only(&root)?)?)),
            TaskRequest::Workspace { task_id } => {
                let store = Store::read_only(&root)?;
                let task: Task = store
                    .read(&Store::task_key(&task_id))?
                    .ok_or("任务不存在")?;
                Ok(json!({"path":layout()?.allocate_task(&provider_id,&task)?,"taskId":task_id}))
            }
            _ => Err("不支持的任务方法".into()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
async fn run_job(
    app: tauri::AppHandle,
    host: ProviderHostState,
    provider_id: String,
    mut job: Job,
) {
    let outcome: Result<(), String> = async {
        let root = directory(&provider_id)?;
        job.state = "running".into();
        Store::open(&root)?.write(&management::job_key(&job.id), &job)?;
        let _ = app.emit(
            "task-lineage-updated",
            json!({"providerId":provider_id,"error":null}),
        );
        let source = CodexSource {
            home: codex::data_directory(&context(&host, &provider_id).await?)?,
        };
        let extractor = make_extractor(&host, &provider_id).await?;
        let thread = job.thread_id.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let store = Store::open(&root)?;
            service::scan(&store, &source)?;
            service::extract_next(&store, &extractor, thread.as_deref()).map(|_| ())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    job.state = if outcome.is_ok() {
        "completed"
    } else {
        "failed"
    }
    .into();
    job.error = outcome.err();
    job.finished_at = Some(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64,
    );
    for _ in 0..30 {
        if directory(&provider_id)
            .and_then(|root| Store::open(&root))
            .and_then(|store| store.write(&management::job_key(&job.id), &job))
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = app.emit(
        "task-lineage-updated",
        json!({"providerId":provider_id,"error":job.error}),
    );
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
    let mut extractor = make_extractor(&host, &provider_id).await?;
    extractor.inner.model = model;
    let root = directory(&provider_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        service::extract_next(&Store::open(&root)?, &extractor, thread_id.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
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
        let mut deadlines = HashMap::<String, (std::time::Instant, u64, u64)>::new();
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let host = app.state::<ProviderHostState>().inner().clone();
            for (id, _, data) in host.instance_data_contexts("codex").await {
                let Ok(root) = directory(&id) else {
                    continue;
                };
                let loaded = (|| -> Result<_, String> {
                    let store = Store::read_only(&root)?;
                    Ok((watch::read(&store)?, management::settings(&store)?))
                })();
                let Ok((watch, config)) = loaded else {
                    continue;
                };
                if !watch.enabled {
                    deadlines.remove(&id);
                    continue;
                }
                if deadlines
                    .get(&id)
                    .is_some_and(|(time, settings_revision, watch_revision)| {
                        *time > std::time::Instant::now()
                            && *settings_revision == config.revision
                            && *watch_revision == watch.revision
                    })
                {
                    continue;
                }
                deadlines.insert(
                    id.clone(),
                    (
                        std::time::Instant::now() + Duration::from_secs(config.interval_seconds),
                        config.revision,
                        watch.revision,
                    ),
                );
                let Ok(home) = codex::data_directory(&data) else {
                    continue;
                };
                let runtime = if watch.extract && watch.remaining_jobs > 0 {
                    make_extractor(&host, &id).await.map(Some)
                } else {
                    Ok(None)
                };
                let runtime_error = runtime.as_ref().err().cloned();
                let extractor = runtime.ok().flatten();
                let tick_root = root.clone();
                let result = tauri::async_runtime::spawn_blocking(move || {
                    watch::tick(
                        &Store::open(&tick_root)?,
                        &CodexSource { home },
                        extractor
                            .as_ref()
                            .map(|e| e as &dyn codepet_task_lineage::extraction::TaskExtractor),
                    )
                })
                .await;
                let error = match result {
                    Ok(Ok(_)) => None,
                    Ok(Err(e)) => Some(runtime_error.unwrap_or(e)),
                    Err(e) => Some(e.to_string()),
                };
                if let Ok(store) = Store::open(&root) {
                    if let Ok(mut current) = watch::read(&store) {
                        if error.is_some() && current.revision == watch.revision {
                            current.last_error = error.clone();
                            current.extract = false;
                            let _ = store.write("watch.json", &current);
                        }
                        deadlines.insert(
                            id.clone(),
                            (
                                std::time::Instant::now()
                                    + Duration::from_secs(config.interval_seconds),
                                config.revision,
                                current.revision,
                            ),
                        );
                    }
                }
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
