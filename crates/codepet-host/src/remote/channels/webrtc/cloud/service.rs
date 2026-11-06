use super::*;

/// Hot-swaps only the signaling worker; LAN listener and established sessions survive.
#[derive(Clone)]
pub struct SignalingServiceController(Arc<Control>);

struct Control {
    access: Arc<RemoteAccessManager>,
    gateway: Arc<ProviderGatewayService>,
    sessions: Arc<SessionRegistry>,
    cloud: Mutex<Option<Arc<Cloud>>>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    update: tokio::sync::Mutex<()>,
    stopped: std::sync::atomic::AtomicBool,
}

impl SignalingServiceController {
    pub(crate) fn new(
        access: Arc<RemoteAccessManager>,
        gateway: Arc<ProviderGatewayService>,
        sessions: Arc<SessionRegistry>,
    ) -> Self {
        let cloud = Cloud::load(access.clone()).unwrap_or(None);
        let task = cloud
            .as_ref()
            .map(|c| c.start(gateway.clone(), sessions.clone()));
        Self(Arc::new(Control {
            access,
            gateway,
            sessions,
            cloud: Mutex::new(cloud),
            task: Mutex::new(task),
            update: tokio::sync::Mutex::new(()),
            stopped: std::sync::atomic::AtomicBool::new(false),
        }))
    }

    pub(crate) fn current(&self) -> HostResult<Option<Arc<Cloud>>> {
        Ok(self.0.cloud.lock().map_err(|_| error())?.clone())
    }

    pub async fn configure(&self, address: Option<String>) -> HostResult<()> {
        let _update = self.0.update.lock().await;
        if self.0.stopped.load(std::sync::atomic::Ordering::Acquire) {
            return Err(error());
        }
        *self
            .0
            .access
            .signaling_service
            .lock()
            .map_err(|_| error())? =
            crate::normalize_signaling_service(address.as_deref().unwrap_or(""))?;
        let cloud = Cloud::load(self.0.access.clone())?;
        // Apply before waiting for the network so retries survive a failed save response.
        {
            let mut task = self.0.task.lock().map_err(|_| error())?;
            if self.0.stopped.load(std::sync::atomic::Ordering::Acquire) {
                return Err(error());
            }
            if let Some(old) = task.take() {
                old.abort();
            }
            *self.0.cloud.lock().map_err(|_| error())? = cloud.clone();
            if let Some(c) = &cloud {
                *task = Some(c.start(self.0.gateway.clone(), self.0.sessions.clone()));
            }
        }
        if let Some(cloud) = cloud {
            cloud.ensure_registered().await?;
        }
        Ok(())
    }

    pub(crate) fn stop(&self) {
        self.0
            .stopped
            .store(true, std::sync::atomic::Ordering::Release);
        if let Ok(mut task) = self.0.task.lock() {
            if let Some(task) = task.take() {
                task.abort();
            }
        }
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        if let Ok(task) = self.task.get_mut() {
            if let Some(task) = task.take() {
                task.abort();
            }
        }
    }
}
