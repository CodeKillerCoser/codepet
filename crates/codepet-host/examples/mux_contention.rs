//! Measure default mux over throttled stdio; historical legacy comparison results are retained in reports/.
//! cargo run --release --manifest-path crates/Cargo.toml -p codepet-host --example mux_contention -- reports/stdio-mux.json
use codepet_host::{PluginDescriptor, PluginProcess, PluginProcessOptions, StderrDiagnostic};
use codepet_provider_sdk::*;
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::{Read, Write}, sync::{Arc, OnceLock}, time::{Duration, Instant}};
const MIB: usize = 1024 * 1024;
static START: OnceLock<tokio::sync::Notify> = OnceLock::new();
fn diagnostic(line: &StderrDiagnostic) {
    if line.line == "MUX_BENCH_BODY_STARTED" { START.get().unwrap().notify_one(); }
}
fn payload(size: usize) -> String {
    let mut state = 0x123456789abcdefu64;
    let alphabet = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!#$%&()*+,-./:;<=>?@[]^_`{|}~ ";
    (0..size).map(|_| { state ^= state << 13; state ^= state >> 7; state ^= state << 17; alphabet[state as usize % alphabet.len()] as char }).collect()
}
struct Pace { rate: usize, count: usize, marked: bool }
impl Pace {
    fn pause(&mut self, n: usize) {
        self.count += n;
        if self.rate > 0 { std::thread::sleep(Duration::from_secs_f64(n as f64 / self.rate as f64)); }
        if self.count > MIB && !self.marked { self.marked = true; eprintln!("MUX_BENCH_BODY_STARTED"); }
    }
}
struct Input { pace: Pace }
impl Read for Input {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let cap = bytes.len().min(4096);
        let n = std::io::stdin().read(&mut bytes[..cap])?;
        self.pace.pause(n); Ok(n)
    }
}
struct Output { pace: Pace }
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let cap = bytes.len().min(4096);
        let n = std::io::stdout().write(&bytes[..cap])?;
        self.pace.pause(n); Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> { std::io::stdout().flush() }
}
fn descriptor(text: String) -> ProviderPluginDescriptor {
    ProviderPluginDescriptor { plugin_id: "dev.codepet.mux-bench".into(), display_name: text, version: "test".into(), default_workspace_root: None,
        supported_versions: VersionRange { min_version: 1, max_version: 1 }, instance_kinds: vec![] }
}
struct Fixture { text: String, events: Arc<dyn ProviderEventSink> }
impl ProtocolServer for Fixture {
    fn provider_initialize<'a>(&'a self, _: ProviderInitializeRequest) -> ProtocolFuture<'a, ProviderInitializeResponse> {
        Box::pin(async { Ok(ProviderInitializeResponse { selected_version: 1, plugin: descriptor("ready".into()) }) })
    }
    fn provider_describe<'a>(&'a self, _: ProviderDescribeRequest) -> ProtocolFuture<'a, ProviderDescribeResponse> {
        Box::pin(async { Ok(ProviderDescribeResponse { plugin: descriptor(self.text.clone()) }) })
    }
    fn project_list<'a>(&'a self, request: ProjectListRequest) -> ProtocolFuture<'a, ProjectListResponse> {
        Box::pin(async move {
            for i in 0..4 {
                self.events.publish(ProtocolEvent::EventProjectChanged { jsonrpc: "2.0".into(), params: ProjectChangedEvent {
                    project: ProviderResourceId { device_id: request.route.device_id.clone(), provider_plugin_id: request.route.provider_plugin_id.clone(), provider_instance_id: request.route.provider_instance_id.clone(), native_resource_id: i.to_string() }, change_type: ProjectChangeType::Updated,
                } })?;
            }
            Ok(ProjectListResponse { projects: vec![], page_info: PageInfo { next_cursor: None } })
        })
    }
    fn provider_shutdown<'a>(&'a self, _: ProviderShutdownRequest) -> ProtocolFuture<'a, ProviderShutdownResponse> {
        Box::pin(async { Ok(ProviderShutdownResponse { accepted: true }) })
    }
}
fn init(text: String) -> ProviderInitializeRequest {
    ProviderInitializeRequest { host_client_id: "bench".into(), host_device_id: "bench".into(), host_version: text,
        supported_versions: VersionRange { min_version: 1, max_version: 1 } }
}
async fn provider() {
    let direction = std::env::var("BENCH_DIRECTION").unwrap();
    let rate: usize = std::env::var("BENCH_RATE").unwrap().parse().unwrap();
    let size: usize = std::env::var("BENCH_SIZE").unwrap().parse().unwrap();
    let text = if direction == "response" { payload(size) } else { "small".into() };
    serve_stdio_with_io(Input { pace: Pace { rate: if direction == "request" { rate } else { 0 }, count: 0, marked: false } },
        Output { pace: Pace { rate: if direction == "response" { rate } else { 0 }, count: 0, marked: false } },
        StdioServerOptions::default(), |events| Fixture { text, events }).await.unwrap();
}
async fn trial(profile: &str, direction: &str, size: usize, rate: usize) -> Value {
    let process = Arc::new(PluginProcess::spawn(&PluginDescriptor { plugin_id: "dev.codepet.mux-bench".into(), display_name: "benchmark".into(), icon: None,
        executable: std::env::current_exe().unwrap(), args: vec!["--provider".into()], env: BTreeMap::from([
            (TRANSPORT_ENV.into(), profile.into()), ("BENCH_DIRECTION".into(), direction.into()), ("BENCH_RATE".into(), rate.to_string()), ("BENCH_SIZE".into(), size.to_string())]),
        enabled: true, instances: vec![] }, PluginProcessOptions { stderr_observer: Some(diagnostic), ..PluginProcessOptions::default() }).unwrap());
    process.client().provider_initialize(init("bench".into())).await.unwrap();
    let mut events = process.take_inbound().await.unwrap();
    let p = process.clone(); let request = direction == "request";
    let input = if request { payload(size) } else { String::new() };
    let large = tokio::spawn(async move {
        let began = Instant::now();
        let result = if request { p.client().provider_initialize(init(input)).await.map(|_| ()) }
            else { p.client().provider_describe(ProviderDescribeRequest {}).await.map(|_| ()) };
        json!({"elapsedMs":began.elapsed().as_secs_f64()*1000.,"error":result.err().map(|e| e.code)})
    });
    tokio::time::timeout(Duration::from_secs(5), START.get().unwrap().notified()).await.unwrap();
    let began = Instant::now();
    let p = process.clone();
    let ping = tokio::spawn(async move {
        let began = Instant::now();
        let result = p.client().provider_ping(ProviderPingRequest { sequence: 1, host_session_id: "bench".into(), clients: ClientConnectionsSnapshot { revision: 1, connections: vec![] }, instances: vec![] }).await;
        json!({"elapsedMs":began.elapsed().as_secs_f64()*1000., "error":result.err().map(|e|e.code)})
    });
    let small = process.client().project_list(ProjectListRequest { route: ProviderInstanceRoute { device_id: "bench".into(), provider_plugin_id: "dev.codepet.mux-bench".into(), provider_instance_id: "bench".into() }, cursor: None, limit: None }).await;
    let small_ms = began.elapsed().as_secs_f64()*1000.;
    let large_pending_when_small_done = !large.is_finished();
    let mut event_ids = vec![];
    for _ in 0..4 {
        if let Ok(Some(ProviderWireMessage::Event(ProtocolEvent::EventProjectChanged { params, .. }))) = tokio::time::timeout(Duration::from_secs(5), events.recv()).await { event_ids.push(params.project.native_resource_id); }
    }
    let ping = ping.await.unwrap(); let large = large.await.unwrap();
    let exit = process.shutdown().await;
    json!({"profile":profile,"direction":direction,"payloadBytes":size,"rateBytesPerSecond":rate,
        "smallMs":small_ms,"smallError":small.err().map(|e|e.code), "largePendingWhenSmallDone":large_pending_when_small_done,
        "ping":ping,"large":large,"orderedEvents":event_ids == ["0","1","2","3"],"cleanShutdown":exit.as_ref().is_ok_and(|e|e.success), "shutdownError":exit.err().map(|e| json!({"code":e.code,"message":e.message})), "stderrTail":process.stderr_diagnostics().into_iter().rev().take(8).map(|v| v.line).collect::<Vec<_>>()})
}
fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build().unwrap();
    runtime.block_on(async {
        let args: Vec<String> = std::env::args().collect();
        if args.get(1).map(String::as_str) == Some("--provider") { provider().await; return; }
        START.set(tokio::sync::Notify::new()).unwrap();
        let mut rows = vec![];
        if args.get(1).map(String::as_str) == Some("--shutdown-case") {
            let repeats = args.get(2).and_then(|v|v.parse::<usize>().ok()).unwrap_or(3);
            for _ in 0..repeats { eprintln!("{}", trial(MUX_PROFILE,"response",16*MIB,MIB).await); }
            return;
        }
        let repeats: usize = args.get(2).and_then(|v|v.parse().ok()).unwrap_or(1);
        for index in 0..repeats { for size in [8*MIB,16*MIB] { for direction in ["request","response"] { for profile in [MUX_PROFILE] {
            let mut row = trial(profile,direction,size,4*MIB).await; row["trial"] = json!(index + 1); eprintln!("{row}"); rows.push(row);
        } } } }
        for direction in ["request","response"] { for profile in [MUX_PROFILE] {
            let mut row = trial(profile,direction,16*MIB,MIB).await; row["trial"] = json!(1); eprintln!("{row}"); rows.push(row);
        } }
        std::fs::write(args.get(1).map(String::as_str).unwrap_or("reports/stdio-mux-current.json"), serde_json::to_vec_pretty(&json!({"rows":rows})).unwrap()).unwrap();
    });
}
