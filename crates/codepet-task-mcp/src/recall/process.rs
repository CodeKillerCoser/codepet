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
        let manifest: Value = serde_json::from_slice(
            &std::fs::read(root.join("manifest.json")).map_err(|_| "任务召回 sidecar 未打包")?,
        )
        .map_err(|_| "任务召回清单无效")?;
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
        let fingerprint = fingerprint(&settings);
        let config = json!({"enabled":settings.enabled,"fingerprint":fingerprint,
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
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(&bytes)
            .await
            .map_err(|_| "任务召回配置传递失败")?;
        let output = child.stdout.take().unwrap();
        let mut reader = BufReader::new(tokio::io::AsyncReadExt::take(output, 65536));
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(15), reader.read_line(&mut line))
            .await
            .map_err(|_| "任务召回启动超时")?
            .map_err(|_| "任务召回握手失败")?;
        let hello: Handshake = serde_json::from_str(&line).map_err(|_| "任务召回握手无效")?;
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
            let _ = child.wait().await;
        }
        let health = process.request("/health", None).await?;
        if health["fingerprint"] != process.fingerprint {
            return Err("任务召回正在切换模型配置，请稍后重试".into());
        }
        Ok(process)
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
