mod client;
pub mod mapper;
mod provider;
pub mod protocol;

pub use client::{
    CodexAppServerSession, JsonRpcReader, JsonRpcWriter, SessionControl,
};
pub use mapper::CodexProtocolMapper;
pub use provider::CodexProviderAdapter;
pub use protocol::{
    CodexAppServerError, CodexApprovalKind, CodexApprovalRequest,
    CodexConversationSnapshot, CodexIncoming, CodexNotification, CodexThread,
    CodexThreadListRequest, CodexThreadPage, CodexThreadStartRequest, CodexThreadStatus,
    CodexTurn, CodexTurnStartRequest, CodexTurnStatus, CodexTurnSteerRequest, JsonRpcId,
    CODEX_EXTENSION_NAMESPACE, CODEX_PROVIDER_ID,
};

use crate::activity_actions::{
    collector_approval_strategy, has_session_id, is_replyable_event, ActivationStrategy,
    ActivationTarget, AgentInteractionDriver, ApprovalStrategy, ReplyStrategy,
};
use crate::app_log;
use crate::events::PetEvent;
use serde_json::Value;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexReplyAction {
    StartTurn,
    SteerTurn(String),
}

#[derive(Clone, Copy)]
pub(crate) struct CodexAppServerManager;

impl AgentInteractionDriver for CodexAppServerManager {
    fn activation_strategy(&self, event: &PetEvent) -> ActivationStrategy {
        if let Some(thread_id) = event
            .session_id
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            ActivationStrategy::Target(ActivationTarget::Url(codex_thread_deeplink(thread_id)))
        } else {
            crate::activity_actions::default_activation_strategy_for_event(event)
        }
    }

    fn reply_strategy(&self, event: &PetEvent) -> ReplyStrategy {
        if is_replyable_event(event) && has_session_id(event) {
            ReplyStrategy::CodexAppServer
        } else {
            ReplyStrategy::Unsupported
        }
    }

    fn approval_strategy(&self, event: &PetEvent) -> ApprovalStrategy {
        collector_approval_strategy(event)
    }

    fn send_reply(&self, event: &PetEvent, message: &str) -> Result<(), String> {
        send_reply(event, message)
    }
}

static REPLY_SESSION: OnceLock<Mutex<Option<CodexAppServerSession>>> = OnceLock::new();

pub fn send_reply(event: &PetEvent, message: &str) -> Result<(), String> {
    let thread_id = event
        .session_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "codex reply requires a thread id".to_string())?;
    let session = shared_reply_session().map_err(|error| error.to_string())?;
    let incoming = session.subscribe();
    let resumed = session
        .thread_resume(thread_id)
        .map_err(|error| error.to_string())?;
    let turn_id = match reply_action_for_thread(&resumed.thread)? {
        CodexReplyAction::SteerTurn(turn_id) => {
            app_log::info(
                "codex_app_server",
                &format!("reply action=turn/steer thread_id={thread_id} turn_id={turn_id}"),
            );
            session
                .turn_steer(CodexTurnSteerRequest {
                    thread_id: thread_id.to_string(),
                    expected_turn_id: turn_id,
                    message: message.to_string(),
                    client_message_id: None,
                })
                .map_err(|error| error.to_string())?
                .id
        }
        CodexReplyAction::StartTurn => {
            app_log::info(
                "codex_app_server",
                &format!("reply action=turn/start thread_id={thread_id}"),
            );
            session
                .turn_start(CodexTurnStartRequest {
                    thread_id: thread_id.to_string(),
                    message: message.to_string(),
                    ..CodexTurnStartRequest::default()
                })
                .map_err(|error| error.to_string())?
                .id
        }
    };
    wait_for_turn_completion(&incoming, thread_id, &turn_id)?;
    refresh_codex_thread_view(thread_id);
    Ok(())
}

pub fn shutdown_shared_session() -> Result<(), String> {
    let slot = REPLY_SESSION.get_or_init(|| Mutex::new(None));
    let session = slot
        .lock()
        .map_err(|_| "codex app-server session lock is poisoned".to_string())?
        .take();
    if let Some(session) = session {
        session.shutdown().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn shared_reply_session() -> Result<CodexAppServerSession, CodexAppServerError> {
    let slot = REPLY_SESSION.get_or_init(|| Mutex::new(None));
    let mut session = slot.lock().map_err(|_| {
        CodexAppServerError::Protocol("shared session lock is poisoned".to_string())
    })?;
    if session
        .as_ref()
        .is_some_and(CodexAppServerSession::is_running)
    {
        return Ok(session.as_ref().unwrap().clone());
    }
    let spawned = CodexAppServerSession::spawn()?;
    *session = Some(spawned.clone());
    Ok(spawned)
}

fn wait_for_turn_completion(
    incoming: &std::sync::mpsc::Receiver<Result<CodexIncoming, CodexAppServerError>>,
    thread_id: &str,
    turn_id: &str,
) -> Result<(), String> {
    loop {
        let message = incoming
            .recv()
            .map_err(|_| "codex app-server notification stream closed".to_string())?
            .map_err(|error| error.to_string())?;
        if let CodexIncoming::Notification(CodexNotification::TurnCompleted {
            thread_id: completed_thread_id,
            turn,
        }) = message
        {
            if completed_thread_id != thread_id || turn.id != turn_id {
                continue;
            }
            return match turn.status {
                CodexTurnStatus::Completed => Ok(()),
                status => Err(format!(
                    "codex reply turn did not complete successfully: status={status:?}"
                )),
            };
        }
    }
}

fn reply_action_for_thread(thread: &CodexThread) -> Result<CodexReplyAction, String> {
    let Some(last_turn) = thread.turns.last() else {
        return Ok(CodexReplyAction::StartTurn);
    };
    if last_turn.status == CodexTurnStatus::InProgress {
        if last_turn.id.is_empty() {
            return Err("codex active turn is missing id".to_string());
        }
        return Ok(CodexReplyAction::SteerTurn(last_turn.id.clone()));
    }
    Ok(CodexReplyAction::StartTurn)
}

pub fn reply_action_for_resume_response(resume: &Value) -> Result<CodexReplyAction, String> {
    let thread = resume
        .get("thread")
        .ok_or_else(|| "codex app-server resume response missing thread".to_string())?;
    let Some(turns) = thread.get("turns").and_then(Value::as_array) else {
        return Ok(CodexReplyAction::StartTurn);
    };
    let Some(last_turn) = turns.last() else {
        return Ok(CodexReplyAction::StartTurn);
    };
    if last_turn.get("status").and_then(Value::as_str) == Some("inProgress") {
        let turn_id = last_turn
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "codex active turn is missing id".to_string())?;
        return Ok(CodexReplyAction::SteerTurn(turn_id.to_string()));
    }
    Ok(CodexReplyAction::StartTurn)
}

fn refresh_codex_thread_view(thread_id: &str) {
    let deeplink = codex_thread_deeplink(thread_id);
    match open::that_detached(&deeplink) {
        Ok(()) => app_log::info(
            "codex_app_server",
            &format!("opened Codex thread deeplink thread_id={thread_id}"),
        ),
        Err(error) => app_log::error(
            "codex_app_server",
            &format!("failed to open Codex thread deeplink thread_id={thread_id} error={error}"),
        ),
    }
}

pub(crate) fn codex_thread_deeplink(thread_id: &str) -> String {
    let escaped: String = url::form_urlencoded::byte_serialize(thread_id.as_bytes()).collect();
    format!("codex://threads/{escaped}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity_actions::{AgentInteractionDriver, ApprovalStrategy, ReplyStrategy};
    use crate::agents::AgentId;
    use crate::events::{PetEvent, PetEventKind, TaskStatus};
    use chrono::Utc;
    use serde_json::json;

    fn codex_event(status: TaskStatus, session_id: Option<&str>) -> PetEvent {
        PetEvent {
            id: "event-1".to_string(),
            provider: AgentId::Codex,
            kind: PetEventKind::TaskUpdated,
            status,
            title: "task".to_string(),
            message: "done".to_string(),
            session_id: session_id.map(str::to_string),
            cwd: Some("/tmp/project".to_string()),
            tool_name: None,
            should_ring: false,
            created_at: Utc::now(),
            raw: json!({}),
            source: None,
        }
    }

    #[test]
    fn chooses_steer_for_in_progress_turn() {
        let resume = json!({
            "thread": {
                "turns": [
                    { "id": "turn-old", "status": "completed" },
                    { "id": "turn-active", "status": "inProgress" }
                ]
            }
        });
        assert_eq!(
            reply_action_for_resume_response(&resume).unwrap(),
            CodexReplyAction::SteerTurn("turn-active".to_string())
        );
    }

    #[test]
    fn app_server_manager_preserves_reply_approval_and_deeplink_behavior() {
        let manager = CodexAppServerManager;
        assert_eq!(
            manager.reply_strategy(&codex_event(TaskStatus::Done, Some("thread-1"))),
            ReplyStrategy::CodexAppServer
        );
        assert_eq!(
            manager.approval_strategy(&codex_event(TaskStatus::WaitingApproval, Some("thread-1"))),
            ApprovalStrategy::CollectorWait
        );
        assert_eq!(
            codex_thread_deeplink("thread/with space"),
            "codex://threads/thread%2Fwith+space"
        );
    }

}
