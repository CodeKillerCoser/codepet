use serde_json::{json, Value};
use std::fmt;

pub const IPC_ROUTER_VERSION: u64 = 1;
pub const INITIALIZE_VERSION: u64 = 0;
pub const CLIENT_STATUS_VERSION: u64 = 0;
pub const THREAD_STREAM_STATE_VERSION: u64 = 11;
pub const INITIAL_CLIENT_ID: &str = "initializing-client";
pub const LOCAL_HOST_ID: &str = "local";

pub const METHOD_INITIALIZE: &str = "initialize";
pub const METHOD_THREAD_OWNER_DISCOVERY: &str = "thread-owner-discovery";
pub const METHOD_THREAD_STREAM_STATE_CHANGED: &str = "thread-stream-state-changed";
pub const METHOD_THREAD_STREAM_FOLLOWING_CHANGED: &str = "thread-stream-following-changed";
pub const METHOD_THREAD_STREAM_FOLLOWING_STATUS_REQUESTED: &str =
    "thread-stream-following-status-requested";
pub const METHOD_THREAD_FOLLOWER_LOAD_COMPLETE_HISTORY: &str =
    "thread-follower-load-complete-history";
pub const METHOD_THREAD_FOLLOWER_START_TURN: &str = "thread-follower-start-turn";
pub const METHOD_THREAD_FOLLOWER_STEER_TURN: &str = "thread-follower-steer-turn";
pub const METHOD_THREAD_FOLLOWER_INTERRUPT_TURN: &str = "thread-follower-interrupt-turn";
pub const METHOD_THREAD_FOLLOWER_COMMAND_APPROVAL_DECISION: &str =
    "thread-follower-command-approval-decision";
pub const METHOD_THREAD_FOLLOWER_FILE_APPROVAL_DECISION: &str =
    "thread-follower-file-approval-decision";
pub const METHOD_CLIENT_STATUS_CHANGED: &str = "client-status-changed";

pub const THREAD_FOLLOWER_START_TURN_VERSION: u64 = 2;
pub const THREAD_FOLLOWER_STEER_TURN_VERSION: u64 = 1;
pub const THREAD_FOLLOWER_INTERRUPT_TURN_VERSION: u64 = 4;
pub const THREAD_FOLLOWER_APPROVAL_DECISION_VERSION: u64 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesktopIpcError {
    SocketPath(String),
    UnsafeSocket(String),
    Io(String),
    Protocol(String),
    Remote(String),
    Timeout(String),
    Stale(String),
    OutcomeUnknown(String),
    PartialFailure(String),
    Disconnected(String),
    Shutdown,
    #[cfg_attr(unix, allow(dead_code))]
    Unsupported(String),
}

impl fmt::Display for DesktopIpcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SocketPath(message) => {
                write!(formatter, "Codex Desktop IPC socket is unavailable: {message}")
            }
            Self::UnsafeSocket(message) => {
                write!(formatter, "Codex Desktop IPC socket failed safety checks: {message}")
            }
            Self::Io(message) => write!(formatter, "Codex Desktop IPC I/O error: {message}"),
            Self::Protocol(message) => {
                write!(formatter, "Codex Desktop IPC protocol error: {message}")
            }
            Self::Remote(message) => write!(formatter, "Codex Desktop IPC request failed: {message}"),
            Self::Timeout(message) => write!(formatter, "Codex Desktop IPC request timed out: {message}"),
            Self::Stale(message) => write!(formatter, "Codex Desktop IPC action is stale: {message}"),
            Self::OutcomeUnknown(message) => {
                write!(formatter, "Codex Desktop IPC action outcome is unknown: {message}")
            }
            Self::PartialFailure(message) => {
                write!(formatter, "Codex Desktop IPC action partially failed: {message}")
            }
            Self::Disconnected(message) => {
                write!(formatter, "Codex Desktop IPC disconnected: {message}")
            }
            Self::Shutdown => write!(formatter, "Codex Desktop IPC client is shut down"),
            Self::Unsupported(message) => write!(formatter, "Codex Desktop IPC is unsupported: {message}"),
        }
    }
}

impl std::error::Error for DesktopIpcError {}

#[derive(Clone, Debug)]
pub struct IpcResponse {
    pub request_id: String,
    pub result_type: String,
    pub method: Option<String>,
    pub handled_by_client_id: Option<String>,
    pub result: Option<Value>,
    pub error: Option<String>,
}

impl IpcResponse {
    pub fn from_value(value: &Value) -> Result<Self, DesktopIpcError> {
        if value.get("type").and_then(Value::as_str) != Some("response") {
            return Err(DesktopIpcError::Protocol(
                "response envelope has an unexpected type".to_string(),
            ));
        }
        let request_id = required_string(value, "requestId")?;
        let result_type = required_string(value, "resultType")?;
        if !matches!(result_type.as_str(), "success" | "error") {
            return Err(DesktopIpcError::Protocol(format!(
                "response has unknown resultType {result_type}"
            )));
        }
        let error = if result_type == "error" {
            Some(required_string(value, "error")?)
        } else {
            optional_string(value, "error")
        };
        Ok(Self {
            request_id,
            result_type,
            method: optional_string(value, "method"),
            handled_by_client_id: optional_string(value, "handledByClientId"),
            result: value.get("result").cloned(),
            error,
        })
    }

    pub fn success_result(&self, expected_method: &str) -> Result<&Value, DesktopIpcError> {
        if self.result_type != "success" {
            let message = self
                .error
                .clone()
                .unwrap_or_else(|| format!("{expected_method} returned an unknown error"));
            if matches!(
                message.as_str(),
                "request-version-mismatch" | "no-handler-for-request"
            ) {
                return Err(DesktopIpcError::Protocol(message));
            }
            return Err(DesktopIpcError::Remote(message));
        }
        let method = self.method.as_deref().ok_or_else(|| {
            DesktopIpcError::Protocol(format!(
                "{expected_method} response is missing method"
            ))
        })?;
        if method != expected_method {
            return Err(DesktopIpcError::Protocol(format!(
                "response method mismatch: expected {expected_method}, received {method}"
            )));
        }
        self.result.as_ref().ok_or_else(|| {
            DesktopIpcError::Protocol(format!("{expected_method} response is missing result"))
        })
    }

    pub fn ensure_handled_by(&self, expected_client_id: &str) -> Result<(), DesktopIpcError> {
        match self.handled_by_client_id.as_deref() {
            Some(client_id) if client_id == expected_client_id => Ok(()),
            Some(client_id) => Err(DesktopIpcError::Protocol(format!(
                "response handler mismatch: expected {expected_client_id}, received {client_id}"
            ))),
            None => Err(DesktopIpcError::Protocol(
                "response is missing handledByClientId".to_string(),
            )),
        }
    }
}

pub fn request_envelope(
    request_id: &str,
    source_client_id: &str,
    version: u64,
    method: &str,
    params: Value,
    target_client_id: Option<&str>,
    timeout_ms: u64,
) -> Value {
    let mut envelope = json!({
        "type": "request",
        "requestId": request_id,
        "sourceClientId": source_client_id,
        "version": version,
        "method": method,
        "params": params,
        "timeoutMs": timeout_ms,
    });
    if let Some(target_client_id) = target_client_id {
        envelope["targetClientId"] = Value::String(target_client_id.to_string());
    }
    envelope
}

pub fn broadcast_envelope(
    source_client_id: &str,
    version: u64,
    method: &str,
    params: Value,
    target_client_ids: Option<&[String]>,
) -> Value {
    let mut envelope = json!({
        "type": "broadcast",
        "sourceClientId": source_client_id,
        "version": version,
        "method": method,
        "params": params,
    });
    if let Some(target_client_ids) = target_client_ids {
        envelope["targetClientIds"] = serde_json::to_value(target_client_ids)
            .unwrap_or_else(|_| Value::Array(Vec::new()));
    }
    envelope
}

pub fn message_targets_client(message: &Value, client_id: &str) -> bool {
    match message.get("type").and_then(Value::as_str) {
        Some("broadcast") => match message.get("targetClientIds") {
            None => true,
            Some(Value::Array(targets)) => {
                targets.iter().all(Value::is_string)
                    && targets
                        .iter()
                        .any(|target| target.as_str() == Some(client_id))
            }
            Some(_) => false,
        },
        Some("request") => match message.get("targetClientId") {
            None => true,
            Some(Value::String(target)) => target == client_id,
            Some(_) => false,
        },
        Some("response") => match message.get("targetClientId") {
            None => true,
            Some(Value::String(target)) => target == client_id,
            Some(_) => false,
        },
        Some("client-discovery-request") => true,
        _ => false,
    }
}

pub fn method(message: &Value) -> Option<&str> {
    message.get("method").and_then(Value::as_str)
}

pub fn source_client_id(message: &Value) -> Option<&str> {
    message.get("sourceClientId").and_then(Value::as_str)
}

pub fn version(message: &Value) -> Option<u64> {
    message.get("version").and_then(Value::as_u64)
}

pub fn required_string(value: &Value, field: &str) -> Result<String, DesktopIpcError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| DesktopIpcError::Protocol(format!("message is missing {field}")))
}

fn optional_string(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(Value::as_str).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targeted_messages_are_isolated_from_bystanders() {
        let targeted = json!({
            "type": "broadcast",
            "targetClientIds": ["client-a"],
        });
        assert!(message_targets_client(&targeted, "client-a"));
        assert!(!message_targets_client(&targeted, "client-b"));

        let untargeted = json!({ "type": "broadcast" });
        assert!(message_targets_client(&untargeted, "client-b"));

        let request = json!({ "type": "request", "targetClientId": "client-b" });
        assert!(!message_targets_client(&request, "client-a"));
        assert!(message_targets_client(&request, "client-b"));

        let malformed_broadcast = json!({
            "type": "broadcast",
            "targetClientIds": "client-a",
        });
        assert!(!message_targets_client(&malformed_broadcast, "client-a"));

        let partially_malformed_broadcast = json!({
            "type": "broadcast",
            "targetClientIds": ["client-a", 42],
        });
        assert!(!message_targets_client(
            &partially_malformed_broadcast,
            "client-a"
        ));

        let malformed_request = json!({
            "type": "request",
            "targetClientId": ["client-a"],
        });
        assert!(!message_targets_client(&malformed_request, "client-a"));

        let response = json!({
            "type": "response",
            "targetClientId": "client-b",
        });
        assert!(!message_targets_client(&response, "client-a"));
        assert!(message_targets_client(&response, "client-b"));
    }

    #[test]
    fn response_envelope_rejects_unknown_result_types_and_missing_error_codes() {
        let unknown = json!({
            "type": "response",
            "requestId": "request-one",
            "resultType": "partial",
        });
        assert!(matches!(
            IpcResponse::from_value(&unknown),
            Err(DesktopIpcError::Protocol(message)) if message.contains("unknown resultType")
        ));

        let missing_error = json!({
            "type": "response",
            "requestId": "request-one",
            "resultType": "error",
        });
        assert!(matches!(
            IpcResponse::from_value(&missing_error),
            Err(DesktopIpcError::Protocol(message)) if message.contains("missing error")
        ));

        let valid_error = json!({
            "type": "response",
            "requestId": "request-one",
            "resultType": "error",
            "error": "no-client-found",
        });
        let response = IpcResponse::from_value(&valid_error).unwrap();
        assert!(matches!(
            response.success_result("thread-owner-discovery"),
            Err(DesktopIpcError::Remote(message)) if message == "no-client-found"
        ));
    }

    #[test]
    fn targeted_success_response_must_name_the_expected_handler() {
        let valid = json!({
            "type": "response",
            "requestId": "request-one",
            "resultType": "success",
            "method": "thread-follower-load-complete-history",
            "handledByClientId": "owner-one",
            "result": { "revision": 7 },
        });
        let response = IpcResponse::from_value(&valid).unwrap();
        assert!(response.ensure_handled_by("owner-one").is_ok());
        assert!(matches!(
            response.ensure_handled_by("owner-two"),
            Err(DesktopIpcError::Protocol(message)) if message.contains("handler mismatch")
        ));

        let missing_handler = json!({
            "type": "response",
            "requestId": "request-two",
            "resultType": "success",
            "method": "thread-follower-load-complete-history",
            "result": { "revision": 7 },
        });
        let response = IpcResponse::from_value(&missing_handler).unwrap();
        assert!(matches!(
            response.ensure_handled_by("owner-one"),
            Err(DesktopIpcError::Protocol(message)) if message.contains("missing handledByClientId")
        ));
    }
}
