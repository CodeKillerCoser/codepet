//! Shared local delivery and managed-file mechanics. Harness definitions live in each Provider.
use axum::{extract::{DefaultBodyLimit, State}, http::{HeaderMap, StatusCode}, routing::post, Json, Router};
use codepet_provider_sdk::{NoticeObserveResponse, ProtocolError, ProviderEventSink};
mod notices;
pub use notices::{HookEvent,HookObserver};
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
    /// Harnesses supporting commandWindows may execute hooks through PowerShell or CMD.
    pub windows_command_override: bool,
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
    worker: JoinHandle<()>,
    _lock: fs::File,
    endpoint: PathBuf,
}
impl Drop for Running { fn drop(&mut self) { self.server.abort(); self.worker.abort(); let _ = fs::remove_file(&self.endpoint); } }

pub struct Observation {
    definition: Definition,
    sink: Arc<dyn ProviderEventSink>,
    observer: Option<Arc<dyn HookObserver>>,
    running: AsyncMutex<Option<Running>>,
}
impl Observation {
    pub fn new(definition: Definition, sink: Arc<dyn ProviderEventSink>) -> Self {
        Self { definition, sink, observer:None, running: AsyncMutex::new(None) }
    }
    pub fn with_observer(mut self,observer:Arc<dyn HookObserver>)->Self{self.observer=Some(observer);self}
    pub async fn observe(&self,enabled:bool)->Result<NoticeObserveResponse,ProtocolError>{if enabled{self.subscribe("notice-host".into()).await}else{self.unsubscribe("notice-host".into()).await}}
    pub async fn subscribe(&self, id: String) -> Result<NoticeObserveResponse, ProtocolError> {
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
                install_hooks_with_options(&self.definition.config, &script_path, &endpoint_path, self.definition.events, self.definition.windows_command_override)?;
            }
            let subscriptions = Arc::new(Mutex::new(BTreeSet::from([id.clone()])));
            let (input,mut incoming)=tokio::sync::mpsc::channel::<(HookEvent,bool)>(256);
            let observer=self.observer.clone();let mut publisher=notices::Publisher::new(self.definition.name,self.sink.clone());
            let worker=tokio::spawn(async move {while let Some((event,publish))=incoming.recv().await{if let Some(observer)=observer.as_ref(){let _=observer.observe(&event);}if publish{if let Err(error)=publisher.apply(event).await{eprintln!("Notice publication failed: {}",error.message);}}}});
            let state=Intake{token,input,subscriptions:subscriptions.clone(),events:self.definition.events,gap:Arc::new(AtomicBool::new(false))};
            let app = Router::new().route("/event", post(receive)).layer(DefaultBodyLimit::max(256 * 1024)).with_state(state);
            let server = tokio::spawn(async move { let _ = axum::serve(listener, app).await; });
            *running = Some(Running { subscriptions, server, worker, _lock: lock, endpoint: endpoint_path });
        } else {
            let ids = &running.as_ref().unwrap().subscriptions;
            let mut ids = ids.lock().unwrap();
            if ids.len() >= 8 && !ids.contains(&id) { return Err(error("Activity subscriber limit reached")); }
            ids.insert(id.clone());
        }
        Ok(NoticeObserveResponse { enabled:true, message: format!("{} 接入已安装；请在原应用信任配置并开始新任务，收到事件后才可确认生效", self.definition.name) })
    }
    pub async fn unsubscribe(&self, id: String) -> Result<NoticeObserveResponse, ProtocolError> {
        let mut running = self.running.lock().await;
        if let Some(active) = running.as_ref() {
            let empty = { let mut ids = active.subscriptions.lock().unwrap(); ids.remove(&id); ids.is_empty() };
            if empty { *running = None; }
        }
        Ok(NoticeObserveResponse { enabled:false,message:"观察发布已关闭".into() })
    }
    pub async fn shutdown(&self) { self.running.lock().await.take(); }
}

#[derive(Clone)]
struct Intake { token: String, input: tokio::sync::mpsc::Sender<(HookEvent,bool)>, subscriptions: Arc<Mutex<BTreeSet<String>>>, events: &'static [&'static str], gap: Arc<AtomicBool> }
#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
struct Incoming { event_id: String, payload: serde_json::Map<String, Value> }
async fn receive(State(state): State<Intake>, headers: HeaderMap, Json(input): Json<Incoming>) -> StatusCode {
    if headers.get("authorization").and_then(|h| h.to_str().ok()) != Some(state.token.as_str()) { return StatusCode::UNAUTHORIZED; }
    let name = input.payload.get("hook_event_name").or_else(|| input.payload.get("type")).and_then(Value::as_str).unwrap_or("");
    if !state.events.contains(&name) || input.event_id.is_empty() || input.event_id.len() > 128 { return StatusCode::BAD_REQUEST; }
    let received_at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let payload=input.payload;
    let gap=state.gap.swap(false,Ordering::SeqCst);
    let publish=state.subscriptions.lock().unwrap().contains("notice-host");
    if state.input.try_send((HookEvent{event_id:input.event_id,received_at,gap,payload},publish)).is_err(){state.gap.store(true,Ordering::SeqCst);return StatusCode::SERVICE_UNAVAILABLE;}
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
    install_hooks_with_options(config, script, endpoint, events, false)
}
fn install_hooks_with_options(config: &Path, script: &Path, endpoint: &Path, events: &[&str], windows_override: bool) -> Result<(), ProtocolError> {
    let target = fs::canonicalize(config).unwrap_or_else(|_| config.to_owned());
    let node = node_executable()?;
    let mut root = read_config(&target)?;
    let root_obj = root.as_object_mut().ok_or_else(|| error("Hook config must be an object"))?;
    let hooks = root_obj.entry("hooks").or_insert(json!({})).as_object_mut().ok_or_else(|| error("hooks must be an object"))?;
    for groups in hooks.values_mut() {
        if let Some(groups) = groups.as_array_mut() {
            for group in groups.iter_mut() {
                if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                    handlers.retain(|h| {
                        let owned=|c:&str| c.contains("code-pet-hook.mjs") || c.contains(&script.to_string_lossy().to_string());
                        !h.get("command").and_then(Value::as_str).is_some_and(owned)
                            && !h.get("args").and_then(Value::as_array).is_some_and(|args|args.iter().filter_map(Value::as_str).any(owned))
                    });
                }
            }
            groups.retain(|g| !g.get("hooks").and_then(Value::as_array).is_some_and(Vec::is_empty));
        }
    }
    for event in events {
        let groups = hooks.entry(*event).or_insert(json!([])).as_array_mut().ok_or_else(|| error("Hook groups must be arrays"))?;
        let mut handler = json!({"type":"command","command":format!("{} {} {}", quote(&node), quote(script), quote(endpoint)),"timeout":3});
        if !windows_override {
            handler["command"]=json!(node);
            handler["args"]=json!([script,endpoint]);
        }
        #[cfg(windows)]
        if windows_override {
            use base64::Engine;
            let ps_quote = |path: &Path| format!("'{}'", path.to_string_lossy().replace('\'', "''"));
            let invocation = format!("& {} {} {}", ps_quote(&node), ps_quote(script), ps_quote(endpoint));
            let bytes: Vec<u8> = invocation.encode_utf16().flat_map(u16::to_le_bytes).collect();
            handler["commandWindows"] = json!(format!("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {}", base64::engine::general_purpose::STANDARD.encode(bytes)));
            // Codex caps Interrupt at three seconds; other events also need Windows shell startup time.
            if *event != "Interrupt" { handler["timeout"] = json!(10); }
        }
        #[cfg(not(windows))]
        let _ = (windows_override, &mut handler);
        groups.push(json!({"hooks":[handler]}));
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
        install_hooks_with_options(&config, &script, &endpoint, &["Stop"], true).unwrap();
        let override_config = fs::read(&config).unwrap();
        install_hooks_with_options(&config, &script, &endpoint, &["Stop"], true).unwrap();
        assert_eq!(override_config, fs::read(&config).unwrap());
        #[cfg(windows)]
        assert!(read_config(&config).unwrap()["hooks"]["Stop"][1]["hooks"][0]["commandWindows"].as_str().unwrap().starts_with("powershell.exe "));
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
    async fn observation_preserves_raw_object_when_reporting_a_gap() {
        let (input,mut events)=tokio::sync::mpsc::channel(2);
        let intake=Intake{token:"secret".into(),input,subscriptions:Arc::new(Mutex::new(BTreeSet::from(["notice-host".into()]))),events:&["Stop"],gap:Arc::new(AtomicBool::new(true))};
        let raw=json!({"hook_event_name":"Stop","codepet_gap":"native value","nested":{"text":"原始内容","null":null}});
        let mut headers=HeaderMap::new();headers.insert("authorization","secret".parse().unwrap());
        assert_eq!(receive(State(intake.clone()),headers.clone(),Json(Incoming{event_id:"one".into(),payload:raw.as_object().unwrap().clone()})).await,StatusCode::NO_CONTENT);
        let (event,publish)=events.recv().await.unwrap();assert_eq!(Value::Object(event.payload),raw);assert!(event.gap&&publish);
        assert_eq!(receive(State(intake.clone()),HeaderMap::new(),Json(Incoming{event_id:"two".into(),payload:raw.as_object().unwrap().clone()})).await,StatusCode::UNAUTHORIZED);
        for id in ["one","two"]{assert_eq!(receive(State(intake.clone()),headers.clone(),Json(Incoming{event_id:id.into(),payload:raw.as_object().unwrap().clone()})).await,StatusCode::NO_CONTENT);}
        assert_eq!(receive(State(intake.clone()),headers.clone(),Json(Incoming{event_id:"overflow".into(),payload:raw.as_object().unwrap().clone()})).await,StatusCode::SERVICE_UNAVAILABLE);
        events.recv().await.unwrap();events.recv().await.unwrap();
        assert_eq!(receive(State(intake),headers,Json(Incoming{event_id:"after".into(),payload:raw.as_object().unwrap().clone()})).await,StatusCode::NO_CONTENT);assert!(events.recv().await.unwrap().0.gap);
    }

    #[tokio::test]
    async fn source_subscription_delivers_only_to_active_subscribers_and_releases_owner() {
        let temp = tempfile::tempdir().unwrap();
        let sink:Arc<dyn ProviderEventSink>=Arc::new(|_|Ok(()));
        let make=||Observation::new(Definition{windows_command_override:true,name:"test",config:temp.path().join("hooks.json"),events:&["Stop"],plugin:None},sink.clone());
        let observer=make();observer.subscribe("internal".into()).await.unwrap();observer.observe(true).await.unwrap();
        let other=make();assert!(other.observe(true).await.is_err());
        observer.observe(false).await.unwrap();assert!(observer.running.lock().await.is_some());
        observer.unsubscribe("internal".into()).await.unwrap();assert!(observer.running.lock().await.is_none());
        other.observe(true).await.unwrap();other.shutdown().await;
    }
    #[tokio::test]
    async fn installed_bridge_receives_multiple_processes_and_fails_open_when_offline() {
        check_installed_bridge(true).await;
    }
    #[tokio::test]
    async fn claude_exec_hook_preserves_argv_and_delivers_events() {
        check_installed_bridge(false).await;
    }
    async fn check_installed_bridge(codex: bool) {
        let temp = tempfile::tempdir().unwrap();
        let (sender, mut events) = tokio::sync::mpsc::unbounded_channel();
        let sink: Arc<dyn ProviderEventSink> = Arc::new(|_|Ok(()));
        let hook_observer:Arc<dyn HookObserver>=Arc::new(move |e:&HookEvent|{sender.send(e.payload.clone()).unwrap();Ok(())});
        let observer = Observation::new(Definition { windows_command_override: codex, name:"test", config:temp.path().join("hooks.json"), events:&["UserPromptSubmit"], plugin:None }, sink).with_observer(hook_observer);
        observer.subscribe("host".into()).await.unwrap();
        let script = temp.path().join("codepet-observation").join("forward.mjs");
        let endpoint = temp.path().join("codepet-observation").join("endpoint.json");
        let installed = read_config(&temp.path().join("hooks.json")).unwrap();
        let handler = &installed["hooks"]["UserPromptSubmit"][0]["hooks"][0];
        let hook_command = handler.get(if codex && cfg!(windows) { "commandWindows" } else { "command" }).unwrap().as_str().unwrap().to_owned();
        let hook_executable = handler["command"].as_str().unwrap().to_owned();
        let hook_args: Vec<String> = handler.get("args").and_then(Value::as_array).map(|args| args.iter().map(|arg| arg.as_str().unwrap().to_owned()).collect()).unwrap_or_default();
        let run = |session: &str, event: &str| {
            let script = script.clone(); let endpoint = endpoint.clone();
            let hook_executable = hook_executable.clone(); let hook_args = hook_args.clone();
            let hook_command = hook_command.clone();
            let powershell = session == "two";
            let input = json!({"session_id":session,"hook_event_name":event}).to_string();
            tokio::task::spawn_blocking(move || {
                use std::io::Write;
                #[cfg(windows)]
                let mut command = {
                    use std::os::windows::process::CommandExt;
                    let command = if powershell {
                        let mut command = codepet_provider_sdk::local_runtime::command("powershell.exe");
                        command.args(["-NoProfile", "-NonInteractive", "-Command"]).arg(hook_command);
                        command
                    } else {
                        let mut command = codepet_provider_sdk::local_runtime::command(std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into()));
                        command.arg("/C").raw_arg(format!("\"{hook_command}\""));
                        command
                    };
                    let _ = (script, endpoint);
                    command
                };
                #[cfg(not(windows))]
                let mut command = {
                    let _ = powershell;
                    let mut command = codepet_provider_sdk::local_runtime::command("/bin/sh");
                    command.arg("-c").arg(hook_command);
                    let _ = (script, endpoint);
                    command
                };
                if !codex {
                    command = codepet_provider_sdk::local_runtime::command(hook_executable);
                    command.args(hook_args);
                }
                let mut child = command
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
            sessions.insert(event["session_id"].as_str().unwrap().to_string());
        }
        assert_eq!(sessions, BTreeSet::from(["one".into(), "two".into()]));
        run("noise", "Notification").await.unwrap();
        assert!(events.try_recv().is_err());
        observer.shutdown().await;
        tokio::time::timeout(std::time::Duration::from_secs(10), run("offline", "UserPromptSubmit")).await.unwrap().unwrap();
    }

}
