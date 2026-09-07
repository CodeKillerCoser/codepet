use tauri::{AppHandle, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// Recreated main windows must use the same chrome as the configured startup window.
pub fn create(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Code Pet")
        .inner_size(980.0, 700.0)
        .min_inner_size(820.0, 600.0)
        .resizable(true);
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);
    builder.build()
}
