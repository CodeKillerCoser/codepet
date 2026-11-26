use codepet_model_gateway::{settings, ModelGateway};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Mutex,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
};

#[derive(Deserialize)]
struct Handshake {
    protocol: u32,
    endpoint: String,
    token: String,
    owned: bool,
}
pub struct Process {
    endpoint: String,
    token: String,
    client: reqwest::Client,
    pub fingerprint: String,
    child: Mutex<Option<Child>>,
    gateway: Mutex<Option<ModelGateway>>,
}
pub fn fingerprint(settings: &settings::Settings) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(settings).unwrap())
    )
}
pub fn bundle() -> Option<PathBuf> {
    std::env::var_os("CODEPET_TASK_RECALL_BUNDLE")
        .map(PathBuf::from)
        .or_else(|| {
            // Older immutable stdio bootstraps do not forward new manifest fields.
            // Resolve the pinned bundle in the runner as well.
            let paths = codepet_paths::PathManager::discover().ok()?;
            let current: Value = serde_json::from_slice(
                &std::fs::read(paths.workspace.join("tools/task-mcp/current.json")).ok()?,
            )
            .ok()?;
            let bundle = PathBuf::from(current["recallBundle"].as_str()?);
            bundle.is_absolute().then_some(bundle)
        })
        .or_else(|| {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../artifacts/task-recall-bundle/native");
            cfg!(debug_assertions).then_some(root)
        })
}
impl Process {
    #[cfg(test)]
    pub(super) fn test_client(endpoint: String, fingerprint: String) -> Self {
        Self {
            endpoint,
            token: "test".into(),
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            fingerprint,
            child: Mutex::new(None),
            gateway: Mutex::new(None),
        }
    }
    pub async fn start(data: &Path, settings: settings::Settings) -> Result<Self, String> {
        let root = bundle().ok_or("未安装任务召回 sidecar")?;
        // An attached owner may exit between its handshake and our first health
        // request. Retry startup once, never replay an application mutation.
        for attempt in 0..2 {
            match Self::start_at(&root, data, &settings).await {
                Err(error) if attempt == 0 && error == "任务召回服务连接失败" => continue,
                result => return result,
            }
        }
        unreachable!()
    }
    async fn start_at(
        root: &Path,
        data: &Path,
        settings: &settings::Settings,
    ) -> Result<Self, String> {
        let bytes =
            std::fs::read(root.join("manifest.json")).map_err(|_| "任务召回 sidecar 未打包")?;
        let bundle_id = format!("{:x}", Sha256::digest(&bytes));
        let manifest: Value = serde_json::from_slice(&bytes).map_err(|_| "任务召回清单无效")?;
        let relative = manifest["pythonPath"]
            .as_str()
            .ok_or("任务召回清单缺少 Python")?;
        if Path::new(relative)
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err("任务召回路径无效".into());
        }
        let gateway = ModelGateway::start(data, settings.clone()).await?;
        let fingerprint = fingerprint(settings);
        let config = json!({"enabled":settings.enabled,"fingerprint":fingerprint,"bundleId":bundle_id,"usageTracking":true,
            "llm":{"model":settings.llm.model},"embedding":{"model":settings.embedding.model,"dimensions":settings.embedding.dimensions},
            "gatewayEndpoint":gateway.endpoint,"gatewayToken":gateway.token});
        let mut command = Command::new(root.join(relative));
        std::fs::create_dir_all(data).map_err(|e| e.to_string())?;
        command.env_clear().current_dir(data);
        for key in [
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "SYSTEMROOT",
            "WINDIR",
            "TEMP",
            "TMP",
            "TMPDIR",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command.env(
            "PATH",
            if cfg!(windows) {
                std::env::var_os("SYSTEMROOT")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("C:\\Windows"))
                    .join("System32")
                    .into_os_string()
            } else {
                "/usr/bin:/bin".into()
            },
        );
        command
            .args(["-I", "-B", "-X", "utf8"])
            .arg(root.join("main.py"))
            .arg("--data-dir")
            .arg(data)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .env("OMP_NUM_THREADS", "4")
            .env("OPENBLAS_NUM_THREADS", "4")
            .env("VECLIB_MAXIMUM_THREADS", "4")
            .env("MKL_NUM_THREADS", "4")
            .env("AWS_EC2_METADATA_DISABLED", "true")
            .env("NO_PROXY", "127.0.0.1,localhost,::1")
            .env("no_proxy", "127.0.0.1,localhost,::1");
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command.spawn().map_err(|_| "任务召回进程无法启动")?;
        let mut bytes = serde_json::to_vec(&config).unwrap();
        bytes.push(b'\n');
        tokio::time::timeout(
            Duration::from_secs(2),
            child.stdin.as_mut().unwrap().write_all(&bytes),
        )
        .await
        .map_err(|_| "任务召回配置传递超时")?
        .map_err(|_| "任务召回配置传递失败")?;
        let output = child.stdout.take().unwrap();
        let mut reader = BufReader::new(tokio::io::AsyncReadExt::take(output, 65536));
        let mut line = String::new();
        let size = tokio::time::timeout(Duration::from_secs(15), reader.read_line(&mut line))
            .await
            .map_err(|_| "任务召回启动超时")?
            .map_err(|_| "任务召回握手失败")?;
        if size == 0 {
            return Err("任务召回进程在就绪前退出，请检查组件完整性".into());
        }
        let hello: Value = serde_json::from_str(&line).map_err(|_| "任务召回握手无效")?;
        if let Some(error) = hello["error"].as_str() {
            return Err(match error {
                "incompatible_service" => {
                    "旧版或不同配置的召回服务仍在运行，请关闭旧版 Code Pet 并重新连接 Agent"
                }
                _ => "任务召回服务连接失败",
            }
            .into());
        }
        let hello: Handshake = serde_json::from_value(hello).map_err(|_| "任务召回握手无效")?;
        let url = reqwest::Url::parse(&hello.endpoint).map_err(|_| "任务召回地址无效")?;
        if hello.protocol != 1
            || url.scheme() != "http"
            || url.host_str() != Some("127.0.0.1")
            || url.port().is_none()
            || url.path() != "/"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || hello.token.len() != 64
            || !hello.token.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err("任务召回握手校验失败".into());
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(58))
            .build()
            .map_err(|e| e.to_string())?;
        let process = Self {
            endpoint: hello.endpoint,
            token: hello.token,
            client,
            fingerprint,
            child: Mutex::new(None),
            gateway: Mutex::new(None),
        };
        if hello.owned {
            *process.child.lock().unwrap() = Some(child);
            *process.gateway.lock().unwrap() = Some(gateway);
        } else {
            tokio::time::timeout(Duration::from_secs(2), child.wait())
                .await
                .map_err(|_| "任务召回接入进程未退出")?
                .map_err(|_| "任务召回接入进程退出失败")?;
        }
        if let Err(error) = process.wait_ready(&bundle_id).await {
            process.shutdown().await;
            return Err(error);
        }
        Ok(process)
    }
    async fn wait_ready(&self, bundle_id: &str) -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            if let Ok(Ok(health)) = tokio::time::timeout_at(
                deadline.min(tokio::time::Instant::now() + Duration::from_millis(500)),
                self.request("/health", None),
            )
            .await
            {
                if health["protocol"] != 1
                    || health["fingerprint"] != self.fingerprint
                    || health["bundleId"] != bundle_id
                {
                    return Err(
                        "召回服务版本或模型配置不匹配，请关闭旧版 Code Pet 并重新连接 Agent".into(),
                    );
                }
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("任务召回服务连接失败".into());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    pub async fn shutdown(&self) {
        let child = self.child.lock().unwrap().take();
        if let Some(mut child) = child {
            let _ = child.start_kill();
            // The OS singleton lock is released on exit, not when kill is sent.
            let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
        }
        self.gateway.lock().unwrap().take();
    }
    pub fn stop(&self) {
        self.child.lock().unwrap().take();
        self.gateway.lock().unwrap().take();
    }
    pub async fn request(&self, path: &str, body: Option<Value>) -> Result<Value, String> {
        let request = if let Some(body) = body {
            self.client
                .post(format!("{}{path}", self.endpoint))
                .json(&body)
        } else {
            self.client.get(format!("{}{path}", self.endpoint))
        };
        let mut response = request
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|_| "任务召回服务连接失败")?;
        if !response.status().is_success() {
            return Err(format!("任务召回请求失败 ({})", response.status().as_u16()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "任务召回响应中断")? {
            if bytes.len() + chunk.len() > 32 * 1024 * 1024 {
                return Err("任务召回响应过大".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| "任务召回响应无效".into())
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn readiness_retries_transient_health_but_rejects_an_old_bundle() {
        use axum::{http::StatusCode, response::IntoResponse, routing::get, Json, Router};
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let handler_calls = calls.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/health", get(move || {
                let calls = handler_calls.clone();
                async move {
                    if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                        StatusCode::SERVICE_UNAVAILABLE.into_response()
                    } else {
                        Json(json!({"protocol":1,"fingerprint":"wanted","bundleId":"current"})).into_response()
                    }
                }
            }))).await.unwrap();
        });
        let process = Process::test_client(endpoint, "wanted".into());
        process.wait_ready("current").await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(process
            .wait_ready("another-version")
            .await
            .unwrap_err()
            .contains("不匹配"));
        server.abort();
    }

    #[tokio::test]
    #[ignore = "requires CODEPET_RECALL_LIFECYCLE_BUNDLE with the current Python sources"]
    async fn native_start_attach_and_immediate_restart_release_the_owner_lock() {
        let root = PathBuf::from(std::env::var_os("CODEPET_RECALL_LIFECYCLE_BUNDLE").unwrap());
        let data = tempfile::tempdir().unwrap();
        let config = settings::Settings::default();
        for _ in 0..10 {
            let owner = Process::start_at(&root, data.path(), &config)
                .await
                .unwrap();
            assert!(owner.child.lock().unwrap().is_some());
            let attached = Process::start_at(&root, data.path(), &config)
                .await
                .unwrap();
            assert!(attached.child.lock().unwrap().is_none());
            assert_eq!(owner.endpoint, attached.endpoint);
            attached.shutdown().await;
            assert_eq!(owner.request("/health", None).await.unwrap()["protocol"], 1);
            owner.shutdown().await;
            assert!(owner.child.lock().unwrap().is_none());
            assert!(attached.request("/health", None).await.is_err());
            // No sleep: the next iteration must acquire the released OS lock.
        }
    }
}
