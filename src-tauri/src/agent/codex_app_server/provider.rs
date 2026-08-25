use super::{
    CodexAppServerError, CodexAppServerSession, CodexApprovalRequest, CodexIncoming,
    CodexNotification, CodexProtocolMapper, CodexThreadListRequest, CodexThreadStartRequest,
    CodexTurnStartRequest, CodexTurnSteerRequest,
};
use crate::runtime_gateway::generated::{
    ApprovalResolveRequest, ApprovalResolveResponse, ConversationCreateRequest,
    ConversationCreateResponse, ConversationGetRequest, ConversationGetResponse,
    ConversationListRequest, ConversationListResponse, ProtocolError, ProtocolEvent, Provider,
    ProviderStatus, ProviderStatusChangedEvent, TurnInterruptRequest, TurnInterruptResponse,
    TurnSendRequest, TurnSendResponse, PROTOCOL_VERSION,
};
use crate::runtime_gateway::{ProviderAdapter, ProviderEventSink, ProviderFuture};
use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

struct CodexProviderState {
    mapper: CodexProtocolMapper,
    pending_approvals: HashMap<String, CodexApprovalRequest>,
    status: ProviderStatus,
}

pub struct CodexProviderAdapter {
    session: Option<CodexAppServerSession>,
    startup_error: Option<CodexAppServerError>,
    state: Arc<Mutex<CodexProviderState>>,
    events: ProviderEventSink,
}

impl CodexProviderAdapter {
    pub fn spawn(events: ProviderEventSink) -> Self {
        match CodexAppServerSession::spawn() {
            Ok(session) => Self::new(session, events),
            Err(error) => {
                crate::app_log::error(
                    "codex_provider",
                    &format!("failed to start Codex provider error={error}"),
                );
                Self::unavailable(error, events)
            }
        }
    }

    pub fn new(session: CodexAppServerSession, events: ProviderEventSink) -> Self {
        let incoming = session.subscribe();
        let state = Arc::new(Mutex::new(CodexProviderState {
            mapper: CodexProtocolMapper::default(),
            pending_approvals: HashMap::new(),
            status: ProviderStatus::Ready,
        }));
        start_event_forwarder(incoming, state.clone(), events.clone());
        Self {
            session: Some(session),
            startup_error: None,
            state,
            events,
        }
    }

    fn unavailable(error: CodexAppServerError, events: ProviderEventSink) -> Self {
        Self {
            session: None,
            startup_error: Some(error),
            state: Arc::new(Mutex::new(CodexProviderState {
                mapper: CodexProtocolMapper::default(),
                pending_approvals: HashMap::new(),
                status: ProviderStatus::Unavailable,
            })),
            events,
        }
    }

    fn call_context(
        &self,
    ) -> (
        Option<CodexAppServerSession>,
        Option<CodexAppServerError>,
        Arc<Mutex<CodexProviderState>>,
        ProviderEventSink,
    ) {
        (
            self.session.clone(),
            self.startup_error.clone(),
            self.state.clone(),
            self.events.clone(),
        )
    }
}

impl ProviderAdapter for CodexProviderAdapter {
    fn provider(&self) -> Provider {
        let state = lock_state(&self.state);
        state
            .mapper
            .provider(None, state.status, Vec::new(), Vec::new())
    }

    fn conversation_list<'a>(
        &'a self,
        request: ConversationListRequest,
    ) -> ProviderFuture<'a, ConversationListResponse> {
        let limit = match request.limit.map(u32::try_from).transpose() {
            Ok(limit) => limit,
            Err(_) => {
                return Box::pin(async {
                    Err(ProtocolError {
                        code: "invalid_request".to_string(),
                        message: "conversation list limit exceeds the Codex provider range"
                            .to_string(),
                        retryable: false,
                        details: None,
                    })
                })
            }
        };
        let (session, startup_error, state, events) = self.call_context();
        Box::pin(async move {
            let page = call_session(
                session,
                startup_error,
                state.clone(),
                events,
                move |session| {
                    session.thread_list(CodexThreadListRequest {
                        cursor: request.cursor,
                        limit,
                        workspace_root: None,
                    })
                },
            )
            .await?;
            let state = lock_state(&state);
            Ok(ConversationListResponse {
                conversations: page
                    .data
                    .iter()
                    .map(|snapshot| state.mapper.conversation(snapshot))
                    .collect(),
                next_cursor: page.next_cursor,
                event_sequence: 0,
            })
        })
    }

    fn conversation_get<'a>(
        &'a self,
        request: ConversationGetRequest,
    ) -> ProviderFuture<'a, ConversationGetResponse> {
        let (session, startup_error, state, events) = self.call_context();
        Box::pin(async move {
            let snapshot = call_session(
                session,
                startup_error,
                state.clone(),
                events,
                move |session| session.thread_read(&request.conversation_id),
            )
            .await?;
            let conversation = lock_state(&state).mapper.conversation(&snapshot);
            Ok(ConversationGetResponse { conversation })
        })
    }

    fn conversation_create<'a>(
        &'a self,
        request: ConversationCreateRequest,
    ) -> ProviderFuture<'a, ConversationCreateResponse> {
        let (session, startup_error, state, events) = self.call_context();
        Box::pin(async move {
            let snapshot = call_session(
                session,
                startup_error,
                state.clone(),
                events,
                move |session| {
                    session.thread_start(CodexThreadStartRequest {
                        workspace_root: request.workspace_root,
                        permission_level: request.permission_level,
                        model: request.model,
                        reasoning_effort: request.reasoning_effort,
                    })
                },
            )
            .await?;
            let conversation = lock_state(&state).mapper.conversation(&snapshot);
            Ok(ConversationCreateResponse { conversation })
        })
    }

    fn turn_send<'a>(
        &'a self,
        request: TurnSendRequest,
    ) -> ProviderFuture<'a, TurnSendResponse> {
        if request.quick_reply_id.is_some() {
            let state = self.state.clone();
            return Box::pin(async move {
                Err(lock_state(&state).mapper.error(
                    CodexAppServerError::UnsupportedCapability {
                        capability: "turn.quick-reply".to_string(),
                        message: "Codex provider does not advertise quick replies".to_string(),
                    },
                ))
            });
        }
        let conversation_id = request.conversation_id.clone();
        let (session, startup_error, state, events) = self.call_context();
        Box::pin(async move {
            let turn = call_session(
                session,
                startup_error,
                state.clone(),
                events,
                move |session| {
                    if let Some(expected_turn_id) = request.steer_turn_id {
                        session.turn_steer(CodexTurnSteerRequest {
                            thread_id: request.conversation_id,
                            expected_turn_id,
                            message: request.message,
                            client_message_id: Some(request.client_message_id),
                        })
                    } else {
                        session.turn_start(CodexTurnStartRequest {
                            thread_id: request.conversation_id,
                            message: request.message,
                            client_message_id: Some(request.client_message_id),
                            model: None,
                            reasoning_effort: None,
                        })
                    }
                },
            )
            .await?;
            let turn = lock_state(&state)
                .mapper
                .turn(&conversation_id, &turn, now_ms());
            Ok(TurnSendResponse { turn })
        })
    }

    fn turn_interrupt<'a>(
        &'a self,
        request: TurnInterruptRequest,
    ) -> ProviderFuture<'a, TurnInterruptResponse> {
        let conversation_id = request.conversation_id.clone();
        let (session, startup_error, state, events) = self.call_context();
        Box::pin(async move {
            let turn = call_session(
                session,
                startup_error,
                state.clone(),
                events,
                move |session| {
                    session.turn_interrupt(&request.conversation_id, &request.turn_id)
                },
            )
            .await?;
            let turn = lock_state(&state)
                .mapper
                .turn(&conversation_id, &turn, now_ms());
            Ok(TurnInterruptResponse { turn })
        })
    }

    fn approval_resolve<'a>(
        &'a self,
        request: ApprovalResolveRequest,
    ) -> ProviderFuture<'a, ApprovalResolveResponse> {
        let (session, startup_error, state, events) = self.call_context();
        Box::pin(async move {
            let Some(session) = session else {
                let error = startup_error.unwrap_or(CodexAppServerError::Shutdown);
                return Err(protocol_error_for(&state, &events, error));
            };
            let approval_id = request.approval_id;
            let decision = request.decision;
            let operation_state = state.clone();
            let result = tokio::task::spawn_blocking(move || {
                let mut state = lock_state(&operation_state);
                let approval = state
                    .pending_approvals
                    .get(&approval_id)
                    .cloned()
                    .ok_or_else(|| ProviderCallError::Protocol(approval_not_found(&approval_id)))?;
                session
                    .respond_to_approval(&approval, decision)
                    .map_err(ProviderCallError::Codex)?;
                let event = state
                    .mapper
                    .approval_resolved_event(&approval_id, decision, 0, now_ms())
                    .map_err(ProviderCallError::Protocol)?;
                state.pending_approvals.remove(&approval_id);
                let ProtocolEvent::ApprovalResolved { payload, .. } = &event else {
                    unreachable!("approval mapper returned a non-approval event")
                };
                let approval = payload.approval.clone();
                Ok((event, approval))
            })
            .await
            .map_err(provider_task_error)?;
            let (event, approval) = match result {
                Ok(result) => result,
                Err(ProviderCallError::Codex(error)) => {
                    return Err(protocol_error_for(&state, &events, error))
                }
                Err(ProviderCallError::Protocol(error)) => return Err(error),
            };
            events.publish(event)?;
            Ok(ApprovalResolveResponse { approval })
        })
    }
}

enum ProviderCallError {
    Codex(CodexAppServerError),
    Protocol(ProtocolError),
}

async fn call_session<T, F>(
    session: Option<CodexAppServerSession>,
    startup_error: Option<CodexAppServerError>,
    state: Arc<Mutex<CodexProviderState>>,
    events: ProviderEventSink,
    operation: F,
) -> Result<T, ProtocolError>
where
    T: Send + 'static,
    F: FnOnce(CodexAppServerSession) -> Result<T, CodexAppServerError> + Send + 'static,
{
    let Some(session) = session else {
        let error = startup_error.unwrap_or(CodexAppServerError::Shutdown);
        return Err(protocol_error_for(&state, &events, error));
    };
    let result = tokio::task::spawn_blocking(move || operation(session))
        .await
        .map_err(provider_task_error)?;
    result.map_err(|error| protocol_error_for(&state, &events, error))
}

fn start_event_forwarder(
    incoming: Receiver<Result<CodexIncoming, CodexAppServerError>>,
    state: Arc<Mutex<CodexProviderState>>,
    events: ProviderEventSink,
) {
    thread::spawn(move || loop {
        match incoming.recv() {
            Ok(Ok(incoming)) => {
                let mapped = {
                    let mut state = lock_state(&state);
                    match &incoming {
                        CodexIncoming::ApprovalRequested(approval) => {
                            state
                                .pending_approvals
                                .insert(approval.approval_id(), approval.clone());
                        }
                        CodexIncoming::Notification(CodexNotification::ServerRequestResolved {
                            request_id,
                            ..
                        }) => {
                            state.pending_approvals.remove(&request_id.approval_id());
                        }
                        _ => {}
                    }
                    state.mapper.events(incoming, 0, now_ms())
                };
                match mapped {
                    Ok(mapped) => {
                        for event in mapped {
                            if let Err(error) = events.publish(event) {
                                crate::app_log::error(
                                    "codex_provider",
                                    &format!("failed to publish Codex event error={error:?}"),
                                );
                            }
                        }
                    }
                    Err(error) => crate::app_log::error(
                        "codex_provider",
                        &format!("failed to map Codex event error={error:?}"),
                    ),
                }
            }
            Ok(Err(error)) => {
                let protocol_error = protocol_error_for(&state, &events, error);
                crate::app_log::error(
                    "codex_provider",
                    &format!("Codex notification stream stopped error={protocol_error:?}"),
                );
                break;
            }
            Err(_) => {
                transition_provider_status(&state, &events, ProviderStatus::Unavailable);
                crate::app_log::error(
                    "codex_provider",
                    "Codex notification stream disconnected",
                );
                break;
            }
        }
    });
}

fn protocol_error_for(
    state: &Arc<Mutex<CodexProviderState>>,
    events: &ProviderEventSink,
    error: CodexAppServerError,
) -> ProtocolError {
    if let Some(status) = provider_status_for_error(&error) {
        transition_provider_status(state, events, status);
    }
    lock_state(state).mapper.error(error)
}

fn provider_status_for_error(error: &CodexAppServerError) -> Option<ProviderStatus> {
    match error {
        CodexAppServerError::Protocol(_) => Some(ProviderStatus::Error),
        CodexAppServerError::Spawn(_)
        | CodexAppServerError::Io(_)
        | CodexAppServerError::ProcessExited
        | CodexAppServerError::Shutdown => Some(ProviderStatus::Unavailable),
        CodexAppServerError::Rpc { .. }
        | CodexAppServerError::UnsupportedCapability { .. } => None,
    }
}

fn transition_provider_status(
    state: &Arc<Mutex<CodexProviderState>>,
    events: &ProviderEventSink,
    status: ProviderStatus,
) {
    let event = {
        let mut state = lock_state(state);
        let previous_status = state.status;
        if previous_status == status {
            return;
        }
        state.status = status;
        ProtocolEvent::ProviderStatusChanged {
            protocol_version: PROTOCOL_VERSION,
            event_sequence: 0,
            payload: ProviderStatusChangedEvent {
                provider: state
                    .mapper
                    .provider(None, status, Vec::new(), Vec::new()),
                previous_status: Some(previous_status),
            },
        }
    };
    if let Err(error) = events.publish(event) {
        crate::app_log::error(
            "codex_provider",
            &format!("failed to publish Codex provider status error={error:?}"),
        );
    }
}

fn approval_not_found(approval_id: &str) -> ProtocolError {
    ProtocolError {
        code: "approval_not_found".to_string(),
        message: format!("approval {approval_id} is not pending"),
        retryable: false,
        details: None,
    }
}

fn provider_task_error(error: tokio::task::JoinError) -> ProtocolError {
    ProtocolError {
        code: "provider_error".to_string(),
        message: format!("Codex provider task failed: {error}"),
        retryable: false,
        details: None,
    }
}

fn lock_state(state: &Arc<Mutex<CodexProviderState>>) -> MutexGuard<'_, CodexProviderState> {
    state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_gateway::generated::{
        ApprovalDecision, ApprovalStatus, PermissionLevel, ProtocolServer,
        ProviderListRequest, TurnTaskStatus,
    };
    use crate::runtime_gateway::{Gateway, ProviderRegistry};
    use serde_json::{json, Value};
    use std::sync::mpsc::{self, Sender};
    use std::time::Duration;

    struct FakeReader {
        receiver: mpsc::Receiver<Value>,
    }

    impl super::super::JsonRpcReader for FakeReader {
        fn read_message(&mut self) -> Result<Option<Value>, CodexAppServerError> {
            match self.receiver.recv() {
                Ok(value) => Ok(Some(value)),
                Err(_) => Ok(None),
            }
        }
    }

    struct FakeWriter {
        sender: Sender<Value>,
    }

    impl super::super::JsonRpcWriter for FakeWriter {
        fn write_message(&mut self, message: &Value) -> Result<(), CodexAppServerError> {
            self.sender
                .send(message.clone())
                .map_err(|error| CodexAppServerError::Io(error.to_string()))
        }
    }

    struct FakePeer {
        incoming: Arc<Mutex<Option<Sender<Value>>>>,
        requests: mpsc::Receiver<Value>,
    }

    impl FakePeer {
        fn send(&self, message: Value) {
            lock_sender(&self.incoming)
                .as_ref()
                .unwrap()
                .send(message)
                .unwrap();
        }

        fn next_request(&self) -> Value {
            self.requests.recv_timeout(Duration::from_secs(1)).unwrap()
        }

        fn disconnect(&self) {
            lock_sender(&self.incoming).take();
        }
    }

    fn fake_session() -> (CodexAppServerSession, FakePeer) {
        let (outgoing_sender, outgoing_receiver) = mpsc::channel::<Value>();
        let (incoming_sender, incoming_receiver) = mpsc::channel::<Value>();
        let incoming = Arc::new(Mutex::new(Some(incoming_sender)));
        let responder_incoming = incoming.clone();
        let (request_sender, request_receiver) = mpsc::channel();
        thread::spawn(move || {
            while let Ok(request) = outgoing_receiver.recv() {
                let method = request.get("method").and_then(Value::as_str);
                if method == Some("initialized") {
                    continue;
                }
                if method == Some("initialize") {
                    send_from_peer(
                        &responder_incoming,
                        json!({
                            "jsonrpc": "2.0",
                            "id": request["id"],
                            "result": { "userAgent": "codex-cli/fake" }
                        }),
                    );
                    continue;
                }
                request_sender.send(request.clone()).unwrap();
                let Some(method) = method else {
                    continue;
                };
                let result = match method {
                    "thread/list" => json!({
                        "data": [thread("listed", "idle", vec![])],
                        "nextCursor": "next-page"
                    }),
                    "thread/read" => json!({
                        "thread": thread(
                            request["params"]["threadId"].as_str().unwrap(),
                            "idle",
                            vec![]
                        ),
                        "sandbox": "workspace-write"
                    }),
                    "thread/start" => json!({
                        "thread": thread("created", "idle", vec![]),
                        "model": request["params"].get("model").cloned(),
                        "reasoningEffort": request["params"]["config"]
                            .get("model_reasoning_effort")
                            .cloned(),
                        "sandbox": request["params"]["sandbox"]
                    }),
                    "turn/start" => json!({
                        "turn": turn("started", "inProgress")
                    }),
                    "turn/steer" => json!({
                        "turnId": request["params"]["expectedTurnId"]
                    }),
                    "turn/interrupt" => json!({}),
                    method => panic!("unexpected fake Codex method {method}"),
                };
                send_from_peer(
                    &responder_incoming,
                    json!({
                        "jsonrpc": "2.0",
                        "id": request["id"],
                        "result": result
                    }),
                );
            }
        });
        let session = CodexAppServerSession::connect(
            Box::new(FakeReader {
                receiver: incoming_receiver,
            }),
            Box::new(FakeWriter {
                sender: outgoing_sender,
            }),
        )
        .unwrap();
        (
            session,
            FakePeer {
                incoming,
                requests: request_receiver,
            },
        )
    }

    #[tokio::test]
    async fn fake_session_closes_gateway_commands_events_approvals_and_status_loop() {
        let (session, peer) = fake_session();
        let registry = ProviderRegistry::default();
        let gateway = Gateway::new(registry.clone());
        let adapter = Arc::new(CodexProviderAdapter::new(session, gateway.event_sink()));
        registry.register(adapter).unwrap();

        let providers = gateway.provider_list(ProviderListRequest {}).await.unwrap();
        assert_eq!(providers.providers.len(), 1);
        assert_eq!(providers.providers[0].id, "codex");
        assert_eq!(providers.providers[0].status, ProviderStatus::Ready);

        let listed = gateway
            .conversation_list(ConversationListRequest {
                provider_id: Some("codex".to_string()),
                cursor: Some("cursor-one".to_string()),
                limit: Some(12),
            })
            .await
            .unwrap();
        assert_eq!(listed.conversations[0].id, "listed");
        assert_eq!(listed.next_cursor.as_deref(), Some("next-page"));
        let list_request = peer.next_request();
        assert_eq!(list_request["method"], "thread/list");
        assert_eq!(list_request["params"]["cursor"], "cursor-one");
        assert_eq!(list_request["params"]["limit"], 12);

        let read = gateway
            .conversation_get(ConversationGetRequest {
                provider_id: "codex".to_string(),
                conversation_id: "read-me".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(read.conversation.id, "read-me");
        assert_eq!(read.conversation.permission_level, PermissionLevel::WorkspaceWrite);
        assert_eq!(peer.next_request()["method"], "thread/read");

        let created = gateway
            .conversation_create(ConversationCreateRequest {
                provider_id: "codex".to_string(),
                title: Some("ignored native title".to_string()),
                permission_level: PermissionLevel::ReadOnly,
                model: Some("gpt-fake".to_string()),
                reasoning_effort: Some("high".to_string()),
                workspace_root: Some("/work/fake".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(created.conversation.id, "created");
        assert_eq!(created.conversation.model.as_deref(), Some("gpt-fake"));
        assert_eq!(created.conversation.workspace_root.as_deref(), Some("/work/fake"));
        let create_request = peer.next_request();
        assert_eq!(create_request["method"], "thread/start");
        assert_eq!(create_request["params"]["cwd"], "/work/fake");
        assert_eq!(create_request["params"]["sandbox"], "read-only");

        let started = gateway
            .turn_send(TurnSendRequest {
                provider_id: "codex".to_string(),
                conversation_id: "created".to_string(),
                client_message_id: "message-start".to_string(),
                message: "hello".to_string(),
                quick_reply_id: None,
                steer_turn_id: None,
            })
            .await
            .unwrap();
        assert_eq!(started.turn.id, "started");
        assert_eq!(started.turn.status, TurnTaskStatus::Running);
        let start_request = peer.next_request();
        assert_eq!(start_request["method"], "turn/start");
        assert_eq!(start_request["params"]["clientUserMessageId"], "message-start");

        let steered = gateway
            .turn_send(TurnSendRequest {
                provider_id: "codex".to_string(),
                conversation_id: "created".to_string(),
                client_message_id: "message-steer".to_string(),
                message: "change course".to_string(),
                quick_reply_id: None,
                steer_turn_id: Some("started".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(steered.turn.id, "started");
        let steer_request = peer.next_request();
        assert_eq!(steer_request["method"], "turn/steer");
        assert_eq!(steer_request["params"]["expectedTurnId"], "started");

        let interrupted = gateway
            .turn_interrupt(TurnInterruptRequest {
                provider_id: "codex".to_string(),
                conversation_id: "created".to_string(),
                turn_id: "started".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(interrupted.turn.status, TurnTaskStatus::Interrupted);
        assert_eq!(peer.next_request()["method"], "turn/interrupt");

        let mut subscription = gateway.subscribe_events(Some(0)).unwrap();
        peer.send(json!({
            "jsonrpc": "2.0",
            "method": "turn/started",
            "params": {
                "threadId": "created",
                "turn": turn("notification-turn", "inProgress")
            }
        }));
        let event = subscription.next_event().await.unwrap();
        assert!(matches!(
            event,
            ProtocolEvent::TurnUpserted { payload, .. }
                if payload.turn.id == "notification-turn"
        ));

        peer.send(json!({
            "jsonrpc": "2.0",
            "method": "item/agentMessage/delta",
            "params": {
                "threadId": "created",
                "turnId": "notification-turn",
                "itemId": "message-item",
                "delta": "hello"
            }
        }));
        let event = subscription.next_event().await.unwrap();
        assert!(matches!(
            event,
            ProtocolEvent::TurnOutputDelta { payload, .. }
                if payload.delta == "hello"
        ));

        peer.send(json!({
            "jsonrpc": "2.0",
            "id": "approval-one",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "created",
                "turnId": "notification-turn",
                "itemId": "command-item",
                "startedAtMs": 123,
                "command": "cargo test",
                "availableDecisions": ["accept", "decline"]
            }
        }));
        let event = subscription.next_event().await.unwrap();
        let ProtocolEvent::ApprovalRequested { payload, .. } = event else {
            panic!("expected approval request event")
        };
        assert_eq!(payload.approval.status, ApprovalStatus::Pending);

        let resolved = gateway
            .approval_resolve(ApprovalResolveRequest {
                provider_id: "codex".to_string(),
                approval_id: payload.approval.id,
                decision: ApprovalDecision::Approve,
            })
            .await
            .unwrap();
        assert_eq!(resolved.approval.status, ApprovalStatus::Approved);
        let approval_response = peer.next_request();
        assert_eq!(approval_response["id"], "approval-one");
        assert_eq!(approval_response["result"]["decision"], "accept");
        assert!(matches!(
            subscription.next_event().await.unwrap(),
            ProtocolEvent::ApprovalResolved { payload, .. }
                if payload.approval.status == ApprovalStatus::Approved
        ));

        peer.disconnect();
        let status_event = subscription.next_event().await.unwrap();
        assert!(matches!(
            status_event,
            ProtocolEvent::ProviderStatusChanged { payload, .. }
                if payload.previous_status == Some(ProviderStatus::Ready)
                    && payload.provider.status == ProviderStatus::Unavailable
        ));
        assert_eq!(
            gateway
                .provider_list(ProviderListRequest {})
                .await
                .unwrap()
                .providers[0]
                .status,
            ProviderStatus::Unavailable
        );
    }

    fn send_from_peer(incoming: &Arc<Mutex<Option<Sender<Value>>>>, message: Value) {
        if let Some(sender) = lock_sender(incoming).as_ref() {
            sender.send(message).unwrap();
        }
    }

    fn lock_sender(
        sender: &Arc<Mutex<Option<Sender<Value>>>>,
    ) -> MutexGuard<'_, Option<Sender<Value>>> {
        sender.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn thread(id: &str, status: &str, turns: Vec<Value>) -> Value {
        json!({
            "id": id,
            "name": format!("Thread {id}"),
            "preview": "fixture",
            "cwd": "/work/fake",
            "createdAt": 100,
            "updatedAt": 101,
            "status": { "type": status },
            "turns": turns
        })
    }

    fn turn(id: &str, status: &str) -> Value {
        json!({
            "id": id,
            "status": status,
            "startedAt": 100,
            "completedAt": null
        })
    }

}
