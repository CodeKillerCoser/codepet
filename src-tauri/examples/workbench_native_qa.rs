//! Native acceptance: production component + production IPC + real local Codex/Git.
//! Only QA preferences/reports are written; the installed application is left running.
use serde_json::{json, Value};
use std::{path::PathBuf, sync::OnceLock};
use tauri::Manager;
static QA_DATA: OnceLock<PathBuf> = OnceLock::new();
static QA_CONFIG: OnceLock<Value> = OnceLock::new();
static QA_REPORT: OnceLock<PathBuf> = OnceLock::new();
mod paths {
    pub fn current() -> Result<codepet_paths::PathManager, String> {
        let root = crate::QA_DATA.get().unwrap().clone();
        Ok(codepet_paths::PathManager {
            install: root.clone(),
            data: root.clone(),
            workspace: root.clone(),
            resources: root.clone(),
            settings_file: root.join("settings.json"),
        })
    }
    pub fn for_package(_: &tauri::PackageInfo) -> Result<codepet_paths::PathManager, String> {
        let mut paths = current()?;
        paths.resources = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
        Ok(paths)
    }
}
#[path = "../src/platform/workbench_actions.rs"]
pub mod qa_actions;
mod platform {
    pub use crate::qa_actions as workbench_actions;
}
#[path = "../src/codex_workbench.rs"]
mod codex_workbench;
#[path = "../src/task_ledger.rs"]
mod task_ledger;
#[path = "../src/task_mcp_setup.rs"]
mod task_mcp_setup;
#[tauri::command]
async fn qa_config() -> Value {
    eprintln!("native QA config requested");
    QA_CONFIG.get().unwrap().clone()
}
// QA-only client: HTTP originates in Rust, never in the production frontend.
#[tauri::command]
async fn qa_http_task_write(
    runtime: tauri::State<'_, codepet_task_mcp::http::HttpRuntime>,
) -> Result<Value, String> {
    let setup = runtime.configuration().await?;
    let config = setup["config"].as_str().ok_or("missing HTTP config")?
        .parse::<toml_edit::DocumentMut>().map_err(|e| e.to_string())?;
    let authorization = config["mcp_servers"]["codepet_tasks"]["http_headers"]["Authorization"]
        .as_str().ok_or("missing local credential")?;
    let url = setup["url"].as_str().ok_or("missing HTTP URL")?;
    let client = reqwest::Client::builder().no_proxy().build().map_err(|e| e.to_string())?;
    let init: Value = client.post(url).header("Authorization", authorization).header("X-CodePet-Agent", "native-qa")
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"native-qa","version":"1"}
        }})).send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    let task_id = QA_CONFIG.get().unwrap()["taskId"].as_str().ok_or("missing task ID")?;
    let task = runtime.ledger().get(task_id)?;
    let result: Value = client.post(url).header("Authorization", authorization).header("X-CodePet-Agent", "native-qa")
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", init["result"]["protocolVersion"].as_str().ok_or("missing protocol")?)
        .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name":"update-task-run","arguments":{"agentConversation":["00000000-0000-4000-8000-000000000001"],"taskId":task_id,"runId":task.runs.last().unwrap().id,"expectedRevision":task.revision,
                "note":"原生 HTTP MCP 写入，随后由生产 UI 与 IPC 读取"}
        }})).send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    if result.get("error").is_some() || result["result"]["isError"] == true {
        return Err(format!("native HTTP MCP write failed: {result}"));
    }
    let mut receipt: Value = serde_json::from_str(result["result"]["content"][0]["text"].as_str().ok_or("missing receipt")?).map_err(|e| e.to_string())?;
    receipt["initializeInstructions"] = init["result"]["instructions"].clone();
    Ok(receipt)
}

#[tauri::command]
async fn qa_stdio_initialize(app: tauri::AppHandle,
    runtime: tauri::State<'_, codepet_task_mcp::http::HttpRuntime>,
) -> Result<Value, String> {
    let setup = task_mcp_setup::configuration(&app, runtime.ledger().data_directory().to_owned()).await?;
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::{BufRead, Write};
        use std::process::{Command, Stdio};
        let mut child = Command::new(setup["command"].as_str().ok_or("missing stdio command")?)
            .arg("--managed-stdio").env("CODEPET_AGENT", "native-qa")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit())
            .spawn().map_err(|e| e.to_string())?;
        let mut input = child.stdin.take().unwrap();
        let request = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"native-qa","version":"1"}
        }});
        writeln!(input, "{request}").map_err(|e| e.to_string())?;
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).map_err(|e| e.to_string())?;
        drop(input);
        let status = child.wait().map_err(|e| e.to_string())?;
        if !status.success() { return Err(format!("stdio exited: {status}")); }
        let initialized: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if initialized.get("error").is_some() { return Err(initialized.to_string()); }
        Ok(json!({"initializeInstructions":initialized["result"]["instructions"]}))
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn qa_trace(value: Value) {
    eprintln!("native QA trace: {value}");
}
#[tauri::command]
async fn qa_report(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    mut value: Value,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if value["success"] == true {
        let capture = QA_REPORT
            .get()
            .unwrap()
            .with_file_name("native-window.tiff");
        let capture_path = capture.clone();
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
        window.with_webview(move |webview| {
            let sender=std::sync::Mutex::new(Some(tx));
            let block=block2::RcBlock::new(move |image:*mut objc2::runtime::AnyObject,_error:*mut objc2::runtime::AnyObject| {
                let result=if image.is_null(){Err("WKWebView snapshot returned no image".into())}else{unsafe{
                    let data:*mut objc2::runtime::AnyObject=objc2::msg_send![image,TIFFRepresentation];
                    let file=objc2_foundation::NSString::from_str(capture_path.to_str().unwrap());
                    let written:bool=objc2::msg_send![data,writeToFile:&*file,atomically:true];
                    if written {Ok(())}else{Err("Unable to write webview snapshot".into())}
                }};
                if let Some(tx)=sender.lock().unwrap().take(){let _=tx.send(result);}
            });
            unsafe {let _:()=objc2::msg_send![webview.inner() as *mut objc2::runtime::AnyObject,takeSnapshotWithConfiguration:std::ptr::null::<objc2::runtime::AnyObject>(),completionHandler:&*block];}
        }).map_err(|e|e.to_string())?;
        match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
            Ok(Ok(Ok(()))) => value["screenshotTiff"] = json!(capture),
            result => value["screenshotError"] = json!(format!("{result:?}")),
        }
    }
    let _ = window;
    std::fs::write(
        QA_REPORT.get().unwrap(),
        serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let success = value["success"] == true;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        app.exit(if success { 0 } else { 1 });
    });
    Ok(())
}

fn main() {
    let data = tempfile::Builder::new()
        .prefix("codepet-workbench-acceptance-")
        .tempdir()
        .unwrap();
    QA_DATA.set(data.path().to_owned()).unwrap();
    let mut expected: Value = serde_json::from_str(
        &std::env::var("CODEPET_QA_EXPECTED").expect("expected workspace/project/thread required"),
    )
    .unwrap();
    if expected["mode"] == "tasks" {
        use codepet_task_lineage::tasks::{Actor, Ledger, CreateTaskInput, UpdateTaskRunInput};
        let ledger=Ledger::new(data.path());
        let workspace=expected["workspace"].as_str().unwrap();
        let output=std::process::Command::new("git").args(["-C",workspace,"rev-parse","HEAD"]).output().unwrap();
        assert!(output.status.success());
        let commit=String::from_utf8(output.stdout).unwrap().trim().to_owned();
        let input:CreateTaskInput=serde_json::from_value(json!({"agentWorkspace":{"path":expected["workspace"],"relatedCommits":[commit]},"title":"修复桌宠菜单点击失效","description":"切换窗口后首次点击无响应，影响菜单操作。","initialRun":{"agentConversation":["00000000-0000-4000-8000-000000000001"],"title":"定位点击事件","kind":"bugfix","goal":"切换窗口后菜单正常响应","solution":"修复焦点切换后的点击事件绑定","description":"定位窗口焦点切换后的事件绑定，修复点击处理并验证菜单。","validationPlan":{"items":[{"title":"切换窗口后点击","precondition":"应用运行十分钟","steps":"切换到其他窗口，再点击桌宠","expected":"菜单正常打开"},{"title":"连续点击仍保持响应","steps":"连续打开和关闭菜单","expected":"每次点击均正常响应"}]}}})).unwrap();
        let created=ledger.create_task(input,Actor::agent("codex").unwrap()).unwrap();
        let update:UpdateTaskRunInput=serde_json::from_value(json!({"agentConversation":["00000000-0000-4000-8000-000000000001"],"taskId":created.task_id,"runId":created.run_id,"expectedRevision":created.revision,"status":"published","delivery":"原生任务验收样例 · 1.2.0","note":"本地实现和自测已完成，等待发布验收","verification":{"planRevision":1,"stage":"local","target":"当前未提交工作区","environment":"macOS","results":[{"criterionId":created.criterion_ids[0],"result":"passed","evidence":"本地实际点击验证通过"}]}})).unwrap();
        ledger.update_task_run(update,Actor::agent("codex").unwrap()).unwrap();
        expected["taskId"]=json!(created.task_id);
    }
    std::fs::write(
        data.path().join("codex-workbench-selection.json"),
        json!([expected["projectId"]]).to_string(),
    )
    .unwrap();
    QA_CONFIG.set(expected).unwrap();
    let report = PathBuf::from(std::env::var("CODEPET_QA_REPORT").expect("report path required"));
    QA_REPORT.set(report).unwrap();
    tauri::Builder::default()
        .on_page_load(|window,payload| {
            eprintln!("native QA page {:?}: {}",payload.event(),payload.url());
            if matches!(payload.event(),tauri::webview::PageLoadEvent::Finished) {
                let _=window.eval("window.addEventListener('error', e => window.__TAURI_INTERNALS__.invoke('qa_trace',{value:{error:e.message}}));window.addEventListener('unhandledrejection', e => window.__TAURI_INTERNALS__.invoke('qa_trace',{value:{rejection:String(e.reason)}}));window.__TAURI_INTERNALS__.invoke('qa_trace',{value:{url:location.href,ready:document.readyState,body:document.body?.innerText?.slice(0,150)}})");
            }
        })
        .invoke_handler(tauri::generate_handler![
            codex_workbench::codex_workbench_request,
            task_ledger::task_ledger_request,
            qa_config,
            qa_http_task_write,
            qa_stdio_initialize,
            qa_trace,
            qa_report
        ])
        .setup(|app| {
            eprintln!("native QA setup ready");
            app.manage(codepet_task_mcp::http::HttpRuntime::start_on(
                tauri::async_runtime::handle().inner(), crate::QA_DATA.get().unwrap().clone(), 0,
            ));
            let app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(90)).await;
                app.exit(2);
            });
            Ok(())
        })
        .run(tauri::generate_context!(
            "examples/workbench-native-qa/tauri.conf.json"
        ))
        .expect("native workbench acceptance failed");
}
