//! Host-owned recall. Canonical ledger reads never wait for model extraction.
pub mod bundle;
mod preparation;
mod process;
mod snapshot;
use codepet_model_gateway::settings;
use codepet_task_lineage::tasks::{FindTaskRunsInput, Ledger};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Duration,
};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

pub struct Recall {
    ledger: Ledger,
    process: AsyncMutex<Option<Arc<process::Process>>>,
    preparation: Mutex<Option<preparation::Preparation>>,
    startup: Mutex<CancellationToken>,
}
fn ledger_key(path: &Path) -> PathBuf {
    // On first launch the data directory may not exist yet. Resolve its existing
    // ancestor so creating it later cannot bypass the shared preparation gate.
    for ancestor in path.ancestors() {
        if let Ok(canonical) = ancestor.canonicalize() {
            return canonical.join(path.strip_prefix(ancestor).unwrap());
        }
    }
    path.to_owned()
}
impl Recall {
    pub fn shared(ledger: &Ledger) -> Arc<Self> {
        static SERVICES: OnceLock<Mutex<HashMap<PathBuf, Weak<Recall>>>> = OnceLock::new();
        let mut services = SERVICES.get_or_init(Default::default).lock().unwrap();
        let key = ledger_key(ledger.data_directory());
        if let Some(service) = services.get(&key).and_then(Weak::upgrade) {
            return service;
        }
        let service = Arc::new(Self {
            ledger: ledger.clone(),
            process: AsyncMutex::new(None),
            preparation: Mutex::new(None),
            startup: Mutex::new(CancellationToken::new()),
        });
        services.insert(key, Arc::downgrade(&service));
        service
    }
    pub(crate) fn prepare(
        &self,
        executor: &tokio::runtime::Handle,
        install: impl std::future::Future<Output = Result<(), String>> + Send + 'static,
    ) {
        *self.preparation.lock().unwrap() =
            Some(preparation::Preparation::start(executor, install));
    }
    pub fn watch(
        self: &Arc<Self>,
        executor: &tokio::runtime::Handle,
    ) -> tokio::task::JoinHandle<()> {
        let weak = Arc::downgrade(self);
        executor.spawn(async move {
            loop {
                let Some(service) = weak.upgrade() else {
                    break;
                };
                let _ = service.sync().await;
                drop(service);
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        })
    }
    pub fn changed(self: &Arc<Self>) {
        let service = self.clone();
        tokio::spawn(async move {
            let _ = service.sync().await;
        });
    }
    async fn request(&self, path: &str, body: Option<Value>) -> Result<Value, String> {
        let startup = self.startup.lock().unwrap().clone();
        let preparation = self.preparation.lock().unwrap().clone();
        if let Some(preparation) = preparation {
            tokio::select! {
                biased;
                _ = startup.cancelled() => return Err("任务召回启动已取消".into()),
                result = preparation.wait() => result?,
            }
        }
        let mut process = self.process.lock().await;
        if startup.is_cancelled() {
            return Err("任务召回启动已取消".into());
        }
        // A request queued behind startup may outlive a settings change.
        let config = settings::read(self.ledger.data_directory())?;
        let hash = process::fingerprint(&config);
        if process.as_ref().is_some_and(|p| p.fingerprint != hash) {
            process.take().unwrap().shutdown().await;
        }
        if process.is_none() {
            let active = tokio::select! {
                biased;
                _ = startup.cancelled() => return Err("任务召回启动已取消".into()),
                result = process::Process::start(self.ledger.data_directory(), config) => result?,
            };
            *process = Some(Arc::new(active));
        }
        let active = process.as_ref().unwrap().clone();
        drop(process);
        let result = active.request(path, body).await;
        // A busy graph (409) or application error must not kill an active
        // ingestion owner. Restart only when the service itself is unreachable.
        if result.is_err()
            && !matches!(
                tokio::time::timeout(Duration::from_secs(1), active.request("/health", None)).await,
                Ok(Ok(_))
            )
        {
            let mut process = self.process.lock().await;
            if process.as_ref().is_some_and(|p| Arc::ptr_eq(p, &active)) {
                *process = None;
                active.shutdown().await;
            }
        }
        result
    }
    async fn documents(&self) -> Result<Vec<Value>, String> {
        let ledger = self.ledger.clone();
        tokio::task::spawn_blocking(move || snapshot::documents(&ledger))
            .await
            .map_err(|e| e.to_string())?
    }
    pub async fn sync(&self) -> Result<Value, String> {
        // Cross-process snapshot ordering: a delayed old host cannot overwrite a
        // newer sync or delete Runs created since its snapshot. No ledger lock is held.
        let root = self.ledger.data_directory().join("tasks/recall");
        let _lock = tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
            let file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(root.join("sync.lock"))
                .map_err(|e| e.to_string())?;
            fs2::FileExt::lock_exclusive(&file).map_err(|e| e.to_string())?;
            Ok::<_, String>(file)
        })
        .await
        .map_err(|e| e.to_string())??;
        let docs = self.documents().await?;
        self.request("/sync", Some(json!({"documents":docs}))).await
    }
    pub async fn stop(&self) {
        // Cancel a pending handshake/install wait before acquiring its lock.
        // New requests stay cancelled until the previous owner has exited.
        self.startup.lock().unwrap().cancel();
        let mut process = self.process.lock().await;
        if let Some(active) = process.take() {
            active.shutdown().await;
        }
        *self.startup.lock().unwrap() = CancellationToken::new();
    }
    pub async fn graph(&self) -> Result<Value, String> {
        self.graph_preview(true).await
    }
    pub async fn graph_preview(&self, semantic: bool) -> Result<Value, String> {
        self.request("/graph", Some(json!({"semantic":semantic})))
            .await
    }
    /// Main-window history and queue controls; not exposed as an Agent MCP tool.
    pub async fn history(&self, input: Value) -> Result<Value, String> {
        let action = input["action"].as_str().unwrap_or("list");
        if !matches!(action, "list" | "detail" | "cancel" | "retry" | "enqueue") {
            return Err("未知加工历史操作".into());
        }
        if action == "enqueue" {
            for key in ["taskId", "operationId"] {
                let id = input[key].as_str().ok_or("缺少任务或操作 ID")?;
                uuid::Uuid::parse_str(id).map_err(|_| "任务或操作 ID 无效")?;
            }
            if !settings::read(self.ledger.data_directory())?.enabled {
                return Err("请先在设置 → 模型配置中启用任务召回并配置模型".into());
            }
            let documents = self.documents().await?;
            if !documents
                .iter()
                .any(|doc| doc["taskId"] == input["taskId"] && doc["eligible"] == true)
            {
                return Err("任务没有可加工的已结束执行，请先完成开发".into());
            }
            self.sync().await?;
        } else if action != "list" {
            let id = input["jobId"].as_str().ok_or("缺少加工记录 ID")?;
            uuid::Uuid::parse_str(id).map_err(|_| "加工记录 ID 无效")?;
        }
        let mut result = self
            .request("/history", Some(input.clone()))
            .await
            .map_err(|error| {
                if error.contains("(409)") {
                    "加工队列已变化，请刷新后重试".to_owned()
                } else if error.contains("(404)") {
                    "召回组件需要更新，请重新启动新版 Code Pet".to_owned()
                } else {
                    error
                }
            })?;
        if matches!(action, "list" | "detail") {
            let ids = if action == "detail" {
                vec![result["id"].as_str().unwrap_or_default().to_owned()]
            } else {
                result["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|r| r["id"].as_str().map(str::to_owned))
                    .collect()
            };
            let data = self.ledger.data_directory().to_owned();
            let usage = tokio::task::spawn_blocking(move || {
                codepet_model_gateway::usage::for_jobs(&data, &ids)
            })
            .await
            .map_err(|e| e.to_string())??;
            if action == "detail" {
                result["usage"] = usage[result["id"].as_str().unwrap_or_default()].clone();
            } else if let Some(items) = result["items"].as_array_mut() {
                for item in items {
                    item["usage"] = usage[item["id"].as_str().unwrap_or_default()].clone();
                }
            }
        }
        Ok(result)
    }
    pub async fn find(&self, input: FindTaskRunsInput) -> Result<Value, String> {
        let query = input.query.as_deref().unwrap_or("").trim().to_owned();
        if input.task_id.is_some()
            || input.run_id.is_some()
            || query.is_empty()
            || input.activity_after.is_some()
            || input.activity_before.is_some()
            || input.activity_limit.is_some()
        {
            let ledger = self.ledger.clone();
            return tokio::task::spawn_blocking(move || ledger.find_runs(input))
                .await
                .map_err(|e| e.to_string())?;
        }
        let result = async {
            self.sync().await?;
            self.request("/query", Some(json!({"query":query}))).await
        }
        .await;
        match result {
            Ok(result) => {
                // Re-read after retrieval: deleted/superseded sources cannot escape through cached hits.
                let documents = self.documents().await?;
                Ok(merge(result, &documents, input.workspace.as_deref()))
            }
            Err(error) => {
                let ledger = self.ledger.clone();
                let mut result = tokio::task::spawn_blocking(move || ledger.find_runs(input))
                    .await
                    .map_err(|e| e.to_string())??;
                result["recall"] =
                    json!({"state":"unavailable","routes":["substring"],"reason":error});
                Ok(result)
            }
        }
    }
}
fn merge(result: Value, documents: &[Value], workspace: Option<&str>) -> Value {
    let mut rows = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut recent = documents.iter().collect::<Vec<_>>();
    recent.sort_by_key(|doc| std::cmp::Reverse(doc["updatedAt"].as_i64().unwrap_or(0)));
    for hit in result["hits"].as_array().into_iter().flatten() {
        let Some(doc) = documents
            .iter()
            .find(|doc| doc["runId"] == hit["runId"] && doc["contentHash"] == hit["contentHash"])
        else {
            continue;
        };
        if !seen.insert(doc["runId"].as_str().unwrap_or_default()) {
            continue;
        }
        let mut row = doc["summary"].clone();
        let mut score = hit["score"].as_f64().unwrap_or(0.0);
        if workspace.is_some_and(|p| {
            doc["workspace"].as_str().is_some_and(|other| {
                let normalized = |value: &str| {
                    std::fs::canonicalize(value).unwrap_or_else(|_| PathBuf::from(value))
                };
                normalized(p) == normalized(other)
            })
        }) {
            score += 0.05 / 11.0;
        }
        if let Some(rank) = recent.iter().position(|d| d["runId"] == doc["runId"]) {
            score += 0.04 / (11 + rank) as f64;
        }
        row["recall"] = json!({"score":score,"evidence":hit["evidence"]});
        rows.push(row);
    }
    rows.sort_by(|a, b| {
        b["recall"]["score"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&a["recall"]["score"].as_f64().unwrap_or(0.0))
    });
    let total = rows.len();
    rows.truncate(20);
    json!({"total":total,"truncated":total>20,"runs":rows,"recall":result["recall"]})
}
#[cfg(test)]
mod tests;
