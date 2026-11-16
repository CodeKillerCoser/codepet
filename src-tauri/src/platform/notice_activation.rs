//! One platform adapter for Notice activation. Business/UI code never runs OS commands.
use codepet_host::provider_sdk::{
    common::{NoticeActionResultStatus, NoticeActivationTarget},
    ProtocolError, ProtocolFuture,
};
pub struct SystemNoticeActivator;
impl codepet_host::NoticeActivator for SystemNoticeActivator {
    fn activate<'a>(
        &'a self,
        target: NoticeActivationTarget,
    ) -> ProtocolFuture<'a, NoticeActionResultStatus> {
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut command = match target {
                    NoticeActivationTarget::NoticeApplicationTarget(target) => {
                        if target.application_id.is_empty()
                            || !target
                                .application_id
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                        {
                            return Err(error("Invalid application identity"));
                        }
                        application_command(&target.application_id)
                    }
                    NoticeActivationTarget::NoticeUriTarget(target) => {
                        let uri = tauri::Url::parse(&target.uri)
                            .map_err(|_| error("Invalid activation URI"))?;
                        if matches!(uri.scheme(), "file" | "data" | "javascript") {
                            return Err(error("Unsupported activation URI scheme"));
                        }
                        uri_command(&target.uri)
                    }
                    _ => return Err(error("Unsupported activation target")),
                };
                let status = command.status().map_err(|e| error(&e.to_string()))?;
                if !status.success() {
                    return Err(error("The platform rejected the activation request"));
                }
                // OS launch acceptance is not proof that a particular window gained focus.
                Ok(NoticeActionResultStatus::Accepted)
            })
            .await
            .map_err(|e| error(&e.to_string()))?
        })
    }
}
fn error(message: &str) -> ProtocolError {
    ProtocolError {
        code: "notice_activation_failed".into(),
        message: message.into(),
        retryable: false,
        details: None,
    }
}
#[cfg(target_os = "macos")]
fn application_command(id: &str) -> std::process::Command {
    let mut command = std::process::Command::new("open");
    command.args(["-b", id]);
    command
}
#[cfg(target_os = "windows")]
fn application_command(id: &str) -> std::process::Command {
    let mut command = std::process::Command::new("explorer.exe");
    command.arg(format!("shell:AppsFolder\\{id}"));
    command
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn application_command(id: &str) -> std::process::Command {
    let mut command = std::process::Command::new("gtk-launch");
    command.arg(id);
    command
}
#[cfg(target_os = "macos")]
fn uri_command(uri: &str) -> std::process::Command {
    let mut command = std::process::Command::new("open");
    command.arg(uri);
    command
}
#[cfg(target_os = "windows")]
fn uri_command(uri: &str) -> std::process::Command {
    let mut command = std::process::Command::new("rundll32.exe");
    command.args(["url.dll,FileProtocolHandler", uri]);
    command
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn uri_command(uri: &str) -> std::process::Command {
    let mut command = std::process::Command::new("xdg-open");
    command.arg(uri);
    command
}
