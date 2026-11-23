//! Native acceptance: production component + production IPC + real local Codex/Git.
//! Only QA preferences/reports are written; the installed application is left running.
use serde_json::{json, Value};
use std::{path::PathBuf, sync::OnceLock};
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
}
#[path = "../src/platform/workbench_actions.rs"]
pub mod qa_actions;
mod platform {
    pub use crate::qa_actions as workbench_actions;
}
#[path = "../src/codex_workbench.rs"]
mod codex_workbench;
#[tauri::command]
async fn qa_config() -> Value {
    eprintln!("native QA config requested");
    QA_CONFIG.get().unwrap().clone()
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
    let expected: Value = serde_json::from_str(
        &std::env::var("CODEPET_QA_EXPECTED").expect("expected workspace/project/thread required"),
    )
    .unwrap();
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
            qa_config,
            qa_trace,
            qa_report
        ])
        .setup(|app| {
            eprintln!("native QA setup ready");
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
