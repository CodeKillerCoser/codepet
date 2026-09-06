//! Read-only Pet application boundary, subscribed independently of Remote Gateway.
mod projection;
use crate::{PluginManager, PluginRuntimeState, ProviderEventSubscription};
use codepet_pet_sdk::{PetSnapshot, PetSource, PetSourceSetEnabledRequest, PetSourceSetEnabledResponse};
use std::{collections::BTreeMap, path::PathBuf, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, time::Duration};
use projection::Projection;
pub use codepet_pet_sdk as protocol;

pub struct PetGateway {
    manager: Arc<PluginManager>,
    state: Mutex<Projection>,
    preferences: PathBuf,
    started: AtomicBool,
}
const SOURCES: &[(&str, &str)] = &[("codex", "Codex"), ("claude", "Claude Code"), ("opencode", "OpenCode")];
impl PetGateway {
    pub fn new(manager: Arc<PluginManager>, preferences: PathBuf) -> Arc<Self> {
        let enabled: BTreeMap<String, bool> = std::fs::read(&preferences).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let sources = SOURCES.iter().map(|(id, name)| (id.to_string(), PetSource { id: id.to_string(), display_name: name.to_string(),
            enabled: enabled.get(*id).copied().unwrap_or(true), status: "connecting".into(), message: String::new() })).collect();
        Arc::new(Self { manager, state: Mutex::new(Projection::new(sources)), preferences, started: AtomicBool::new(false) })
    }
    pub fn start(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::SeqCst) { return; }
        for (id, _) in SOURCES {
            let weak = Arc::downgrade(self); let manager = Arc::downgrade(&self.manager); let id = id.to_string();
            tokio::spawn(async move {
                let plugin = format!("dev.codepet.{id}");
                let subscription_id = format!("pet-{}", uuid::Uuid::new_v4());
                let mut subscription: Option<ProviderEventSubscription> = None;
                let mut generation = None;
                let mut retry_at = tokio::time::Instant::now();
                loop {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    let Some(gateway) = weak.upgrade() else { break; };
                    if !gateway.started.load(Ordering::SeqCst) { break; }
                    let enabled = gateway.state.lock().unwrap().sources[&id].enabled;
                    let snapshot = gateway.manager.snapshots().await.into_iter().find(|p| p.catalog.plugin_id == plugin);
                    let unavailable = match snapshot.as_ref() {
                        None => Some("未找到此来源的 Provider".to_string()),
                        Some(p) if p.state == PluginRuntimeState::Crashed => Some(p.diagnostic.as_ref().map(ToString::to_string).unwrap_or_else(|| "Provider 已停止，请检查连接设置".into())),
                        _ => None,
                    };
                    let current = snapshot.filter(|p| p.state == PluginRuntimeState::Ready).map(|p| p.generation);
                    if !enabled || current != generation || current.is_none() {
                        if subscription.take().is_some() { let _ = gateway.manager.unsubscribe_events(&plugin, &subscription_id).await; }
                        generation = current;
                        gateway.state.lock().unwrap().source_status(&id, if !enabled { "disabled" } else if unavailable.is_some() { "error" } else { "connecting" }, if enabled { unavailable.as_deref().unwrap_or("") } else { "" });
                    }
                    if !enabled || current.is_none() { continue; }
                    if subscription.is_none() && tokio::time::Instant::now() >= retry_at {
                        match gateway.manager.subscribe_events(&plugin, &subscription_id).await {
                            Ok(s) => {
                                gateway.state.lock().unwrap().source_status(&id, "installed", &s.message);
                                subscription = Some(s);
                            }
                            Err(e) => {
                                gateway.state.lock().unwrap().source_status(&id, "error", &e.to_string());
                                retry_at = tokio::time::Instant::now() + Duration::from_secs(15);
                            }
                        }
                    }
                    let mut disconnected = false;
                    if let Some(s) = subscription.as_mut() {
                        for _ in 0..256 {
                            match s.receiver.try_recv() {
                                Ok(event) => gateway.state.lock().unwrap().apply(&id, event),
                                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => { disconnected = true; break; }
                            }
                        }
                    }
                    if disconnected {
                        subscription = None;
                        gateway.state.lock().unwrap().source_status(&id, "error", "活动订阅中断，正在重新连接；部分事件可能丢失");
                    }
                }
                if subscription.is_some() {
                    if let Some(manager) = manager.upgrade() { let _ = manager.unsubscribe_events(&plugin, &subscription_id).await; }
                }
            });
        }
    }
    pub fn stop(&self) { self.started.store(false, Ordering::SeqCst); }
    pub fn snapshot(&self) -> PetSnapshot { self.state.lock().unwrap().snapshot() }
    pub fn set_enabled(&self, request: PetSourceSetEnabledRequest) -> Result<PetSourceSetEnabledResponse, String> {
        let mut state = self.state.lock().unwrap();
        if !state.sources.contains_key(&request.source_id) { return Err("Unknown activity source".into()); }
        let enabled: BTreeMap<_, _> = state.sources.iter().map(|(id, s)| (id.clone(), if id == &request.source_id { request.enabled } else { s.enabled })).collect();
        if let Some(parent) = self.preferences.parent() { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
        let temp = self.preferences.with_extension("tmp");
        std::fs::write(&temp, serde_json::to_vec(&enabled).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        std::fs::rename(temp, &self.preferences).map_err(|e| e.to_string())?;
        state.sources.get_mut(&request.source_id).unwrap().enabled = request.enabled;
        state.source_status(&request.source_id, if request.enabled { "connecting" } else { "disabled" }, "");
        Ok(PetSourceSetEnabledResponse { source: state.sources[&request.source_id].clone() })
    }
}

impl protocol::ProtocolServer for PetGateway {
    fn protocol_initialize<'a>(&'a self, request: protocol::PetInitializeRequest) -> protocol::ProtocolFuture<'a, protocol::PetInitializeResponse> {
        Box::pin(async move {
            if request.supported_versions.min_version > protocol::PROTOCOL_VERSION || request.supported_versions.max_version < protocol::PROTOCOL_VERSION {
                return Err(protocol::ProtocolError { code: "unsupported_version".into(), message: "Unsupported Pet protocol version".into(), retryable: false, details: None });
            }
            Ok(protocol::PetInitializeResponse { selected_version: protocol::PROTOCOL_VERSION, snapshot: self.snapshot() })
        })
    }
    fn pet_snapshot<'a>(&'a self, _request: protocol::PetSnapshotRequest) -> protocol::ProtocolFuture<'a, protocol::PetSnapshotResponse> {
        Box::pin(async move { Ok(protocol::PetSnapshotResponse { snapshot: self.snapshot() }) })
    }
    fn source_set_enabled<'a>(&'a self, request: PetSourceSetEnabledRequest) -> protocol::ProtocolFuture<'a, PetSourceSetEnabledResponse> {
        Box::pin(async move { self.set_enabled(request).map_err(|message| protocol::ProtocolError { code: "source_update_failed".into(), message, retryable: true, details: None }) })
    }
}
