//! Installation is asynchronous; recall requests must not race the active manifest.
use std::future::Future;
use tokio::sync::watch;

#[derive(Clone)]
pub(super) struct Preparation(watch::Receiver<Option<Result<(), String>>>);
impl Preparation {
    pub fn pending(&self) -> bool {
        self.0.borrow().is_none() && self.0.has_changed().is_ok()
    }
    pub fn start(
        executor: &tokio::runtime::Handle,
        install: impl Future<Output = Result<(), String>> + Send + 'static,
    ) -> Self {
        let (ready, receiver) = watch::channel(None);
        executor.spawn(async move {
            let _ = ready.send(Some(install.await));
        });
        Self(receiver)
    }
    pub async fn wait(mut self) -> Result<(), String> {
        loop {
            if let Some(result) = self.0.borrow().clone() {
                return result.map_err(|e| format!("召回组件准备失败：{e}"));
            }
            self.0
                .changed()
                .await
                .map_err(|_| "召回组件准备已中断".to_owned())?;
        }
    }
}
