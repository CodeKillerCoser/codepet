//! One local-model environment interface. Native discovery/installation stays behind platform modules.
mod installer;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod platform;
#[cfg(not(target_os = "macos"))]
#[path = "unsupported.rs"]
mod platform;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio::sync::watch;

pub const ENDPOINT: &str = "http://127.0.0.1:11234/v1";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Installation {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    pub source: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub id: String,
    pub state: String,
    pub stage: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub version: String,
    pub can_cancel: bool,
    pub error: Option<String>,
}
impl Progress {
    fn active(&self) -> bool {
        matches!(
            self.state.as_str(),
            "preparing" | "downloading" | "verifying" | "installing" | "cancelling"
        )
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub supported: bool,
    pub reason: Option<String>,
    pub installation: Option<Installation>,
    pub endpoint: &'static str,
    pub job: Option<Progress>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub id: String,
    pub version: String,
    pub bytes: u64,
    pub source_url: String,
    #[serde(skip)]
    url: String,
    #[serde(skip)]
    digest: String,
    #[serde(skip)]
    created: Option<Instant>,
}
struct Job {
    progress: Progress,
    cancel: watch::Sender<bool>,
}
struct State {
    plan: Option<Plan>,
    job: Option<Job>,
}
pub(super) struct Manager {
    root: PathBuf,
    state: Mutex<State>,
}
impl Manager {
    fn shared(root: PathBuf) -> Arc<Self> {
        static MANAGERS: OnceLock<Mutex<HashMap<PathBuf, Arc<Manager>>>> = OnceLock::new();
        let mut managers = MANAGERS.get_or_init(Default::default).lock().unwrap();
        managers
            .entry(root.clone())
            .or_insert_with(|| {
                let previous = std::fs::read(root.join("installation.json"))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Progress>(&bytes).ok())
                    .map(|mut progress| {
                        if progress.active() {
                            progress.state = "failed".into();
                            progress.stage = "上次安装已中断".into();
                            progress.error = Some("请重试安装，已有系统应用不会受影响".into());
                            progress.can_cancel = false;
                        }
                        Job {
                            progress,
                            cancel: watch::channel(false).0,
                        }
                    });
                Arc::new(Self {
                    root,
                    state: Mutex::new(State {
                        plan: None,
                        job: previous,
                    }),
                })
            })
            .clone()
    }
    async fn status(&self) -> Result<Status, String> {
        let (supported, reason) = platform::support().await;
        let installation = platform::detect(&self.root).await?;
        let job = self
            .state
            .lock()
            .unwrap()
            .job
            .as_ref()
            .map(|job| job.progress.clone());
        Ok(Status {
            supported,
            reason,
            installation,
            endpoint: ENDPOINT,
            job,
        })
    }
    pub(super) fn progress(&self, id: &str, state: &str, stage: &str, downloaded: u64) {
        let mut guard = self.state.lock().unwrap();
        if let Some(job) = guard.job.as_mut().filter(|job| job.progress.id == id) {
            if job.progress.state != "cancelling" {
                job.progress.state = state.into();
                job.progress.stage = stage.into();
            }
            job.progress.downloaded_bytes = downloaded;
        }
    }
    pub(super) fn commit(&self, id: &str) -> Result<(), String> {
        let mut guard = self.state.lock().unwrap();
        let job = guard
            .job
            .as_mut()
            .filter(|job| job.progress.id == id)
            .ok_or("安装记录已失效")?;
        if *job.cancel.borrow() {
            return Err(installer::CANCELLED.into());
        }
        job.progress.can_cancel = false;
        job.progress.stage = "正在完成安装".into();
        Ok(())
    }
    async fn start(self: Arc<Self>, plan_id: &str) -> Result<Status, String> {
        if platform::detect(&self.root).await?.is_some() {
            return self.status().await;
        }
        let plan = {
            let mut state = self.state.lock().unwrap();
            if state.job.as_ref().is_some_and(|job| job.progress.active()) {
                return Err("已有安装正在进行".into());
            }
            let plan = state
                .plan
                .as_ref()
                .filter(|plan| {
                    plan.id == plan_id
                        && plan
                            .created
                            .is_some_and(|created| created.elapsed() < Duration::from_secs(900))
                })
                .ok_or("安装确认已过期，请重新选择安装")?
                .clone();
            state.plan = None;
            plan
        };
        let lease = installer::lease(&self.root)?;
        let id = uuid::Uuid::new_v4().to_string();
        let (cancel, rx) = watch::channel(false);
        let progress = Progress {
            id: id.clone(),
            state: "preparing".into(),
            stage: "准备安装".into(),
            downloaded_bytes: 0,
            total_bytes: plan.bytes,
            version: plan.version.clone(),
            can_cancel: true,
            error: None,
        };
        installer::persist(&self.root, &progress)?;
        self.state.lock().unwrap().job = Some(Job { progress, cancel });
        let manager = self.clone();
        tokio::spawn(async move {
            let _lease = lease;
            let result = installer::run(manager.clone(), &id, plan, rx).await;
            let mut guard = manager.state.lock().unwrap();
            if let Some(job) = guard.job.as_mut().filter(|job| job.progress.id == id) {
                job.progress.can_cancel = false;
                match result {
                    Ok(()) => {
                        job.progress.state = "completed".into();
                        job.progress.stage = "安装完成".into();
                    }
                    Err(error) if error == installer::CANCELLED => {
                        job.progress.state = "cancelled".into();
                        job.progress.stage = "已取消安装".into();
                    }
                    Err(error) => {
                        job.progress.state = "failed".into();
                        job.progress.stage = "安装失败".into();
                        job.progress.error = Some(error);
                    }
                }
                let _ = installer::persist(&manager.root, &job.progress);
            }
        });
        self.status().await
    }
}

/// Called only after the Tauri main-window guard. No Agent MCP installation endpoint exists.
pub async fn request(root: PathBuf, action: &str, id: Option<&str>) -> Result<Value, String> {
    let manager = Manager::shared(root);
    match action {
        "status" => serde_json::to_value(manager.status().await?).map_err(|e| e.to_string()),
        "prepare" => {
            let status = manager.status().await?;
            if status.installation.is_some() {
                return Err("已检测到 MLX，直接使用现有应用即可".into());
            }
            if !status.supported {
                return Err(status
                    .reason
                    .unwrap_or_else(|| "当前系统不支持 MLX 安装".into()));
            }
            let plan = installer::prepare().await?;
            manager.state.lock().unwrap().plan = Some(plan.clone());
            serde_json::to_value(plan).map_err(|e| e.to_string())
        }
        "install" => serde_json::to_value(manager.start(id.ok_or("缺少安装确认 ID")?).await?)
            .map_err(|e| e.to_string()),
        "cancel" => {
            {
                let mut state = manager.state.lock().unwrap();
                let job = state
                    .job
                    .as_mut()
                    .filter(|job| Some(job.progress.id.as_str()) == id && job.progress.active())
                    .ok_or("安装已结束或记录已变化")?;
                if !job.progress.can_cancel {
                    return Err("正在完成安装，请稍候".into());
                }
                job.cancel.send_replace(true);
                job.progress.state = "cancelling".into();
                job.progress.stage = "正在取消并清理临时文件".into();
                job.progress.can_cancel = false;
            }
            serde_json::to_value(manager.status().await?).map_err(|e| e.to_string())
        }
        "open" => {
            let installation = platform::detect(&manager.root)
                .await?
                .ok_or("未找到已安装的 MLX")?;
            platform::open(&installation.path).await?;
            serde_json::to_value(manager.status().await?).map_err(|e| e.to_string())
        }
        _ => Err("未知本地模型环境操作".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn running(root: PathBuf) -> (Arc<Manager>, watch::Receiver<bool>) {
        let (cancel, rx) = watch::channel(false);
        let progress = Progress {
            id: "test-install".into(),
            state: "downloading".into(),
            stage: "download".into(),
            downloaded_bytes: 0,
            total_bytes: 100,
            version: "test".into(),
            can_cancel: true,
            error: None,
        };
        (
            Arc::new(Manager {
                root,
                state: Mutex::new(State {
                    plan: None,
                    job: Some(Job { progress, cancel }),
                }),
            }),
            rx,
        )
    }
    #[tokio::test]
    async fn installation_requires_confirmation_and_cannot_commit_after_cancel() {
        let root = tempfile::tempdir().unwrap();
        let (manager, _) = running(root.path().into());
        manager.state.lock().unwrap().job = None;
        assert!(
            manager.clone().start("unconfirmed").await.is_err()
                || platform::detect(root.path()).await.unwrap().is_some()
        );
        let (manager, _) = running(root.path().into());
        manager
            .state
            .lock()
            .unwrap()
            .job
            .as_ref()
            .unwrap()
            .cancel
            .send_replace(true);
        assert_eq!(
            manager.commit("test-install").unwrap_err(),
            installer::CANCELLED
        );
        assert!(manager.commit("stale-job").is_err());
    }
    #[test]
    fn persisted_interrupted_installation_is_recoverable() {
        let root = tempfile::tempdir().unwrap();
        let (manager, _) = running(root.path().into());
        let progress = manager
            .state
            .lock()
            .unwrap()
            .job
            .as_ref()
            .unwrap()
            .progress
            .clone();
        installer::persist(root.path(), &progress).unwrap();
        let restored = Manager::shared(root.path().into());
        let state = restored.state.lock().unwrap();
        let job = state.job.as_ref().unwrap();
        assert_eq!(job.progress.state, "failed");
        assert!(!job.progress.can_cancel);
    }
}
