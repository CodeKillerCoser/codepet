use super::{HttpServer, HttpShutdown};
use codepet_task_lineage::tasks::Ledger;
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    sync::{watch, Mutex},
    task::JoinHandle,
};

#[derive(Clone)]
enum EndpointStatus {
    Starting,
    Ready(Value),
    Failed(String),
    Stopped,
}
struct Inner {
    ledger: Ledger,
    recall: Arc<crate::recall::Recall>,
    recall_watch: JoinHandle<()>,
    status: watch::Receiver<EndpointStatus>,
    shutdown: HttpShutdown,
    completed: Arc<AtomicBool>,
    task: Mutex<Option<JoinHandle<()>>>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.recall_watch.abort();
    }
}

/// Owned by the Rust application, independent of any frontend window.
#[derive(Clone)]
pub struct HttpRuntime {
    inner: Arc<Inner>,
}
impl HttpRuntime {
    /// Call within the application's Tokio runtime. Binding failures do not disable the ledger.
    pub fn start(data: PathBuf, port: u16) -> Self {
        Self::start_on(&tokio::runtime::Handle::current(), data, port)
    }
    pub fn start_on(executor: &tokio::runtime::Handle, data: PathBuf, port: u16) -> Self {
        Self::start_on_prepared(executor, data, port, async { Ok(()) })
    }
    /// MCP/ledger stay available during installation; recall alone waits for it.
    pub fn start_on_prepared(
        executor: &tokio::runtime::Handle,
        data: PathBuf,
        port: u16,
        install: impl std::future::Future<Output = Result<(), String>> + Send + 'static,
    ) -> Self {
        let ledger = Ledger::new(&data);
        let recall = crate::recall::Recall::shared(&ledger);
        recall.prepare(executor, install);
        let recall_watch = recall.watch(executor);
        let shutdown = HttpShutdown::default();
        let completed = Arc::new(AtomicBool::new(false));
        let (tx, status) = watch::channel(EndpointStatus::Starting);
        let (task_ledger, stop, finished) = (ledger.clone(), shutdown.clone(), completed.clone());
        let task = executor.spawn(async move {
            match HttpServer::bind(task_ledger, &data, port, stop).await {
                Ok(server) => {
                    let _ = tx.send(EndpointStatus::Ready(server.configuration()));
                    let result = server.serve().await;
                    let _ = tx.send(match result {
                        Ok(()) => EndpointStatus::Stopped,
                        Err(error) => {
                            EndpointStatus::Failed(format!("任务 MCP HTTP 服务中断：{error}"))
                        }
                    });
                }
                Err(error) => {
                    let _ = tx.send(EndpointStatus::Failed(format!(
                        "本机任务 MCP HTTP 服务启动失败（端口 {port}）：{error}"
                    )));
                }
            }
            finished.store(true, Ordering::Release);
        });
        Self {
            inner: Arc::new(Inner {
                ledger,
                recall,
                recall_watch,
                status,
                shutdown,
                completed,
                task: Mutex::new(Some(task)),
            }),
        }
    }
    /// Human IPC and MCP share this exact active data directory until application restart.
    pub fn ledger(&self) -> Ledger {
        self.inner.ledger.clone()
    }
    pub fn recall(&self) -> Arc<crate::recall::Recall> {
        self.inner.recall.clone()
    }
    pub fn shutdown_completed(&self) -> bool {
        self.inner.completed.load(Ordering::Acquire)
    }
    pub async fn configuration(&self) -> Result<Value, String> {
        let mut status = self.inner.status.clone();
        tokio::time::timeout(Duration::from_secs(5), async move {
            loop {
                let value = status.borrow().clone();
                match value {
                    EndpointStatus::Starting => {}
                    EndpointStatus::Ready(mut config) => {
                        if status.has_changed().is_err() {
                            return Err("本机任务 MCP HTTP 服务已退出".into());
                        }
                        let settings = self.instructions().await?;
                        config
                            .as_object_mut()
                            .unwrap()
                            .extend(settings.as_object().unwrap().clone());
                        return Ok(config);
                    }
                    EndpointStatus::Failed(error) => return Err(error),
                    EndpointStatus::Stopped => return Err("本机任务 MCP HTTP 服务已停止".into()),
                }
                status
                    .changed()
                    .await
                    .map_err(|_| "本机任务 MCP HTTP 服务已退出")?;
            }
        })
        .await
        .map_err(|_| "本机任务 MCP HTTP 服务启动超时，请重试".to_string())?
    }
    pub async fn instructions(&self) -> Result<Value, String> {
        let ledger = self.ledger();
        tokio::task::spawn_blocking(move || crate::instructions::read(&ledger))
            .await
            .map_err(|e| e.to_string())?
    }
    pub async fn save_instructions(
        &self,
        expected_revision: u64,
        text: Option<String>,
    ) -> Result<Value, String> {
        let ledger = self.ledger();
        tokio::task::spawn_blocking(move || {
            crate::instructions::save(&ledger, expected_revision, text.as_deref())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    pub async fn shutdown_once(&self) {
        self.inner.recall_watch.abort();
        self.inner.recall.stop().await;
        self.inner.shutdown.cancel();
        let mut guard = self.inner.task.lock().await;
        if let Some(mut task) = guard.take() {
            if tokio::time::timeout(Duration::from_secs(3), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
        self.inner.completed.store(true, Ordering::Release);
    }
}
