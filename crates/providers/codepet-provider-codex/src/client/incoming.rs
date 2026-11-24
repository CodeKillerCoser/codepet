use super::*;

pub(super) fn handle_message(
    inner: &SessionInner,
    message: Value,
) -> Result<(), CodexAppServerError> {
    let object = message.as_object().ok_or_else(|| {
        CodexAppServerError::Protocol("App Server message must be an object".to_string())
    })?;
    if object
        .get("jsonrpc")
        .is_some_and(|version| version.as_str() != Some("2.0"))
    {
        return Err(CodexAppServerError::Protocol(
            "App Server jsonrpc metadata must be exactly 2.0 when present".to_string(),
        ));
    }
    if let Some(trace) = object.get("trace") {
        if !trace.is_null() {
            let trace = trace.as_object().ok_or_else(|| {
                CodexAppServerError::Protocol(
                    "App Server trace metadata must be an object or null".to_string(),
                )
            })?;
            for field in ["traceparent", "tracestate"] {
                if trace
                    .get(field)
                    .is_some_and(|value| !value.is_null() && !value.is_string())
                {
                    return Err(CodexAppServerError::Protocol(format!(
                        "App Server trace {field} must be a string or null"
                    )));
                }
            }
        }
    }
    if object
        .get("emittedAtMs")
        .is_some_and(|emitted_at| emitted_at.as_i64().is_none())
    {
        return Err(CodexAppServerError::Protocol(
            "App Server emittedAtMs metadata must be an integer".to_string(),
        ));
    }
    if object.contains_key("method") {
        let incoming = parse_incoming(message, &inner.generation)?;
        if let CodexIncoming::UnsupportedServerRequest { request_id, method } = &incoming {
            let response = json!({
                "id": request_id,
                "error": {
                    "code": -32601,
                    "message": format!(
                        "Code Pet does not support Codex server request {method}"
                    ),
                    "data": {
                        "method": method,
                        "reason": "unsupported_client_capability"
                    }
                }
            });
            inner.write(response)?;
        }
        if let CodexIncoming::Notification(CodexNotification::ThreadStarted { snapshot }) =
            &incoming
        {
            if snapshot.thread.ephemeral {
                inner
                    .ephemeral_threads
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(snapshot.thread.id.clone());
            }
        }
        if let Some(thread_id) = incoming_thread_id(&incoming) {
            inner.mark_thread_loaded(thread_id);
        }
        inner.broadcast(Ok(incoming));
        return Ok(());
    }
    let has_result = object.contains_key("result");
    let has_error = object.contains_key("error");
    if has_result == has_error {
        return Err(CodexAppServerError::Protocol(
            "App Server response must contain exactly one of result or error".to_string(),
        ));
    }
    let id: JsonRpcId = serde_json::from_value(
        object
            .get("id")
            .cloned()
            .ok_or_else(|| CodexAppServerError::Protocol("response is missing id".to_string()))?,
    )
    .map_err(|error| CodexAppServerError::Protocol(format!("invalid response id: {error}")))?;
    let response = if let Some(error) = object.get("error") {
        let error = error.as_object().ok_or_else(|| {
            CodexAppServerError::Protocol("App Server error must be an object".to_string())
        })?;
        let code = error.get("code").and_then(Value::as_i64).ok_or_else(|| {
            CodexAppServerError::Protocol("App Server error is missing integer code".to_string())
        })?;
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .filter(|message| !message.is_empty())
            .ok_or_else(|| {
                CodexAppServerError::Protocol(
                    "App Server error is missing non-empty message".to_string(),
                )
            })?;
        Err(CodexAppServerError::Rpc {
            code,
            message: message.to_string(),
            data: error.get("data").filter(|data| !data.is_null()).cloned(),
        })
    } else {
        Ok(object["result"].clone())
    };
    let sender = inner
        .pending
        .lock()
        .map_err(|_| CodexAppServerError::Protocol("pending map lock is poisoned".to_string()))?
        .remove(&id)
        .ok_or_else(|| {
            CodexAppServerError::Protocol(format!("response has no matching request id {id}"))
        })?;
    let _ = sender.send(response);
    Ok(())
}

pub(super) fn incoming_thread_id(incoming: &CodexIncoming) -> Option<&str> {
    match incoming {
        CodexIncoming::Notification(CodexNotification::ThreadStarted { snapshot }) => {
            Some(&snapshot.thread.id)
        }
        CodexIncoming::Notification(
            CodexNotification::ThreadNameUpdated { thread_id, .. }
            | CodexNotification::ThreadStatusChanged { thread_id, .. }
            | CodexNotification::TurnStarted { thread_id, .. }
            | CodexNotification::TurnError { thread_id, .. }
            | CodexNotification::TurnCompleted { thread_id, .. }
            | CodexNotification::ItemUpserted { thread_id, .. }
            | CodexNotification::OutputDelta { thread_id, .. }
            | CodexNotification::ServerRequestResolved { thread_id, .. },
        ) => Some(thread_id),
        CodexIncoming::ApprovalRequested(approval) => Some(&approval.thread_id),
        CodexIncoming::Notification(
            CodexNotification::ProjectChanged { .. } | CodexNotification::Unknown { .. },
        )
        | CodexIncoming::UnsupportedServerRequest { .. } => None,
    }
}

pub(super) fn parse_incoming(
    message: Value,
    session_generation: &str,
) -> Result<CodexIncoming, CodexAppServerError> {
    let method = required_string(&message, "method")?;
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    if let Some(id_value) = message.get("id") {
        let request_id: JsonRpcId = serde_json::from_value(id_value.clone()).map_err(|error| {
            CodexAppServerError::Protocol(format!("invalid server request id: {error}"))
        })?;
        return parse_server_request(request_id, &method, &params, session_generation);
    }
    parse_notification(&method, params, session_generation).map(CodexIncoming::Notification)
}

pub(super) fn parse_server_request(
    request_id: JsonRpcId,
    method: &str,
    params: &Value,
    session_generation: &str,
) -> Result<CodexIncoming, CodexAppServerError> {
    let unsupported = || CodexIncoming::UnsupportedServerRequest {
        request_id: request_id.clone(),
        method: method.to_string(),
    };
    let (
        kind,
        thread_id,
        turn_id,
        item_id,
        requested_at_ms,
        available_decisions,
        title,
        description,
    ) = match method {
        "item/commandExecution/requestApproval" => {
            if has_any_key(
                params,
                &[
                    "additionalPermissions",
                    "networkApprovalContext",
                    "proposedExecpolicyAmendment",
                    "proposedNetworkPolicyAmendments",
                ],
            ) || params
                .as_object()
                .is_some_and(|object| object.get("kind").is_some_and(Value::is_null))
            {
                return Ok(unsupported());
            }
            let Ok(params) = serde_json::from_value::<CommandApprovalParams>(params.clone()) else {
                return Ok(unsupported());
            };
            if params.has_unsupported_semantics()
                || params.thread_id.is_empty()
                || params.turn_id.is_empty()
                || params.item_id.is_empty()
            {
                return Ok(unsupported());
            }
            let Ok(requested_at_ms) = u64::try_from(params.started_at_ms) else {
                return Ok(unsupported());
            };
            let available_decisions = params.binary_decisions();
            let description = params.command.clone().or(params.reason.clone());
            (
                CodexApprovalKind::CommandExecution,
                params.thread_id,
                params.turn_id,
                params.item_id,
                requested_at_ms,
                available_decisions,
                "Run command".to_string(),
                description,
            )
        }
        "item/fileChange/requestApproval" => {
            if has_any_key(params, &["grantRoot"]) {
                return Ok(unsupported());
            }
            let Ok(params) = serde_json::from_value::<FileApprovalParams>(params.clone()) else {
                return Ok(unsupported());
            };
            if params.grant_root.is_some()
                || params.thread_id.is_empty()
                || params.turn_id.is_empty()
                || params.item_id.is_empty()
            {
                return Ok(unsupported());
            }
            let Ok(requested_at_ms) = u64::try_from(params.started_at_ms) else {
                return Ok(unsupported());
            };
            (
                CodexApprovalKind::FileChange,
                params.thread_id,
                params.turn_id,
                params.item_id,
                requested_at_ms,
                vec!["accept".to_string(), "decline".to_string()],
                "Apply file changes".to_string(),
                params.reason,
            )
        }
        _ => return Ok(unsupported()),
    };
    Ok(CodexIncoming::ApprovalRequested(CodexApprovalRequest {
        request_id,
        session_generation: session_generation.to_string(),
        kind,
        thread_id,
        turn_id,
        item_id,
        title,
        description,
        requested_at_ms,
        available_decisions,
    }))
}

pub(super) fn has_any_key(value: &Value, keys: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|object| keys.iter().any(|key| object.contains_key(*key)))
}

pub(super) fn parse_notification(
    method: &str,
    params: Value,
    session_generation: &str,
) -> Result<CodexNotification, CodexAppServerError> {
    match method {
        "project/changed" => {
            let change_type = match required_string(&params, "changeType")?.as_str() {
                "created" => crate::protocol::CodexProjectChangeType::Created,
                "updated" => crate::protocol::CodexProjectChangeType::Updated,
                "deleted" => crate::protocol::CodexProjectChangeType::Deleted,
                value => {
                    return Err(CodexAppServerError::Protocol(format!(
                        "unknown project change type: {value}"
                    )))
                }
            };
            Ok(CodexNotification::ProjectChanged {
                project_id: required_string(&params, "projectId")?,
                change_type,
            })
        }
        "thread/started" => {
            let thread = deserialize_field(&params, "thread")?;
            Ok(CodexNotification::ThreadStarted {
                snapshot: CodexConversationSnapshot::from_thread(thread),
            })
        }
        "thread/name/updated" => Ok(CodexNotification::ThreadNameUpdated {
            thread_id: required_string(&params, "threadId")?,
            thread_name: params
                .get("threadName")
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()
                .map_err(|error| {
                    CodexAppServerError::Protocol(format!(
                        "invalid notification field threadName: {error}"
                    ))
                })?
                .flatten(),
        }),
        "thread/status/changed" => Ok(CodexNotification::ThreadStatusChanged {
            thread_id: required_string(&params, "threadId")?,
            status: deserialize_field(&params, "status")?,
        }),
        "turn/started" => Ok(CodexNotification::TurnStarted {
            thread_id: required_string(&params, "threadId")?,
            turn: deserialize_field(&params, "turn")?,
        }),
        "error" => Ok(CodexNotification::TurnError {
            thread_id: required_string(&params, "threadId")?,
            turn_id: required_string(&params, "turnId")?,
            error: deserialize_field(&params, "error")?,
        }),
        "turn/completed" => Ok(CodexNotification::TurnCompleted {
            thread_id: required_string(&params, "threadId")?,
            turn: deserialize_field(&params, "turn")?,
        }),
        "item/started" | "item/completed" => {
            let item = deserialize_field(&params, "item")?;
            Ok(CodexNotification::ItemUpserted {
                thread_id: required_string(&params, "threadId")?,
                turn_id: required_string(&params, "turnId")?,
                turn_status: if method == "item/started" {
                    CodexTurnStatus::InProgress
                } else {
                    CodexTurnStatus::Completed
                },
                item,
            })
        }
        "item/agentMessage/delta"
        | "item/plan/delta"
        | "item/commandExecution/outputDelta"
        | "item/fileChange/outputDelta"
        | "item/reasoning/summaryTextDelta" => {
            let item_id = required_string(&params, "itemId")?;
            let (content_id, kind) = match method {
                "item/agentMessage/delta" | "item/plan/delta" => {
                    (text_content_id(&item_id), CodexContentKind::Text)
                }
                "item/reasoning/summaryTextDelta" => {
                    let summary_index = required_usize(&params, "summaryIndex")?;
                    (
                        reasoning_summary_content_id(&item_id, summary_index),
                        CodexContentKind::ReasoningSummary,
                    )
                }
                _ => (output_content_id(&item_id), CodexContentKind::Output),
            };
            Ok(CodexNotification::OutputDelta {
                native_method: method.to_string(),
                thread_id: required_string(&params, "threadId")?,
                turn_id: required_string(&params, "turnId")?,
                item_id,
                content_id,
                kind,
                delta: required_string(&params, "delta")?,
            })
        }
        "serverRequest/resolved" => Ok(CodexNotification::ServerRequestResolved {
            request_id: deserialize_field(&params, "requestId")?,
            thread_id: required_string(&params, "threadId")?,
            session_generation: session_generation.to_string(),
        }),
        _ => Ok(CodexNotification::Unknown {
            method: method.to_string(),
        }),
    }
}

pub(super) fn required_string(value: &Value, key: &str) -> Result<String, CodexAppServerError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| CodexAppServerError::Protocol(format!("message is missing {key}")))
}

pub(super) fn required_usize(value: &Value, key: &str) -> Result<usize, CodexAppServerError> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| CodexAppServerError::Protocol(format!("message is missing {key}")))
}

fn deserialize_field<T: DeserializeOwned>(
    value: &Value,
    key: &str,
) -> Result<T, CodexAppServerError> {
    serde_json::from_value(
        value
            .get(key)
            .cloned()
            .ok_or_else(|| CodexAppServerError::Protocol(format!("message is missing {key}")))?,
    )
    .map_err(|error| CodexAppServerError::Protocol(format!("invalid {key}: {error}")))
}
