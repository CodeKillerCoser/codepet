use super::{platform, Manager, Plan, Progress};
use fs2::FileExt;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Write,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{io::AsyncWriteExt, sync::watch};
pub(super) const CANCELLED: &str = "installation_cancelled";
pub(super) fn lease(root: &Path) -> Result<File, String> {
    std::fs::create_dir_all(root).map_err(|_| "无法创建本地模型目录")?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("install.lock"))
        .map_err(|_| "无法打开安装锁")?;
    file.try_lock_exclusive()
        .map_err(|_| "另一个 CodePet 实例正在安装，请稍后重试")?;
    Ok(file)
}
pub(super) fn persist(root: &Path, progress: &Progress) -> Result<(), String> {
    let mut file = tempfile::NamedTempFile::new_in(root).map_err(|_| "无法保存安装状态")?;
    file.write_all(&serde_json::to_vec(progress).unwrap())
        .map_err(|_| "无法保存安装状态")?;
    file.persist(root.join("installation.json"))
        .map_err(|_| "无法保存安装状态")?;
    Ok(())
}
fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("CodePet-local-model-installer")
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() > 5
                || attempt.url().scheme() != "https"
                || !matches!(
                    attempt.url().host_str(),
                    Some(
                        "github.com"
                            | "release-assets.githubusercontent.com"
                            | "objects.githubusercontent.com"
                    )
                )
            {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|_| "无法初始化安装下载".into())
}
pub(super) async fn prepare() -> Result<Plan, String> {
    let mut response = client()?
        .get("https://api.github.com/repos/ddalcu/mlx-serve/releases/latest")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| "无法获取 MLX 官方安装信息，请检查网络")?;
    if !response.status().is_success() {
        return Err("MLX 官方安装源暂不可用，请稍后重试".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "读取安装信息失败")? {
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err("安装信息超出大小限制".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    release(&serde_json::from_slice(&bytes).map_err(|_| "官方安装信息格式无效")?)
}
fn release(value: &Value) -> Result<Plan, String> {
    let asset = value["assets"]
        .as_array()
        .and_then(|assets| assets.iter().find(|asset| asset["name"] == "MLX-Serve.dmg"))
        .ok_or("官方版本没有可用的 macOS 安装包")?;
    let url = asset["browser_download_url"]
        .as_str()
        .ok_or("缺少安装包地址")?;
    let parsed = reqwest::Url::parse(url).map_err(|_| "安装包地址无效")?;
    let digest = asset["digest"]
        .as_str()
        .and_then(|value| value.strip_prefix("sha256:"))
        .filter(|value| value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit()))
        .ok_or("官方安装包缺少 SHA-256 校验，暂不支持自动安装")?;
    let size = asset["size"]
        .as_u64()
        .filter(|size| *size > 0 && *size <= 1024 * 1024 * 1024)
        .ok_or("安装包大小无效")?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("github.com")
        || !parsed
            .path()
            .starts_with("/ddalcu/mlx-serve/releases/download/")
        || parsed.query().is_some()
    {
        return Err("安装包不是官方发布地址".into());
    }
    Ok(Plan {
        id: uuid::Uuid::new_v4().to_string(),
        version: value["tag_name"].as_str().ok_or("缺少版本号")?.into(),
        bytes: size,
        source_url: "https://github.com/ddalcu/mlx-serve/releases".into(),
        url: url.into(),
        digest: digest.into(),
        created: Some(Instant::now()),
    })
}
pub(super) async fn run(
    manager: Arc<Manager>,
    id: &str,
    plan: Plan,
    mut cancel: watch::Receiver<bool>,
) -> Result<(), String> {
    if *cancel.borrow() {
        return Err(CANCELLED.into());
    }
    let temporary = tempfile::Builder::new()
        .prefix("install-")
        .tempdir_in(&manager.root)
        .map_err(|_| "无法创建安装临时目录")?;
    let image = temporary.path().join("MLX-Serve.dmg");
    manager.progress(id, "downloading", "正在下载 MLX-Serve", 0);
    let mut response = tokio::select! {_ = cancel.changed()=>return Err(CANCELLED.into()),response=client()?.get(&plan.url).timeout(Duration::from_secs(1800)).send()=>response.map_err(|_|"下载安装包失败，请检查网络后重试")?};
    if !response.status().is_success() {
        return Err("官方安装包下载失败，请稍后重试".into());
    }
    let mut file = tokio::fs::File::create(&image)
        .await
        .map_err(|_| "无法写入安装临时文件")?;
    let mut downloaded = 0u64;
    let mut hash = Sha256::new();
    loop {
        let chunk = tokio::select! {_ = cancel.changed()=>return Err(CANCELLED.into()),chunk=response.chunk()=>chunk.map_err(|_|"下载中断，可重新安装")?};
        let Some(chunk) = chunk else { break };
        downloaded += chunk.len() as u64;
        if downloaded > plan.bytes {
            return Err("安装包大小不符，已停止安装".into());
        }
        hash.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|_| "写入下载文件失败，请检查磁盘空间")?;
        manager.progress(id, "downloading", "正在下载 MLX-Serve", downloaded);
    }
    file.flush().await.map_err(|_| "保存安装包失败")?;
    drop(file);
    manager.progress(id, "verifying", "校验官方安装包", downloaded);
    if downloaded != plan.bytes || format!("{:x}", hash.finalize()) != plan.digest.to_lowercase() {
        return Err("安装包校验失败，请重新下载".into());
    }
    if *cancel.borrow() {
        return Err(CANCELLED.into());
    }
    manager.progress(id, "installing", "正在安装 MLX-Serve", downloaded);
    platform::install(&manager, id, &image, temporary.path(), &mut cancel).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn release_requires_official_asset_and_integrity() {
        let mut info = json!({"tag_name":"v1","assets":[{"name":"MLX-Serve.dmg","browser_download_url":"https://github.com/ddalcu/mlx-serve/releases/download/v1/MLX-Serve.dmg","size":100,"digest":format!("sha256:{}","a".repeat(64))}]});
        assert_eq!(release(&info).unwrap().bytes, 100);
        info["assets"][0]["browser_download_url"] = json!("https://evil.invalid/app.dmg");
        assert!(release(&info).is_err());
        info["assets"][0]["digest"] = Value::Null;
        assert!(release(&info).is_err());
    }
    #[test]
    fn install_lease_blocks_other_writers() {
        let root = tempfile::tempdir().unwrap();
        let a = lease(root.path()).unwrap();
        assert!(lease(root.path()).is_err());
        drop(a);
        assert!(lease(root.path()).is_ok());
    }
}

#[cfg(test)]
mod download_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn cancelling_partial_download_removes_temporary_files() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/test.dmg", listener.local_addr().unwrap());
        let (started, ready) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 2048];
            socket.read(&mut request).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial")
                .await
                .unwrap();
            let _ = started.send(());
            tokio::time::sleep(Duration::from_secs(30)).await;
        });
        let root = tempfile::tempdir().unwrap();
        let (manager, rx) = super::super::tests::running(root.path().into());
        let plan = Plan {
            id: "test".into(),
            version: "test".into(),
            bytes: 100,
            source_url: String::new(),
            url,
            digest: "0".repeat(64),
            created: Some(Instant::now()),
        };
        let task = tokio::spawn(run(manager.clone(), "test-install", plan, rx));
        ready.await.unwrap();
        manager
            .state
            .lock()
            .unwrap()
            .job
            .as_ref()
            .unwrap()
            .cancel
            .send_replace(true);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err(),
            CANCELLED
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        server.abort();
    }
}

#[cfg(test)]
mod official_source_tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires network access to the official GitHub release API; metadata only"]
    async fn official_latest_release_has_supported_install_metadata() {
        let plan = prepare().await.unwrap();
        assert!(plan.bytes > 0);
        assert_eq!(plan.digest.len(), 64);
        assert!(plan
            .url
            .starts_with("https://github.com/ddalcu/mlx-serve/releases/download/"));
    }
}
