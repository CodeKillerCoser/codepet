//! Authenticated, loopback-only model forwarding shared by the host and task MCP.
pub mod settings;
pub mod usage;
use axum::{
    extract::{DefaultBodyLimit, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::{
    net::TcpListener,
    sync::{watch, Semaphore},
};

struct Access {
    token: String,
    authority: String,
    client: reqwest::Client,
    settings: settings::Settings,
    data: PathBuf,
    model_slot: Semaphore,
    stop: watch::Receiver<bool>,
}
pub struct ModelGateway {
    pub endpoint: String,
    pub token: String,
    stop: watch::Sender<bool>,
}
impl Drop for ModelGateway {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}
impl ModelGateway {
    pub async fn start(data: &Path, settings: settings::Settings) -> Result<Self, String> {
        settings::validate(&settings)?;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|e| e.to_string())?;
        let authority = listener.local_addr().unwrap().to_string();
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let (stop, mut rx) = watch::channel(false);
        let access = Arc::new(Access {
            token: token.clone(),
            authority: authority.clone(),
            data: data.to_owned(),
            settings,
            // Explicitly bypass HTTP(S)_PROXY/ALL_PROXY, including upstream requests.
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(600))
                .build()
                .map_err(|e| e.to_string())?,
            model_slot: Semaphore::new(1),
            stop: rx.clone(),
        });
        let app = Router::new()
            .route("/v1/chat/completions", post(chat))
            .route("/v1/embeddings", post(embeddings))
            .layer(DefaultBodyLimit::max(4 * 1024 * 1024))
            .route_layer(middleware::from_fn_with_state(access.clone(), authorize))
            .with_state(access);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = rx.changed().await;
                })
                .await;
        });
        Ok(Self {
            endpoint: format!("http://{authority}/v1"),
            token,
            stop,
        })
    }
}
async fn authorize(State(state): State<Arc<Access>>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    if headers.get_all(header::HOST).iter().count() != 1
        || headers.get(header::HOST).and_then(|v| v.to_str().ok()) != Some(&state.authority)
        || headers.contains_key(header::ORIGIN)
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let expected = format!("Bearer {}", state.token);
    let valid = headers.get_all(header::AUTHORIZATION).iter().count() == 1
        && headers
            .get(header::AUTHORIZATION)
            .is_some_and(|v| bool::from(v.as_bytes().ct_eq(expected.as_bytes())));
    if !valid {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut stop = state.stop.clone();
    tokio::select! {
        response = next.run(request) => response,
        _ = stop.changed() => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}
async fn chat(State(state): State<Arc<Access>>, Json(body): Json<Value>) -> Response {
    forward(state, body, false).await
}
async fn embeddings(State(state): State<Arc<Access>>, Json(body): Json<Value>) -> Response {
    forward(state, body, true).await
}

async fn forward(state: Arc<Access>, mut body: Value, embedding: bool) -> Response {
    if !state.settings.enabled {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    if !body.is_object() || body["stream"].as_bool() == Some(true) {
        return (
            StatusCode::BAD_REQUEST,
            "Only JSON non-streaming model requests are supported",
        )
            .into_response();
    }
    let profile = if embedding {
        &state.settings.embedding
    } else {
        &state.settings.llm
    };
    body["model"] = json!(profile.model);
    if !embedding {
        if state.settings.plain_json && body["response_format"]["type"] == "json_object" {
            body.as_object_mut().unwrap().remove("response_format");
        }
        body.as_object_mut()
            .unwrap()
            .extend(state.settings.llm_extra_body.clone());
    }
    let _slot = state.model_slot.acquire().await;
    let purpose = if embedding { "embedding" } else { "llm" };
    let path = if embedding {
        "/embeddings"
    } else {
        "/chat/completions"
    };
    let batches = if embedding {
        body["input"]
            .as_array()
            .filter(|v| v.len() > 8)
            .map(|values| {
                values
                    .chunks(8)
                    .map(|v| {
                        let mut part = body.clone();
                        part["input"] = json!(v);
                        part
                    })
                    .collect::<Vec<_>>()
            })
    } else {
        None
    };
    let mut merged = None;
    for part in batches.unwrap_or_else(|| vec![body]) {
        let started = Instant::now();
        let result = state
            .client
            .post(format!("{}{path}", profile.endpoint.trim_end_matches('/')))
            .bearer_auth(if profile.api_key.is_empty() {
                "local"
            } else {
                &profile.api_key
            })
            .json(&part)
            .send()
            .await;
        let parsed = match result {
            Ok(response) if response.status().is_success() => read_bounded(response).await,
            _ => Err(()),
        };
        let ok = parsed.is_ok();
        let value = parsed.unwrap_or(Value::Null);
        if usage::record(
            &state.data,
            purpose,
            &profile.model,
            ok,
            &value,
            started.elapsed().as_millis(),
        )
        .is_err()
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Model usage could not be recorded",
            )
                .into_response();
        }
        if !ok {
            return (StatusCode::BAD_GATEWAY, "Model endpoint request failed").into_response();
        }
        if let Some(previous) = merged.as_mut() {
            let previous: &mut Value = previous;
            let offset = previous["data"].as_array().map_or(0, Vec::len);
            let Some(items) = value["data"].as_array() else {
                return StatusCode::BAD_GATEWAY.into_response();
            };
            let Some(target) = previous["data"].as_array_mut() else {
                return StatusCode::BAD_GATEWAY.into_response();
            };
            for item in items {
                let mut item = item.clone();
                item["index"] = json!(target.len().max(offset));
                target.push(item);
            }
        } else {
            merged = Some(value);
        }
    }
    Json(merged.unwrap_or(Value::Null)).into_response()
}
async fn read_bounded(mut response: reqwest::Response) -> Result<Value, ()> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if bytes.len() + chunk.len() > 32 * 1024 * 1024 {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| ())
}
