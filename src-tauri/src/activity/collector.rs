use crate::agents::AgentId;
use crate::events::{frontend_event, normalize_hook_payload, PetEvent, TaskStatus};
use crate::settings::{configured_app_data_dir, load_app_settings, AppSettings};
use crate::state::{ApprovalDecision, SharedState, COLLECTOR_PORT};
use crate::title_resolver::enrich_event_title;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tauri::{AppHandle, Emitter};

#[derive(Clone)]
struct CollectorState {
    app_state: SharedState,
    app_handle: AppHandle,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IncomingHook {
    agent: String,
    payload: Value,
    spooled_at: Option<DateTime<Utc>>,
}

pub async fn run_collector(
    app_state: SharedState,
    app_handle: AppHandle,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let collector_state = CollectorState {
        app_state,
        app_handle,
    };
    let app = Router::new()
        .route("/health", get(|| async { Json(json!({ "ok": true })) }))
        .route("/events", get(recent_events))
        .route("/hook", post(receive_hook))
        .route("/approvals/:event_id/wait", get(wait_for_approval))
        .with_state(collector_state);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", COLLECTOR_PORT)).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

pub fn replay_default_spooled_events(app_state: &SharedState) -> Result<usize, std::io::Error> {
    let spool_path = load_app_settings()
        .map(|settings| spool_path_for_settings(&settings))
        .unwrap_or_else(|_| legacy_default_spool_path());
    replay_spooled_events(app_state, &spool_path)
}

pub fn replay_spooled_events(app_state: &SharedState, spool_path: &Path) -> Result<usize, std::io::Error> {
    if !spool_path.exists() {
        return Ok(0);
    }

    let text = fs::read_to_string(spool_path)?;
    let mut imported = 0;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(incoming) = serde_json::from_str::<IncomingHook>(line) else {
            continue;
        };
        let Ok(agent) = AgentId::from_str(&incoming.agent) else {
            continue;
        };
        if !app_state.agent_enabled(agent) {
            continue;
        }
        let Ok(event) = normalize_hook_payload(agent, incoming.payload) else {
            continue;
        };
        let mut event = enrich_event_title(event);
        if let Some(spooled_at) = incoming.spooled_at {
            event.created_at = spooled_at;
        }
        app_state.push_event(event);
        imported += 1;
    }
    fs::remove_file(spool_path)?;
    Ok(imported)
}

pub fn spool_path_for_settings(settings: &AppSettings) -> PathBuf {
    if settings
        .data
        .data_directory
        .as_deref()
        .is_some_and(|path| !path.trim().is_empty())
    {
        return configured_app_data_dir(settings).join("spool").join("events.jsonl");
    }
    legacy_default_spool_path()
}

fn legacy_default_spool_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".code-pet")
        .join("spool")
        .join("events.jsonl")
}

async fn recent_events(State(state): State<CollectorState>) -> impl IntoResponse {
    ([("Access-Control-Allow-Origin", "*")], Json(state.app_state.recent_events()))
}

async fn receive_hook(
    State(state): State<CollectorState>,
    Json(incoming): Json<IncomingHook>,
) -> Result<Json<Option<PetEvent>>, (StatusCode, String)> {
    let agent = AgentId::from_str(&incoming.agent)
        .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
    if !state.app_state.agent_enabled(agent) {
        return Ok(Json(None));
    }
    let event = normalize_hook_payload(agent, incoming.payload)
        .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
    let event = enrich_event_title(event);
    state.app_state.push_event(event.clone());
    let frontend_event = frontend_event(&event);
    let _ = state.app_handle.emit("pet-event", &frontend_event);
    refresh_token_usage_if_needed(&state.app_handle, event.clone());
    watch_claude_transcript_if_needed(&state, &event);
    Ok(Json(Some(frontend_event)))
}

fn refresh_token_usage_if_needed(app_handle: &AppHandle, event: PetEvent) {
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Ok(Some(summary)) = crate::token_usage::refresh_usage_for_event(&event) {
            let _ = app_handle.emit("token-usage-updated", summary);
        }
    });
}

fn watch_claude_transcript_if_needed(state: &CollectorState, event: &PetEvent) {
    if event.provider != AgentId::Claude || !matches!(event.status, TaskStatus::Thinking | TaskStatus::Running) {
        return;
    }
    let Some(transcript_path) = crate::claude_transcript::transcript_path_from_event(event) else {
        return;
    };

    let fallback = event.clone();
    let app_state = state.app_state.clone();
    let app_handle = state.app_handle.clone();
    tauri::async_runtime::spawn(async move {
        crate::claude_transcript::watch_claude_transcript_for_outcome(
            transcript_path,
            fallback,
            app_state,
            app_handle,
        )
        .await;
    });
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WaitApprovalQuery {
    timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WaitApprovalResponse {
    decision: Option<ApprovalDecision>,
}

async fn wait_for_approval(
    State(state): State<CollectorState>,
    AxumPath(event_id): AxumPath<String>,
    Query(query): Query<WaitApprovalQuery>,
) -> Json<WaitApprovalResponse> {
    let timeout = std::time::Duration::from_millis(query.timeout_ms.unwrap_or(590_000).min(590_000));
    let decision = state.app_state.wait_for_approval(&event_id, timeout).await;
    Json(WaitApprovalResponse { decision })
}
