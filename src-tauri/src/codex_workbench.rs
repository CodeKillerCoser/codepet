//! Desktop adapter. Every action resolves an ID against the current scan.
use codepet_task_lineage::{sources::local, workbench};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        LazyLock, Mutex,
    },
};
use tauri::Emitter;

static GENERATION: AtomicU64 = AtomicU64::new(0);
static SNAPSHOT: LazyLock<Mutex<Option<(u64, Value)>>> = LazyLock::new(|| Mutex::new(None));

fn preferences() -> Result<PathBuf, String> {
    Ok(crate::paths::current()
        .map_err(|e| e.to_string())?
        .data
        .join("codex-workbench-selection.json"))
}

fn resolve(request: &Value) -> Result<Value, String> {
    let guard = SNAPSHOT.lock().map_err(|e| e.to_string())?;
    let (generation, snapshot) = guard.as_ref().ok_or("请先确认并扫描项目")?;
    if request["scanId"].as_u64() != Some(*generation)
        || GENERATION.load(Ordering::SeqCst) != *generation
    {
        return Err("扫描已过期，请使用最新结果".into());
    }
    snapshot["worktrees"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == request["worktreeId"])
        .cloned()
        .ok_or("工作树不在当前扫描范围内".into())
}

fn recheck(request: &Value, root: &std::path::Path) -> Result<Value, String> {
    let facts = workbench::inspect(root);
    let mut guard = SNAPSHOT.lock().map_err(|e| e.to_string())?;
    let (id, snapshot) = guard.as_mut().ok_or("扫描已过期")?;
    if Some(*id) != request["scanId"].as_u64() || GENERATION.load(Ordering::SeqCst) != *id {
        return Err("扫描已过期".into());
    }
    let row = snapshot["worktrees"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|r| r["id"] == request["worktreeId"])
        .ok_or("工作树已失效")?;
    row["git"] = facts;
    Ok(row.clone())
}

fn scoped_action(op: &str, request: &Value) -> Result<Value, String> {
    let row = resolve(request)?;
    let root = PathBuf::from(row["path"].as_str().ok_or("工作树目录无效")?);
    if op == "recheck" {
        return recheck(request, &root);
    }
    if !root.is_dir() {
        return Err("工作树目录已不存在".into());
    }
    if root.canonicalize().map_err(|e| e.to_string())? != root {
        return Err("工作树路径已变化，请重新扫描".into());
    }
    match op {
        "openDirectory" => open::that(&root).map_err(|e| e.to_string())?,
        "openTerminal" => crate::platform::workbench_actions::open_terminal(&root)?,
        "openCodexChat" => {
            let chat = row["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == request["conversationId"] && c["source"] == "Codex")
                .ok_or("对话不在工作树中")?;
            let id = chat["id"].as_str().unwrap();
            uuid::Uuid::parse_str(id).map_err(|_| "对话 ID 无效")?;
            open::that(format!("codex://threads/{id}")).map_err(|e| e.to_string())?;
        }
        _ => {
            let change = row["git"]["changes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == request["changeId"])
                .ok_or("改动条目不在当前扫描中")?;
            if op == "preview" {
                return workbench::preview(
                    &root,
                    change,
                    row["git"]["head"].as_str().unwrap_or_default(),
                    request["version"].as_str().unwrap_or("workspace"),
                );
            }
            crate::platform::workbench_actions::reveal(&workbench::change_path(&root, change)?)?;
        }
    }
    Ok(Value::Null)
}

#[tauri::command]
pub(crate) async fn codex_workbench_request(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    request: Value,
) -> Result<Value, String> {
    if window.label() != "main" {
        return Err("工作台操作仅允许主窗口调用".into());
    }
    let op = request["op"].as_str().ok_or("缺少工作台操作")?.to_owned();
    // Reserve before yielding so an old scan can never replace a newer scan.
    let generation = if op == "scan" {
        GENERATION.fetch_add(1, Ordering::SeqCst) + 1
    } else {
        0
    };
    // Cancellation is synchronous with command admission, before a new scan can start.
    if op == "cancel" {
        GENERATION.fetch_add(1, Ordering::SeqCst);
        return Ok(Value::Null);
    }
    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        match op.as_str() {
            "projects" => workbench::projects(&local::directory(local::CODEX_ID)?),
            "preferences" => Ok(std::fs::read(preferences()?).ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok()).unwrap_or(json!([]))),
            "saveSelection" => {
                let ids = request["projectIds"].as_array().ok_or("项目列表无效")?;
                if ids.len() > 500 || ids.iter().any(|v| v.as_str().is_none()) { return Err("项目列表无效".into()); }
                let path = preferences()?;
                std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
                let temp = path.with_extension("tmp");
                std::fs::write(&temp, serde_json::to_vec(ids).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                std::fs::rename(temp, path).map_err(|e| e.to_string())?;
                Ok(Value::Null)
            }
            "scan" => {
                let selected: Vec<String> = serde_json::from_value(request["projectIds"].clone()).map_err(|e| e.to_string())?;
                if selected.is_empty() { return Err("请选择至少一个项目".into()); }
                let mut snapshot = workbench::scan(&local::directory(local::CODEX_ID)?, &selected, |checked, total| {
                    if GENERATION.load(Ordering::SeqCst) == generation {
                        let _ = app.emit("codex-workbench-progress", json!({"scanId":generation,"checked":checked,"total":total,"phase":"git"}));
                    }
                    GENERATION.load(Ordering::SeqCst) == generation
                })?;
                let mut guard = SNAPSHOT.lock().map_err(|e| e.to_string())?;
                if GENERATION.load(Ordering::SeqCst) != generation { return Err("扫描已被新任务取代".into()); }
                snapshot["scanId"] = json!(generation);
                *guard = Some((generation, snapshot.clone()));
                Ok(snapshot)
            }
            "recheck" | "preview" | "openDirectory" | "openTerminal" | "revealChange" | "openCodexChat" => scoped_action(&op, &request),
            _ => Err("未知工作台操作".into()),
        }
    }).await.map_err(|e| e.to_string())?
}
