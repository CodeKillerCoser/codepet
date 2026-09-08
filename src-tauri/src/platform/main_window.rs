use tauri::{AppHandle, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolbarAnchor {
    left: f64,
    center_y: f64,
}

/// Read AppKit geometry in logical points, on its main thread. Never estimate
/// native button sizes or tie window controls to the collapsible content grid.
#[tauri::command]
pub async fn main_window_toolbar_anchor(
    window: WebviewWindow,
) -> Result<Option<ToolbarAnchor>, String> {
    if window.label() != "main" {
        return Err("Toolbar geometry is only available to the main window".into());
    }
    #[cfg(target_os = "macos")]
    {
        use tauri_nspanel::objc2_app_kit::{NSWindow, NSWindowButton};
        let (send, receive) = tokio::sync::oneshot::channel();
        let target = window.clone();
        window
            .run_on_main_thread(move || {
                let result = (|| {
                    let handle = target.ns_window().map_err(|error| error.to_string())?;
                    // The window is retained by `target` for this main-thread read.
                    let native = unsafe { &*(handle as *const NSWindow) };
                    let content = native.contentView().ok_or("Missing window content view")?;
                    let button = native
                        .standardWindowButton(NSWindowButton::ZoomButton)
                        .ok_or("Missing native window controls")?;
                    let rect = button.convertRect_toView(button.bounds(), Some(&content));
                    let center_y = if content.isFlipped() {
                        rect.origin.y + rect.size.height / 2.0
                    } else {
                        content.bounds().size.height - rect.origin.y - rect.size.height / 2.0
                    };
                    Ok(Some(ToolbarAnchor {
                        left: rect.origin.x + rect.size.width + 12.0,
                        center_y,
                    }))
                })();
                let _ = send.send(result);
            })
            .map_err(|error| error.to_string())?;
        receive.await.map_err(|error| error.to_string())?
    }
    #[cfg(not(target_os = "macos"))]
    Ok(None)
}

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
