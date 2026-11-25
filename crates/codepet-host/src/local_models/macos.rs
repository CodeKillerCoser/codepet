//! macOS app operations. Never invoke a shell or overwrite a system installation.
use super::{installer::CANCELLED, Installation, Manager};
use std::{
    path::{Path, PathBuf},
    process::{Output, Stdio},
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command, sync::watch};
fn command(program: &str) -> Command {
    let mut command =
        Command::from(codepet_provider_sdk::local_runtime::command(program).into_std());
    command
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}
async fn output(mut command: Command) -> Result<Output, String> {
    tokio::time::timeout(Duration::from_secs(15), command.output())
        .await
        .map_err(|_| "系统环境检测超时")?
        .map_err(|_| "无法读取系统环境".into())
}
async fn property(app: &Path, key: &str) -> Option<String> {
    let mut command = command("/usr/bin/plutil");
    command
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(app.join("Contents/Info.plist"));
    output(command)
        .await
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
}
async fn installation(app: &Path, source: &str) -> Option<Installation> {
    if !app.is_dir() || app.is_symlink() {
        return None;
    }
    let identifier = property(app, "CFBundleIdentifier").await?;
    if !matches!(
        identifier.as_str(),
        "com.dalcu.mlx-core" | "com.dalcu.mlx-serve"
    ) {
        return None;
    }
    Some(Installation {
        name: property(app, "CFBundleName")
            .await
            .unwrap_or_else(|| "MLX".into()),
        version: property(app, "CFBundleShortVersionString")
            .await
            .unwrap_or_else(|| "未知版本".into()),
        path: app.into(),
        source: source.into(),
    })
}
pub(super) async fn support() -> (bool, Option<String>) {
    if !cfg!(target_arch = "aarch64") {
        return (
            false,
            Some("MLX 本地环境需要 Apple Silicon Mac；仍可使用云端或其他兼容 API".into()),
        );
    }
    let mut cmd = command("/usr/bin/sw_vers");
    cmd.arg("-productVersion");
    let supported = output(cmd)
        .await
        .ok()
        .filter(|out| out.status.success())
        .is_some_and(|out| {
            let version = String::from_utf8_lossy(&out.stdout);
            let mut parts = version
                .trim()
                .split('.')
                .filter_map(|part| part.parse::<u32>().ok());
            (parts.next().unwrap_or(0), parts.next().unwrap_or(0)) >= (26, 2)
        });
    (
        supported,
        (!supported)
            .then(|| "MLX-Serve 安装需要 macOS 26.2 或更新版本；仍可使用云端或其他兼容 API".into()),
    )
}
pub(super) async fn detect(root: &Path) -> Result<Option<Installation>, String> {
    let home = codepet_provider_sdk::local_runtime::home_dir();
    let mut locations = vec![PathBuf::from("/Applications")];
    if let Some(home) = home {
        locations.push(home.join("Applications"));
    }
    for directory in locations {
        for name in ["MLX-Serve.app", "MLX Core.app"] {
            if let Some(app) = installation(&directory.join(name), "system").await {
                return Ok(Some(app));
            }
        }
    }
    // Spotlight also discovers applications the user keeps outside Applications.
    let mut cmd = command("/usr/bin/mdfind");
    cmd.arg("kMDItemCFBundleIdentifier == 'com.dalcu.mlx-core' || kMDItemCFBundleIdentifier == 'com.dalcu.mlx-serve'");
    if let Ok(found) = output(cmd).await {
        for path in String::from_utf8_lossy(&found.stdout).lines().take(20) {
            let path = Path::new(path);
            if !path.starts_with("/Volumes") && !path.starts_with(root) {
                if let Some(app) = installation(path, "system").await {
                    return Ok(Some(app));
                }
            }
        }
    }
    Ok(installation(&root.join("MLX-Serve.app"), "managed").await)
}
pub(super) async fn open(path: &Path) -> Result<(), String> {
    installation(path, "system")
        .await
        .ok_or("MLX 应用无效，请重新检测")?;
    let mut cmd = command("/usr/bin/open");
    cmd.arg("-a").arg(path);
    if !output(cmd).await?.status.success() {
        return Err("无法打开 MLX，请在访达中检查该应用".into());
    }
    Ok(())
}
async fn step(
    mut command: Command,
    cancel: &mut watch::Receiver<bool>,
    message: &str,
) -> Result<Output, String> {
    if *cancel.borrow() {
        return Err(CANCELLED.into());
    }
    let mut child = command.spawn().map_err(|_| format!("{message}失败"))?;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await.map(|_| bytes)
    });
    let err = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).await.map(|_| bytes)
    });
    let status = tokio::select! {
        _ = cancel.changed()=>Err(CANCELLED.into()),
        result=tokio::time::timeout(Duration::from_secs(120),child.wait())=>result.map_err(|_|format!("{message}超时")).and_then(|result|result.map_err(|_|format!("{message}失败")))
    };
    if status.is_err() {
        let _ = child.kill().await;
        let _ = child.wait().await;
        out.abort();
        err.abort();
        return Err(status.unwrap_err());
    }
    let result = Output {
        status: status.unwrap(),
        stdout: out
            .await
            .map_err(|_| format!("{message}失败"))?
            .map_err(|_| format!("{message}失败"))?,
        stderr: err
            .await
            .map_err(|_| format!("{message}失败"))?
            .map_err(|_| format!("{message}失败"))?,
    };
    if !result.status.success() {
        return Err(format!("{message}失败，请重试或手动安装"));
    }
    Ok(result)
}
pub(super) async fn install(
    manager: &Manager,
    id: &str,
    image: &Path,
    temporary: &Path,
    cancel: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    let mount = temporary.join("mount");
    std::fs::create_dir(&mount).map_err(|_| "无法创建安装挂载目录")?;
    let mut attach = command("/usr/bin/hdiutil");
    attach
        .args([
            "attach",
            "-readonly",
            "-nobrowse",
            "-noautoopen",
            "-mountpoint",
        ])
        .arg(&mount)
        .arg(image);
    // Always attempt detach, including an interrupted attach that may already have mounted.
    let result = async {
        step(attach, cancel, "打开安装包").await?;
        let app = std::fs::read_dir(&mount)
            .map_err(|_| "无法读取安装包")?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.extension().is_some_and(|extension| extension == "app")
                    && path.is_dir()
                    && !path.is_symlink()
            })
            .ok_or("安装包中没有 MLX 应用")?;
        installation(&app, "managed")
            .await
            .ok_or("安装包中的应用身份不符")?;
        let mut identity = command("/usr/bin/codesign");
        identity.args(["-dv", "--verbose=4"]).arg(&app);
        let identity = step(identity, cancel, "验证发布者").await?;
        if !String::from_utf8_lossy(&identity.stderr)
            .lines()
            .any(|line| line == "TeamIdentifier=S77TW432YG")
        {
            return Err("MLX 应用发布者校验失败，已停止安装".into());
        }
        let mut verify = command("/usr/bin/codesign");
        verify.args(["--verify", "--deep", "--strict"]).arg(&app);
        step(verify, cancel, "验证应用签名").await?;
        let staged = temporary.join("MLX-Serve.app");
        let mut copy = command("/usr/bin/ditto");
        copy.arg(&app).arg(&staged);
        step(copy, cancel, "复制 MLX 应用").await?;
        if !staged.join("Contents/Info.plist").is_file() {
            return Err("MLX 应用复制不完整".into());
        }
        manager.commit(id)?;
        let destination = manager.root.join("MLX-Serve.app");
        if destination.exists() {
            return Err("已存在 MLX 应用，请重新检测后使用；不会覆盖已有应用".into());
        }
        std::fs::rename(staged, destination).map_err(|_| "无法完成 MLX 安装")?;
        Ok(())
    }
    .await;
    let mounted_contents =
        std::fs::read_dir(&mount).is_ok_and(|mut entries| entries.next().is_some());
    let mut detach = command("/usr/bin/hdiutil");
    detach.args(["detach", "-force"]).arg(&mount);
    let detached = output(detach).await;
    if mounted_contents && !detached.is_ok_and(|out| out.status.success()) {
        return Err("临时安装镜像未能卸载，请在磁盘工具中推出 MLX 安装镜像后重试".into());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn system_probe_is_read_only_and_unknown_bundles_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        assert!(installation(root.path(), "system").await.is_none());
        let result = detect(root.path()).await.unwrap();
        if let Some(app) = result {
            assert!(app.path.is_absolute());
            assert!(!app.version.is_empty());
            assert_eq!(app.source, "system");
        }
    }
}

#[cfg(test)]
mod package_tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires CODEPET_MLX_QA_DMG pointing to an official signed MLX-Serve DMG"]
    async fn signed_package_installs_in_isolated_directory_and_never_overwrites() {
        let image = std::env::var_os("CODEPET_MLX_QA_DMG").expect("DMG path");
        let root = tempfile::tempdir().unwrap();
        let temporary = tempfile::tempdir_in(root.path()).unwrap();
        let (manager, mut cancel) = super::super::tests::running(root.path().into());
        install(
            &manager,
            "test-install",
            Path::new(&image),
            temporary.path(),
            &mut cancel,
        )
        .await
        .unwrap();
        let app = installation(&root.path().join("MLX-Serve.app"), "managed")
            .await
            .unwrap();
        assert_eq!(app.source, "managed");
        assert!(!app.version.is_empty());
        assert!(
            !temporary.path().join("mount/MLX-Serve.app").exists(),
            "image must be detached"
        );
        let second = tempfile::tempdir_in(root.path()).unwrap();
        let error = install(
            &manager,
            "test-install",
            Path::new(&image),
            second.path(),
            &mut cancel,
        )
        .await
        .unwrap_err();
        assert!(error.contains("不会覆盖"));
        assert!(app.path.join("Contents/Info.plist").exists());
    }
    #[tokio::test]
    async fn cancellation_stops_and_reaps_native_process() {
        let (tx, mut rx) = watch::channel(false);
        let mut cmd = command("/bin/sleep");
        cmd.arg("30");
        let task = tokio::spawn(async move { step(cmd, &mut rx, "测试").await });
        tokio::task::yield_now().await;
        tx.send_replace(true);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err(),
            CANCELLED
        );
    }
}
