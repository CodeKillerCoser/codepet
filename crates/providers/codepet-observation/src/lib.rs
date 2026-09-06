//! Shared local delivery and managed-file mechanics. Harness definitions live in each Provider.
use axum::{extract::{DefaultBodyLimit, State}, http::{HeaderMap, StatusCode}, routing::post, Json, Router};
use codepet_provider_sdk::{EventSubscribeResponse, EventUnsubscribeResponse, ProtocolError,
    ProtocolEvent, ProviderEventSink, ProviderNotificationEvent};
use fs2::FileExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeSet, fs, path::{Path, PathBuf}, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, time::{SystemTime, UNIX_EPOCH}};
use tokio::{sync::Mutex as AsyncMutex, task::JoinHandle};
use uuid::Uuid;

pub struct Definition {
    pub name: &'static str,
    pub config: PathBuf,
    pub events: &'static [&'static str],
    /// Native harness plugin, supplied by its Provider.
    pub plugin: Option<&'static str>,
}

pub fn home() -> PathBuf { dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")) }
pub fn config_home(variable: &str, fallback: PathBuf) -> PathBuf {
    std::env::var_os(variable).map(PathBuf::from).unwrap_or(fallback)
}
fn error(message: impl ToString) -> ProtocolError {
    ProtocolError { code: "observation_unavailable".into(), message: message.to_string(), retryable: true, details: None }
}

struct Running {
    subscriptions: Arc<Mutex<BTreeSet<String>>>,
    server: JoinHandle<()>,
    _lock: fs::File,
    endpoint: PathBuf,
}
impl Drop for Running { fn drop(&mut self) { self.server.abort(); let _ = fs::remove_file(&self.endpoint); } }

pub struct Observation {
    definition: Definition,
    sink: Arc<dyn ProviderEventSink>,
    running: AsyncMutex<Option<Running>>,
}
impl Observation {
    pub fn new(definition: Definition, sink: Arc<dyn ProviderEventSink>) -> Self {
        Self { definition, sink, running: AsyncMutex::new(None) }
    }
    pub async fn subscribe(&self, id: String) -> Result<EventSubscribeResponse, ProtocolError> {
        if id.is_empty() || id.len() > 128 { return Err(error("Invalid subscription id")); }
        let mut running = self.running.lock().await;
        if running.is_none() {
            let config = fs::canonicalize(&self.definition.config).unwrap_or_else(|_| self.definition.config.clone());
            let parent = self.definition.config.parent().ok_or_else(|| error("Missing configuration directory"))?;
            fs::create_dir_all(parent).map_err(error)?;
            // One owner per actual harness config, including other CodePet processes.
            let lock = fs::OpenOptions::new().create(true).truncate(false).read(true).write(true)
                .open(config.with_extension("codepet-observation.lock")).map_err(error)?;
            lock.try_lock_exclusive().map_err(|_| error("Another Provider owns this activity source"))?;
            let directory = parent.join("codepet-observation");
            fs::create_dir_all(&directory).map_err(error)?;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(error)?;
            let token = Uuid::new_v4().to_string();
            let endpoint = json!({"url": format!("http://{}/event", listener.local_addr().map_err(error)?), "token":token});
            let endpoint_path = directory.join("endpoint.json");
            atomic_write(&endpoint_path, &serde_json::to_vec(&endpoint).map_err(error)?)?;
            let script_path = directory.join("forward.mjs");
            atomic_write(&script_path, include_bytes!("forward.mjs"))?;
            if let Some(plugin) = self.definition.plugin {
                let plugins = parent.join("plugins");
                fs::create_dir_all(&plugins).map_err(error)?;
                atomic_write(&plugins.join("codepet-observation.ts"), plugin.as_bytes())?;
            } else {
                install_hooks(&self.definition.config, &script_path, &endpoint_path, self.definition.events)?;
            }
            let subscriptions = Arc::new(Mutex::new(BTreeSet::from([id.clone()])));
            let state = Intake { token, sink: self.sink.clone(), subscriptions: subscriptions.clone(),
                events: self.definition.events, gap: Arc::new(AtomicBool::new(false)) };
            let app = Router::new().route("/event", post(receive)).layer(DefaultBodyLimit::max(256 * 1024)).with_state(state);
            let server = tokio::spawn(async move { let _ = axum::serve(listener, app).await; });
            *running = Some(Running { subscriptions, server, _lock: lock, endpoint: endpoint_path });
        } else {
            let ids = &running.as_ref().unwrap().subscriptions;
            let mut ids = ids.lock().unwrap();
            if ids.len() >= 8 && !ids.contains(&id) { return Err(error("Activity subscriber limit reached")); }
            ids.insert(id.clone());
        }
        Ok(EventSubscribeResponse { subscription_id: id, message: format!("{} 接入已安装；请在原应用信任配置并开始新任务，收到事件后才可确认生效", self.definition.name) })
    }
    pub async fn unsubscribe(&self, id: String) -> Result<EventUnsubscribeResponse, ProtocolError> {
        let mut running = self.running.lock().await;
        if let Some(active) = running.as_ref() {
            let empty = { let mut ids = active.subscriptions.lock().unwrap(); ids.remove(&id); ids.is_empty() };
            if empty { *running = None; }
        }
        Ok(EventUnsubscribeResponse { subscription_id: id })
    }
    pub async fn shutdown(&self) { self.running.lock().await.take(); }
}

#[derive(Clone)]
struct Intake { token: String, sink: Arc<dyn ProviderEventSink>, subscriptions: Arc<Mutex<BTreeSet<String>>>, events: &'static [&'static str], gap: Arc<AtomicBool> }
#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
struct Incoming { event_id: String, payload: serde_json::Map<String, Value> }
async fn receive(State(state): State<Intake>, headers: HeaderMap, Json(input): Json<Incoming>) -> StatusCode {
    if headers.get("authorization").and_then(|h| h.to_str().ok()) != Some(state.token.as_str()) { return StatusCode::UNAUTHORIZED; }
    let name = input.payload.get("hook_event_name").or_else(|| input.payload.get("type")).and_then(Value::as_str).unwrap_or("");
    if !state.events.contains(&name) || input.event_id.is_empty() || input.event_id.len() > 128 { return StatusCode::BAD_REQUEST; }
    let received_at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let mut payload = input.payload;
    if state.gap.swap(false, Ordering::SeqCst) { payload.insert("codepet_gap".into(), json!(true)); }
    for subscription_id in state.subscriptions.lock().unwrap().iter() {
        let params = ProviderNotificationEvent { subscription_id: subscription_id.clone(), event_id: input.event_id.clone(),
            received_at, payload: payload.clone().into_iter().collect() };
        if state.sink.publish(ProtocolEvent::EventNotification { jsonrpc: "2.0".into(), params }).is_err() { state.gap.store(true, Ordering::SeqCst); return StatusCode::SERVICE_UNAVAILABLE; }
    }
    StatusCode::NO_CONTENT
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ProtocolError> {
    let temp = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    let mut file = options.open(&temp).map_err(error)?;
    if let Err(e) = file.write_all(bytes) { let _ = fs::remove_file(&temp); return Err(error(e)); }
    drop(file);
    if let Err(e) = fs::rename(&temp, path) { let _ = fs::remove_file(&temp); return Err(error(e)); }
    Ok(())
}
fn read_config(path: &Path) -> Result<Value, ProtocolError> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(error),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(error(e)),
    }
}
fn node_executable() -> Result<PathBuf, ProtocolError> {
    let filename = if cfg!(windows) { "node.exe" } else { "node" };
    let mut directories: Vec<PathBuf> = std::env::var_os("PATH").map(|value| std::env::split_paths(&value).collect()).unwrap_or_default();
    if cfg!(unix) { directories.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from)); }
    directories.into_iter().map(|directory| directory.join(filename)).find(|path| path.is_absolute() && path.is_file())
        .ok_or_else(|| error("需要 Node.js 才能投递 Hook 活动，请安装后重新启用来源"))
}
fn quote(path: &Path) -> String {
    let value = path.to_string_lossy();
    #[cfg(not(windows))] { format!("'{}'", value.replace('\'', "'\\''")) }
    #[cfg(windows)] { format!("\"{}\"", value.replace('%', "%%").replace('"', "\\\"")) }
}
pub fn install_hooks(config: &Path, script: &Path, endpoint: &Path, events: &[&str]) -> Result<(), ProtocolError> {
    let target = fs::canonicalize(config).unwrap_or_else(|_| config.to_owned());
    let node = node_executable()?;
    let mut root = read_config(&target)?;
    let root_obj = root.as_object_mut().ok_or_else(|| error("Hook config must be an object"))?;
    let hooks = root_obj.entry("hooks").or_insert(json!({})).as_object_mut().ok_or_else(|| error("hooks must be an object"))?;
    for groups in hooks.values_mut() {
        if let Some(groups) = groups.as_array_mut() {
            for group in groups.iter_mut() {
                if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                    handlers.retain(|h| !h.get("command").and_then(Value::as_str).is_some_and(|c|
                        c.contains("code-pet-hook.mjs") || c.contains(&script.to_string_lossy().to_string())));
                }
            }
            groups.retain(|g| !g.get("hooks").and_then(Value::as_array).is_some_and(Vec::is_empty));
        }
    }
    for event in events {
        let groups = hooks.entry(*event).or_insert(json!([])).as_array_mut().ok_or_else(|| error("Hook groups must be arrays"))?;
        groups.push(json!({"hooks":[{"type":"command","command":format!("{} {} {}", quote(&node), quote(script), quote(endpoint)),"timeout":3}]}));
    }
    atomic_write(&target, &serde_json::to_vec_pretty(&root).map_err(error)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_install_is_idempotent_and_preserves_user_handlers() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("hooks.json");
        let script = temp.path().join("forward.mjs");
        let endpoint = temp.path().join("endpoint.json");
        fs::write(&config, r#"{"unrelated":true,"hooks":{"Stop":[{"matcher":"*","hooks":[{"command":"my-script"},{"command":"node /old/code-pet-hook.mjs"}]}]}}"#).unwrap();
        install_hooks(&config, &script, &endpoint, &["UserPromptSubmit", "Stop"]).unwrap();
        let first = fs::read(&config).unwrap();
        install_hooks(&config, &script, &endpoint, &["UserPromptSubmit", "Stop"]).unwrap();
        assert_eq!(first, fs::read(&config).unwrap());
        let root: Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(root["unrelated"], true);
        assert_eq!(root["hooks"]["Stop"][0]["hooks"], json!([{"command":"my-script"}]));
        assert_eq!(root["hooks"]["Stop"].as_array().unwrap().len(), 2);
    }
    #[cfg(unix)]
    #[test]
    fn hook_install_follows_config_symlinks_and_leaves_invalid_config_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("real.json");
        let alias = temp.path().join("hooks.json");
        fs::write(&target, b"{}").unwrap();
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        install_hooks(&alias, &temp.path().join("forward.mjs"), &temp.path().join("endpoint.json"), &["Stop"]).unwrap();
        assert!(fs::symlink_metadata(&alias).unwrap().file_type().is_symlink());
        assert!(read_config(&target).unwrap()["hooks"]["Stop"].is_array());
        fs::write(&target, b"{invalid").unwrap();
        assert!(install_hooks(&alias, &temp.path().join("forward.mjs"), &temp.path().join("endpoint.json"), &["Stop"]).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"{invalid");
    }
    #[tokio::test]
    async fn source_subscription_delivers_only_to_active_subscribers_and_releases_owner() {
        let temp = tempfile::tempdir().unwrap();
        let (sender, mut events) = tokio::sync::mpsc::unbounded_channel();
        let sink: Arc<dyn ProviderEventSink> = Arc::new(move |e| { sender.send(e).unwrap(); Ok(()) });
        let make = || Observation::new(Definition { name:"test", config:temp.path().join("hooks.json"), events:&["Stop"], plugin:None }, sink.clone());
        let observer = make();
        observer.subscribe("one".into()).await.unwrap();
        observer.subscribe("two".into()).await.unwrap();
        observer.unsubscribe("one".into()).await.unwrap();
        let other = make();
        assert!(other.subscribe("other".into()).await.is_err());
        let endpoint: Value = serde_json::from_slice(&fs::read(temp.path().join("codepet-observation/endpoint.json")).unwrap()).unwrap();
        let intake = Intake { token: endpoint["token"].as_str().unwrap().into(), sink: sink.clone(), subscriptions:observer.running.lock().await.as_ref().unwrap().subscriptions.clone(), events:&["Stop"], gap:Arc::new(AtomicBool::new(false)) };
        let mut headers = HeaderMap::new();
        headers.insert("authorization", endpoint["token"].as_str().unwrap().parse().unwrap());
        let status = receive(State(intake), headers, Json(Incoming { event_id:"event-one".into(), payload:json!({"hook_event_name":"Stop"}).as_object().unwrap().clone() })).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let ProtocolEvent::EventNotification { params, .. } = events.recv().await.unwrap() else { panic!("wrong event") };
        assert_eq!(params.subscription_id,"two");
        assert!(events.try_recv().is_err());
        observer.unsubscribe("two".into()).await.unwrap();
        other.subscribe("other".into()).await.unwrap();
        other.shutdown().await;
    }
    #[tokio::test]
    async fn installed_bridge_receives_multiple_processes_and_fails_open_when_offline() {
        let temp = tempfile::tempdir().unwrap();
        let (sender, mut events) = tokio::sync::mpsc::unbounded_channel();
        let sink: Arc<dyn ProviderEventSink> = Arc::new(move |e| { sender.send(e).unwrap(); Ok(()) });
        let observer = Observation::new(Definition { name:"test", config:temp.path().join("hooks.json"), events:&["UserPromptSubmit"], plugin:None }, sink);
        observer.subscribe("host".into()).await.unwrap();
        let script = temp.path().join("codepet-observation/forward.mjs");
        let endpoint = temp.path().join("codepet-observation/endpoint.json");
        let run = |session: &str, event: &str| {
            let script = script.clone(); let endpoint = endpoint.clone();
            let input = json!({"session_id":session,"hook_event_name":event}).to_string();
            tokio::task::spawn_blocking(move || {
                use std::io::Write;
                let mut child = std::process::Command::new("node").arg(script).arg(endpoint)
                    .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
                child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
                let result = child.wait_with_output().unwrap();
                assert!(result.status.success()); assert!(result.stdout.is_empty()); assert!(result.stderr.is_empty());
            })
        };
        let (one, two) = tokio::join!(run("one", "UserPromptSubmit"), run("two", "UserPromptSubmit"));
        one.unwrap(); two.unwrap();
        let mut sessions = BTreeSet::new();
        for _ in 0..2 {
            let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv()).await.unwrap().unwrap();
            let ProtocolEvent::EventNotification { params, .. } = event else { panic!("wrong event") };
            sessions.insert(params.payload["session_id"].as_str().unwrap().to_string());
        }
        assert_eq!(sessions, BTreeSet::from(["one".into(), "two".into()]));
        run("noise", "Notification").await.unwrap();
        assert!(events.try_recv().is_err());
        observer.shutdown().await;
        tokio::time::timeout(std::time::Duration::from_secs(3), run("offline", "UserPromptSubmit")).await.unwrap().unwrap();
    }

}
