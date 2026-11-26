//! Desktop integration for the stable standalone stdio installation.
use serde_json::Value;
use std::path::PathBuf;

pub(crate) async fn configuration(app: &tauri::AppHandle, data: PathBuf) -> Result<Value, String> {
    let span = crate::app_log::PerfSpan::start("task_graph.components_ready");
    let paths = crate::paths::for_package(app.package_info()).map_err(|e| e.to_string())?;
    let source = packaged_binary(&paths.resources);
    let result = tauri::async_runtime::spawn_blocking(move || {
        codepet_task_mcp::managed::install(&source, &paths.workspace, &data)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?;
    match &result {
        Ok(_) => span.finish_ok(&[]),
        Err(_) => span.finish_error("component installation failed", &[]),
    }
    result
}

fn packaged_binary(resources: &std::path::Path) -> PathBuf {
    let name = codepet_task_mcp::managed::binary_name();
    let packaged = resources.join("task-mcp").join(&name);
    #[cfg(debug_assertions)]
    if !packaged.is_file() {
        return PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/task-mcp")
            .join(name);
    }
    packaged
}
