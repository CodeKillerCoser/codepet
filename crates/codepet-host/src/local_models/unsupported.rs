use super::{Installation, Manager};
use std::path::Path;
use tokio::sync::watch;
pub(super) async fn support() -> (bool, Option<String>) {
    (false,Some("MLX 本地环境需要 Apple Silicon Mac 和 macOS 26.2 或更新版本；仍可配置云端或其他兼容 API".into()))
}
pub(super) async fn detect(_root: &Path) -> Result<Option<Installation>, String> {
    Ok(None)
}
pub(super) async fn open(_path: &Path) -> Result<(), String> {
    Err("当前系统不支持打开 MLX".into())
}
pub(super) async fn install(
    _manager: &Manager,
    _id: &str,
    _image: &Path,
    _temporary: &Path,
    _cancel: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    Err("当前系统不支持 MLX 安装".into())
}
