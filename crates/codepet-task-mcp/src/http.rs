//! App-owned, loopback-only Streamable HTTP transport. No frontend forwarding.
mod credentials;
mod runtime;
pub use runtime::HttpRuntime;

use crate::TaskServer;
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Router,
};
use codepet_task_lineage::tasks::Ledger;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use serde_json::{json, Value};
use std::{
    io,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

pub const DEFAULT_HTTP_PORT: u16 = 47623;

#[derive(Clone, Default)]
pub struct HttpShutdown(CancellationToken);
impl HttpShutdown {
    pub fn cancel(&self) {
        self.0.cancel();
    }
}

#[derive(Clone)]
struct Access {
    authorities: Vec<String>,
    origins: Vec<String>,
    authorization: String,
}

async fn authorize(State(access): State<Arc<Access>>, request: Request, next: Next) -> Response {
    // Validate before authentication too: invalid browser Origins always get 403.
    let hosts = request
        .headers()
        .get_all(header::HOST)
        .iter()
        .collect::<Vec<_>>();
    let valid_host = hosts.len() == 1
        && hosts[0].to_str().ok().is_some_and(|host| {
            access
                .authorities
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(host))
        });
    let origins = request
        .headers()
        .get_all(header::ORIGIN)
        .iter()
        .collect::<Vec<_>>();
    let valid_origin = origins.is_empty()
        || (origins.len() == 1
            && origins[0]
                .to_str()
                .ok()
                .is_some_and(|origin| access.origins.iter().any(|allowed| allowed == origin)));
    if !valid_host || !valid_origin {
        return (StatusCode::FORBIDDEN, "Invalid local MCP Host or Origin").into_response();
    }
    let headers = request
        .headers()
        .get_all(header::AUTHORIZATION)
        .iter()
        .collect::<Vec<_>>();
    let valid = headers.len() == 1
        && headers[0].to_str().ok().is_some_and(|actual| {
            actual.len() == access.authorization.len()
                && actual
                    .bytes()
                    .zip(access.authorization.bytes())
                    .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
                    == 0
        });
    if !valid {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer realm=\"codepet-tasks\"")],
            "Local MCP credential required",
        )
            .into_response();
    }
    next.run(request).await
}

pub struct HttpServer {
    listener: TcpListener,
    ledger: Ledger,
    access: Arc<Access>,
    shutdown: HttpShutdown,
    url: String,
    token_file: PathBuf,
}
impl HttpServer {
    /// Port zero is reserved for isolated tests/development; production uses a stable port.
    pub async fn bind(
        ledger: Ledger,
        data: &Path,
        port: u16,
        shutdown: HttpShutdown,
    ) -> io::Result<Self> {
        let token = credentials::load_or_create(data)?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
        let port = listener.local_addr()?.port();
        let authorities = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")];
        let origins = authorities
            .iter()
            .map(|host| format!("http://{host}"))
            .collect();
        Ok(Self {
            listener,
            ledger,
            shutdown,
            access: Arc::new(Access {
                authorities,
                origins,
                authorization: format!("Bearer {token}"),
            }),
            url: format!("http://127.0.0.1:{port}/mcp"),
            token_file: credentials::token_path(data),
        })
    }
    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn configuration(&self) -> Value {
        // JSON quoted strings are valid TOML basic strings for these ASCII values.
        let config = format!(
            "[mcp_servers.codepet_tasks]\nurl = {}\nhttp_headers = {{ Authorization = {} }}\n",
            json!(self.url),
            json!(self.access.authorization)
        );
        json!({"config":config,"url":self.url,
            "instructions":include_str!("../instructions.md")})
    }
    pub async fn serve(self) -> io::Result<()> {
        let ledger = self.ledger;
        let config = StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true)
            .with_allowed_hosts(self.access.authorities.clone())
            .with_allowed_origins(self.access.origins.clone())
            .enforce_origin_validation()
            .with_cancellation_token(self.shutdown.0.child_token());
        let service = StreamableHttpService::new(
            move || Ok(TaskServer::new(ledger.clone())),
            Arc::new(LocalSessionManager::default()),
            config,
        );
        let app = Router::new()
            .nest_service("/mcp", service)
            .layer(middleware::from_fn_with_state(self.access, authorize));
        axum::serve(self.listener, app)
            .with_graceful_shutdown(self.shutdown.0.cancelled_owned())
            .await
    }
}

pub fn run_cli(port: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let port = port
        .map(str::parse::<u16>)
        .transpose()?
        .unwrap_or(DEFAULT_HTTP_PORT);
    let data = codepet_paths::PathManager::discover()?.data;
    let ledger = Ledger::new(&data);
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            let shutdown = HttpShutdown::default();
            let server = HttpServer::bind(ledger, &data, port, shutdown.clone()).await?;
            // No credentials in logs/stdout. Test clients read their isolated token file.
            println!(
                "{}",
                json!({"url":server.url(),"tokenFile":server.token_file})
            );
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                shutdown.cancel();
            });
            server.serve().await
        })?;
    Ok(())
}
