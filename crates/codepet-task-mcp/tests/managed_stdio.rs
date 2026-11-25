use codepet_task_lineage::tasks::Ledger;
use codepet_task_mcp::managed;
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

fn install(source: &Path, workspace: &Path, data: &Path) -> Value {
    managed::install(source, workspace, data).unwrap()
}

#[test]
fn upgrades_keep_the_bootstrap_and_install_immutable_versions_before_switching() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("用户目录");
    let data = dir.path().join("data");
    let source = dir.path().join("runner");
    fs::write(&source, b"first standalone version").unwrap();
    let first = install(&source, &workspace, &data);
    let launcher = first["command"].as_str().unwrap();
    assert!(!first["config"].as_str().unwrap().contains("url ="));
    assert!(first["config"].as_str().unwrap().contains("CODEPET_AGENT"));
    assert!(first["config"]
        .as_str()
        .unwrap()
        .contains("--managed-stdio"));
    fs::write(&source, b"second standalone version").unwrap();
    let second = install(&source, &workspace, &data);
    assert_eq!(first, second);
    assert_eq!(fs::read(launcher).unwrap(), b"first standalone version");
    assert_eq!(
        fs::read_dir(workspace.join("tools/task-mcp/versions"))
            .unwrap()
            .count(),
        2
    );
    // Failed preparation must not switch away from the last complete installation.
    let current = workspace.join("tools/task-mcp/current.json");
    let before = fs::read(&current).unwrap();
    assert!(managed::install(&dir.path().join("missing"), &workspace, &data).is_err());
    assert_eq!(fs::read(current).unwrap(), before);
    fs::write(launcher, b"unmanaged modification").unwrap();
    assert!(managed::install(&source, &workspace, &data).is_err());
}

struct Client {
    child: Child,
    messages: Receiver<Value>,
    next: u32,
}
impl Client {
    fn start(command: &str) -> Self {
        let mut child = Command::new(command)
            .arg("--managed-stdio")
            .env("CODEPET_AGENT", "managed-stdio-test")
            // An unusable HTTP proxy must have no effect on stdio.
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .env("NO_PROXY", "")
            .env("no_proxy", "")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, messages) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let line = line.unwrap();
                let message =
                    serde_json::from_str(&line).expect("stdout must only contain MCP JSON");
                if tx.send(message).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            messages,
            next: 0,
        }
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        self.send(json!({"jsonrpc":"2.0", "id":self.next, "method":method, "params":params}));
        loop {
            let message = self
                .messages
                .recv_timeout(Duration::from_secs(15))
                .expect("MCP response");
            if message["id"] == self.next {
                assert!(message.get("error").is_none(), "{message}");
                return message["result"].clone();
            }
        }
    }
    fn send(&mut self, message: Value) {
        writeln!(self.child.stdin.as_mut().unwrap(), "{message}").unwrap();
    }
    fn initialize(&mut self) -> Value {
        let response = self.request(
            "initialize",
            json!({"protocolVersion":"2025-06-18",
            "capabilities":{}, "clientInfo":{"name":"managed-test", "version":"1"}}),
        );
        self.send(json!({"jsonrpc":"2.0", "method":"notifications/initialized"}));
        response
    }
    fn call(&mut self, name: &str, args: Value) -> Value {
        let response = self.request("tools/call", json!({"name":name, "arguments":args}));
        assert_ne!(response["isError"], true, "{response}");
        serde_json::from_str(response["content"][0]["text"].as_str().unwrap()).unwrap()
    }
    fn stop(&mut self) {
        drop(self.child.stdin.take());
        let started = Instant::now();
        while self.child.try_wait().unwrap().is_none() {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "stdio must exit on EOF"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn moved_app_and_concurrent_clients_share_the_pinned_ledger_without_http() {
    let dir = tempfile::Builder::new()
        .prefix("Code Pet 中文 ")
        .tempdir()
        .unwrap();
    let package = dir
        .path()
        .join("Applications/Code Pet.app/Contents/Resources/task-mcp");
    fs::create_dir_all(&package).unwrap();
    let source = package.join(managed::binary_name());
    fs::copy(env!("CARGO_BIN_EXE_codepet-task-mcp"), &source).unwrap();
    let workspace = dir.path().join("user/.codepet");
    let data = dir.path().join("custom data");
    let config = install(&source, &workspace, &data);
    let command = config["command"].as_str().unwrap();
    fs::rename(
        dir.path().join("Applications"),
        dir.path().join("Moved Applications"),
    )
    .unwrap();
    fs::remove_dir_all(dir.path().join("Moved Applications")).unwrap();
    let ledger = Ledger::new(&data);
    codepet_task_mcp::instructions::save(&ledger, 0, Some("pinned ledger custom instructions"))
        .unwrap();
    let mut a = Client::start(command);
    assert_eq!(
        a.initialize()["instructions"],
        "pinned ledger custom instructions"
    );
    assert_eq!(
        a.request("tools/list", json!({}))["tools"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    let receipt = a.call(
        "create-task",
        json!({
            "title":"isolated stdio fixture", "agentWorkspace":{"path":dir.path()},
            "initialRun":{"agentConversation":["managed-test-fixture"], "kind":"feature"}
        }),
    );
    assert_eq!(receipt["createdBy"], "managed-stdio-test");
    assert_eq!(ledger.list().unwrap().len(), 1);
    let mut b = Client::start(command);
    b.initialize();
    assert_eq!(
        b.call("find-task-runs", json!({"taskId":receipt["taskId"]}))["taskId"],
        receipt["taskId"]
    );
    // A user-completed task remains locked even for existing connections and custom prompts.
    let task = ledger.get(receipt["taskId"].as_str().unwrap()).unwrap();
    let closed = ledger
        .close(
            &task.id,
            &task.runs.last().unwrap().id,
            task.revision,
            false,
            "用户标记完成",
        )
        .unwrap();
    let before = json!(ledger.get(&task.id).unwrap());
    for client in [&mut a, &mut b] {
        let denied=client.request("tools/call",json!({"name":"create-task-run","arguments":{"taskId":task.id,"expectedRevision":closed.revision,"title":"试图自动重新开启","agentConversation":["managed-test-fixture"]}}));
        assert_eq!(denied["isError"], true);
        assert!(denied["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("已完成并锁定"));
    }
    assert_eq!(json!(ledger.get(&task.id).unwrap()), before);
    let restored=ledger.create_task_run(serde_json::from_value(json!({"taskId":task.id,"expectedRevision":closed.revision,"title":"用户手动继续","agentConversation":[]})).unwrap(),codepet_task_lineage::tasks::Actor::human()).unwrap();
    a.call("update-task-run",json!({"taskId":task.id,"runId":restored.run_id,"expectedRevision":restored.revision,"note":"用户恢复后更新当前执行","agentConversation":["managed-test-fixture"]}));
    assert_eq!(
        json!(ledger.get(&task.id).unwrap().runs[0]),
        before["runs"][0]
    );
    // Updating while clients run never replaces a locked bootstrap. The next
    // connection follows the new active data directory, old connections stay pinned.
    let updated_source = dir.path().join("new runner");
    fs::copy(env!("CARGO_BIN_EXE_codepet-task-mcp"), &updated_source).unwrap();
    install(&updated_source, &workspace, &dir.path().join("new data"));
    let mut c = Client::start(command);
    c.initialize();
    assert_eq!(c.call("find-task-runs", json!({}))["total"], 0);
    let found = a.call("find-task-runs", json!({}));
    assert_eq!(found["total"], 2);
    assert!(found["runs"].as_array().unwrap().iter().any(|run| {
        run["runId"] == receipt["runId"] && run["isCurrentRun"] == false
    }));
    assert!(found["runs"].as_array().unwrap().iter().any(|run| {
        run["runId"] == restored.run_id && run["isCurrentRun"] == true
    }));
    a.stop();
    b.stop();
    c.stop();
}
