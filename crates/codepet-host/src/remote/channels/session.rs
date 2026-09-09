use crate::{ProviderGatewayService, RemoteAccessManager, RemoteCredential};
use codepet_gateway_sdk as gateway;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc, watch, Notify, OwnedSemaphorePermit, Semaphore};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::timeout;

const OUTBOUND_QUEUE_CAPACITY: usize = 64;
const REQUEST_QUEUE_CAPACITY: usize = 64;
const MAX_CONCURRENT_REQUESTS_PER_SESSION: usize = 8;
const CHANNEL_SEND_TIMEOUT: Duration = Duration::from_secs(15);
const OUTBOUND_ENQUEUE_TIMEOUT: Duration = Duration::from_secs(2);
pub(crate) const CONNECTION_CLOSE_TIMEOUT: Duration = Duration::from_millis(500);

/// Channel adapters carry complete Gateway JSON messages. Framing and flow
/// control belong to each adapter; authentication and dispatch stay shared.
pub(crate) enum GatewayFrame {
    Text(Vec<u8>),
    Close { code: u16, reason: String },
    KeepAlive,
    Invalid,
}

impl GatewayFrame {
    fn text(text: String) -> Self {
        Self::Text(text.into_bytes())
    }
}

pub(crate) type ChannelFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub(crate) trait GatewaySink: Send {
    fn send(&mut self, frame: GatewayFrame) -> ChannelFuture<'_, bool>;
}

pub(crate) trait GatewaySource: Send {
    fn recv(&mut self) -> ChannelFuture<'_, Option<GatewayFrame>>;
}

pub(crate) struct GatewayChannel {
    pub(crate) sink: Box<dyn GatewaySink>,
    pub(crate) source: Box<dyn GatewaySource>,
}

struct GatewayRequestDelivery {
    request: gateway::ProtocolRequest,
    trace_context: Option<gateway::TraceContext>,
    received_at: Instant,
}

#[derive(Default)]
struct SessionTraceLinks {
    by_turn: BTreeMap<String, gateway::TraceContext>,
    pending_by_conversation: BTreeMap<String, gateway::TraceContext>,
}

impl SessionTraceLinks {
    fn begin_request(
        &mut self,
        request: &gateway::ProtocolRequest,
        context: Option<&gateway::TraceContext>,
    ) {
        let (gateway::ProtocolRequest::TurnSend { params, .. }, Some(context)) = (request, context)
        else {
            return;
        };
        self.pending_by_conversation
            .insert(resource_trace_key(&params.conversation), context.clone());
    }

    fn complete_request(
        &mut self,
        request: &gateway::ProtocolRequest,
        response: &gateway::JsonRpcResponse,
        context: Option<&gateway::TraceContext>,
    ) {
        let gateway::ProtocolRequest::TurnSend { params, .. } = request else {
            return;
        };
        let conversation_key = resource_trace_key(&params.conversation);
        self.pending_by_conversation.remove(&conversation_key);
        let (Some(context), gateway::JsonRpcResponsePayload::Ok { result }) =
            (context, &response.response)
        else {
            return;
        };
        if let Ok(receipt) = serde_json::from_value::<gateway::TurnSendResponse>(result.clone()) {
            self.by_turn
                .insert(resource_trace_key(&receipt.turn.resource), context.clone());
        }
    }

    fn context_for_event(&self, event: &gateway::ProtocolEvent) -> Option<&gateway::TraceContext> {
        event_turn(event)
            .and_then(|turn| self.by_turn.get(&resource_trace_key(turn)))
            .or_else(|| {
                event_conversation(event).and_then(|conversation| {
                    self.pending_by_conversation
                        .get(&resource_trace_key(conversation))
                })
            })
    }

    fn observe_event(&mut self, event: &gateway::ProtocolEvent) -> Option<gateway::TraceContext> {
        let context = self.context_for_event(event).cloned();
        if let gateway::ProtocolEvent::TurnUpserted { params, .. } = event {
            if matches!(
                params.payload.turn.status,
                gateway::TurnStatus::Completed
                    | gateway::TurnStatus::Failed
                    | gateway::TurnStatus::Interrupted
            ) {
                self.by_turn
                    .remove(&resource_trace_key(&params.payload.turn.resource));
            }
        }
        context
    }
}

fn resource_trace_key(resource: &gateway::RoutedResourceId) -> String {
    format!("{}\0{}", resource.provider_id, resource.native_resource_id)
}

fn event_turn(event: &gateway::ProtocolEvent) -> Option<&gateway::RoutedResourceId> {
    match event {
        gateway::ProtocolEvent::TurnUpserted { params, .. } => Some(&params.payload.turn.resource),
        gateway::ProtocolEvent::TurnOutputDelta { params, .. } => Some(&params.payload.turn),
        gateway::ProtocolEvent::ConversationItemUpserted { params, .. } => {
            params.payload.item.as_ref().map(gateway_item_turn)
        }
        gateway::ProtocolEvent::ApprovalRequested { params, .. } => {
            Some(&params.payload.approval.turn)
        }
        gateway::ProtocolEvent::ApprovalResolved { params, .. } => {
            Some(&params.payload.approval.turn)
        }
        _ => None,
    }
}

fn event_conversation(event: &gateway::ProtocolEvent) -> Option<&gateway::RoutedResourceId> {
    match event {
        gateway::ProtocolEvent::ConversationUpserted { params, .. } => {
            Some(&params.payload.conversation.resource)
        }
        gateway::ProtocolEvent::TurnUpserted { params, .. } => {
            Some(&params.payload.turn.conversation)
        }
        gateway::ProtocolEvent::TurnOutputDelta { params, .. } => {
            Some(&params.payload.conversation)
        }
        gateway::ProtocolEvent::ConversationItemUpserted { params, .. } => params
            .payload
            .item
            .as_ref()
            .map(gateway_item_conversation)
            .or(params.payload.conversation.as_ref()),
        gateway::ProtocolEvent::ApprovalRequested { params, .. } => {
            Some(&params.payload.approval.conversation)
        }
        gateway::ProtocolEvent::ApprovalResolved { params, .. } => {
            Some(&params.payload.approval.conversation)
        }
        _ => None,
    }
}

fn gateway_item_turn(item: &gateway::ConversationItem) -> &gateway::RoutedResourceId {
    match item {
        gateway::ConversationItem::MessageConversationItem(item) => &item.turn,
        gateway::ConversationItem::ReasoningConversationItem(item) => &item.turn,
        gateway::ConversationItem::CommandConversationItem(item) => &item.turn,
        gateway::ConversationItem::FileChangeConversationItem(item) => &item.turn,
        gateway::ConversationItem::ToolConversationItem(item) => &item.turn,
        gateway::ConversationItem::ApprovalConversationItem(item) => &item.turn,
        gateway::ConversationItem::UnknownConversationItem(item) => &item.turn,
    }
}

fn gateway_item_conversation(item: &gateway::ConversationItem) -> &gateway::RoutedResourceId {
    match item {
        gateway::ConversationItem::MessageConversationItem(item) => &item.conversation,
        gateway::ConversationItem::ReasoningConversationItem(item) => &item.conversation,
        gateway::ConversationItem::CommandConversationItem(item) => &item.conversation,
        gateway::ConversationItem::FileChangeConversationItem(item) => &item.conversation,
        gateway::ConversationItem::ToolConversationItem(item) => &item.conversation,
        gateway::ConversationItem::ApprovalConversationItem(item) => &item.conversation,
        gateway::ConversationItem::UnknownConversationItem(item) => &item.conversation,
    }
}

fn write_trace_record(
    name: &str,
    context: Option<&gateway::TraceContext>,
    attributes: serde_json::Value,
) {
    let Some(context) = context else {
        return;
    };
    let mut parts = context.traceparent.split('-');
    let _version = parts.next();
    let Some(trace_id) = parts.next() else {
        return;
    };
    let Some(span_id) = parts.next() else {
        return;
    };
    let timestamp_us = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_micros())
        .unwrap_or_default();
    eprintln!(
        "{}",
        serde_json::json!({
            "schema": "codepet.trace.v1",
            "recordType": "event",
            "service": "gateway",
            "timestampUnixUs": timestamp_us.to_string(),
            "name": name,
            "traceId": trace_id,
            "spanId": span_id,
            "attributes": attributes,
        })
    );
}

/// Run an admitted connection with the same Gateway semantics on every channel.
pub(crate) async fn run_gateway_channel(
    channel: GatewayChannel,
    gateway: Arc<ProviderGatewayService>,
    remote_access: Arc<RemoteAccessManager>,
    credential: RemoteCredential,
    mut registration: SessionRegistration,
) {
    let GatewayChannel {
        mut sink,
        mut source,
    } = channel;
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<GatewayFrame>(OUTBOUND_QUEUE_CAPACITY);
    let (request_tx, request_rx) = mpsc::channel::<GatewayRequestDelivery>(REQUEST_QUEUE_CAPACITY);
    let trace_links = Arc::new(Mutex::new(SessionTraceLinks::default()));
    let (stop_tx, _) = watch::channel(false);
    let (writer_done_tx, mut writer_done) = watch::channel(false);
    let (transport_failed_tx, mut transport_failed) = watch::channel(false);
    let writer_failed = transport_failed_tx.clone();
    let writer = tokio::spawn(async move {
        while let Some(message) = outbound_rx.recv().await {
            let closing = matches!(message, GatewayFrame::Close { .. });
            let sent = timeout(CHANNEL_SEND_TIMEOUT, sink.send(message)).await;
            if !matches!(sent, Ok(true)) {
                let _ = writer_failed.send(true);
                break;
            }
            if closing {
                break;
            }
        }
        let _ = writer_done_tx.send(true);
    });
    let request_dispatcher = tokio::spawn(run_gateway_requests(
        request_rx,
        gateway.clone(),
        format!("remote-client:{}", credential.client_id),
        outbound_tx.clone(),
        transport_failed_tx.clone(),
        stop_tx.subscribe(),
        trace_links.clone(),
    ));

    let mut presence = None;
    let mut heartbeat = gateway::GatewayHeartbeat::default();
    let mut handshaken = false;
    let mut subscribed = false;
    let mut event_task: Option<JoinHandle<()>> = None;
    let mut close_frame = None;
    let caller_scope = format!("remote-client:{}", credential.client_id);

    loop {
        let next = tokio::select! {
            _ = tokio::time::sleep_until(heartbeat.deadline().into()) => {
                close_frame = Some(close_message(1001, "heartbeat_timeout"));
                break;
            }
            cancellation = registration.cancelled() => {
                close_frame = Some(match cancellation {
                    SessionCancellation::CredentialRevoked => close_message(1008, "credential_revoked"),
                    SessionCancellation::ServerShutdown => close_message(1001, "server_shutdown"),
                });
                break;
            }
            changed = writer_done.changed() => {
                let _ = changed;
                break;
            }
            changed = transport_failed.changed() => {
                if changed.is_err() || *transport_failed.borrow() {
                    break;
                }
                continue;
            }
            next = source.recv() => next,
        };
        let Some(message) = next else {
            break;
        };
        let text = match message {
            GatewayFrame::Text(text) => text,
            GatewayFrame::KeepAlive => continue,
            GatewayFrame::Close { .. } => break,
            GatewayFrame::Invalid => {
                close_frame = Some(close_message(1003, "text_frames_required"));
                break;
            }
        };
        let observed = match gateway::decode_observed_wire_message(&text) {
            Ok(observed) => observed,
            Err(error) => {
                if !queue_json(&outbound_tx, &error.into_response(), &mut registration).await {
                    break;
                }
                if !handshaken {
                    close_frame = Some(close_message(1008, "protocol_handshake_required"));
                    break;
                }
                continue;
            }
        };
        let trace_context = observed.trace_context;
        let request = match observed.message {
            gateway::ProviderWireMessage::Request(gateway::JsonRpcInboundRequest::Typed(
                request,
            )) => request,
            gateway::ProviderWireMessage::Request(gateway::JsonRpcInboundRequest::Rejected(
                rejection,
            )) => {
                if !queue_json(&outbound_tx, &rejection.into_response(), &mut registration).await {
                    break;
                }
                if !handshaken {
                    close_frame = Some(close_message(1008, "protocol_handshake_required"));
                    break;
                }
                continue;
            }
            _ => {
                close_frame = Some(close_message(1008, "gateway_requests_required"));
                break;
            }
        };
        write_trace_record(
            "gateway.rpc.received",
            trace_context.as_ref(),
            serde_json::json!({
                "rpc.method": request.method().as_str(),
                "rpc.requestId": request.id(),
            }),
        );

        if !handshaken {
            let gateway::ProtocolRequest::ProtocolHandshake {
                jsonrpc,
                id,
                params,
            } = request
            else {
                close_frame = Some(close_message(1008, "protocol_handshake_required"));
                break;
            };
            if params.client_id != credential.client_id {
                let response = json_rpc_error_response(
                    jsonrpc,
                    Some(id),
                    protocol_error(
                        "gateway_client_identity_mismatch",
                        "Handshake clientId does not match the authenticated credential",
                        false,
                    ),
                );
                if !queue_json(&outbound_tx, &response, &mut registration).await {
                    break;
                }
                close_frame = Some(close_message(1008, "gateway_client_identity_mismatch"));
                break;
            }
            let device_descriptor = params.device.clone();
            let response_id = id.clone();
            let request = gateway::ProtocolRequest::ProtocolHandshake {
                jsonrpc: jsonrpc.clone(),
                id,
                params,
            };
            let mut response = tokio::select! {
                cancellation = registration.cancelled() => {
                    close_frame = Some(cancellation_close(cancellation));
                    break;
                }
                response = gateway.dispatch_for_caller_scope(&caller_scope, request) => response,
            };
            let mut succeeded = matches!(
                &response.response,
                gateway::JsonRpcResponsePayload::Ok { .. }
            );
            if succeeded {
                if let Err(error) = remote_access
                    .update_credential_descriptor(&credential.credential_id, device_descriptor)
                {
                    response = json_rpc_error_response(
                        jsonrpc,
                        Some(response_id),
                        error.into_protocol_error(),
                    );
                    succeeded = false;
                    close_frame = Some(close_message(
                        1008,
                        "gateway_client_descriptor_persistence_failed",
                    ));
                }
            }
            if !queue_json(&outbound_tx, &response, &mut registration).await {
                break;
            }
            if !succeeded {
                if close_frame.is_none() {
                    close_frame = Some(close_message(1008, "protocol_handshake_rejected"));
                }
                break;
            }
            presence = Some(
                gateway
                    .remote_connections()
                    .register(credential.client_id.clone()),
            );
            heartbeat = gateway::GatewayHeartbeat::default();
            handshaken = true;
            continue;
        }

        if let gateway::ProtocolRequest::ProtocolHandshake { jsonrpc, id, .. } = &request {
            let response = json_rpc_error_response(
                jsonrpc.clone(),
                Some(id.clone()),
                protocol_error(
                    "gateway_handshake_already_completed",
                    "protocol.handshake may succeed only once per socket",
                    false,
                ),
            );
            if !queue_json(&outbound_tx, &response, &mut registration).await {
                break;
            }
            continue;
        }

        if let gateway::ProtocolRequest::ProtocolPing { params, .. } = &request {
            if !heartbeat.accept(params.sequence) {
                close_frame = Some(close_message(1008, "stale_heartbeat"));
                break;
            }
            let response = gateway
                .dispatch_for_caller_scope(&caller_scope, request)
                .await;
            if !queue_json(&outbound_tx, &response, &mut registration).await {
                break;
            }
            continue;
        }

        if let gateway::ProtocolRequest::EventSubscribe {
            jsonrpc,
            id,
            params,
        } = &request
        {
            if subscribed {
                let response = json_rpc_error_response(
                    jsonrpc.clone(),
                    Some(id.clone()),
                    protocol_error(
                        "gateway_event_already_subscribed",
                        "event.subscribe may succeed only once per socket",
                        false,
                    ),
                );
                if !queue_json(&outbound_tx, &response, &mut registration).await {
                    break;
                }
                continue;
            }
            let subscription = gateway.subscribe_events(Some(&params.after_cursor));
            let dispatcher = EventSubscribeDispatcher {
                response: subscription
                    .as_ref()
                    .map(|_| gateway::EventSubscribeResponse {
                        subscribed_after_cursor: params.after_cursor.clone(),
                    })
                    .map_err(|error| error.clone()),
            };
            let response = tokio::select! {
                cancellation = registration.cancelled() => {
                    close_frame = Some(cancellation_close(cancellation));
                    break;
                }
                response = gateway::dispatch(&dispatcher, request) => response,
            };
            if !queue_json(&outbound_tx, &response, &mut registration).await {
                break;
            }
            let Ok(mut subscription) = subscription else {
                continue;
            };
            subscribed = true;
            let event_outbound = outbound_tx.clone();
            let event_transport_failed = transport_failed_tx.clone();
            let event_trace_links = trace_links.clone();
            let mut event_stop = stop_tx.subscribe();
            event_task = Some(tokio::spawn(async move {
                loop {
                    let event = tokio::select! {
                        changed = event_stop.changed() => {
                            if changed.is_err() || *event_stop.borrow() {
                                break;
                            }
                            continue;
                        }
                        event = subscription.next_event() => event,
                    };
                    let event = match event {
                        Ok(event) => event,
                        Err(error) => {
                            if !enqueue_outbound(&event_outbound, close_message(1011, error.code))
                                .await
                            {
                                let _ = event_transport_failed.send(true);
                            }
                            break;
                        }
                    };
                    let trace_context = event_trace_links
                        .lock()
                        .ok()
                        .and_then(|mut links| links.observe_event(&event));
                    let text =
                        match gateway::encode_event_with_trace(&event, trace_context.as_ref()) {
                            Ok(bytes) => match String::from_utf8(bytes) {
                                Ok(text) => text,
                                Err(_) => break,
                            },
                            Err(_) => {
                                if !enqueue_outbound(
                                    &event_outbound,
                                    close_message(1011, "gateway_event_encoding_failed"),
                                )
                                .await
                                {
                                    let _ = event_transport_failed.send(true);
                                }
                                break;
                            }
                        };
                    if !enqueue_outbound(&event_outbound, GatewayFrame::text(text)).await {
                        let _ = event_transport_failed.send(true);
                        break;
                    }
                }
            }));
            continue;
        }

        match request_tx.try_send(GatewayRequestDelivery {
            request,
            trace_context,
            received_at: Instant::now(),
        }) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                close_frame = Some(close_message(1013, "gateway_request_queue_full"));
                break;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => break,
        }
    }

    drop(presence);
    let _ = stop_tx.send(true);
    drop(request_tx);
    // All cleanup shares one budget shorter than the caller's revocation wait.
    // In particular, a relay ACK must not consume that entire outer deadline.
    let cleanup_deadline = tokio::time::Instant::now() + CONNECTION_CLOSE_TIMEOUT / 2;
    if let Some(close_frame) = close_frame {
        let _ = outbound_tx.try_send(close_frame);
    }
    if let Some(mut event_task) = event_task {
        if tokio::time::timeout_at(cleanup_deadline, &mut event_task)
            .await
            .is_err()
        {
            event_task.abort();
            let _ = event_task.await;
        }
    }
    let mut request_dispatcher = request_dispatcher;
    if tokio::time::timeout_at(cleanup_deadline, &mut request_dispatcher)
        .await
        .is_err()
    {
        request_dispatcher.abort();
        let _ = request_dispatcher.await;
    }
    drop(outbound_tx);
    let mut writer = writer;
    if tokio::time::timeout_at(cleanup_deadline, &mut writer)
        .await
        .is_err()
    {
        writer.abort();
        let _ = writer.await;
    }
}

async fn run_gateway_requests(
    mut requests: mpsc::Receiver<GatewayRequestDelivery>,
    gateway: Arc<ProviderGatewayService>,
    caller_scope: String,
    outbound: mpsc::Sender<GatewayFrame>,
    transport_failed: watch::Sender<bool>,
    mut stop: watch::Receiver<bool>,
    trace_links: Arc<Mutex<SessionTraceLinks>>,
) {
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break;
                }
            }
            completed = tasks.join_next(), if !tasks.is_empty() => {
                if completed.is_some_and(|result| result.is_err()) {
                    let _ = transport_failed.send(true);
                    break;
                }
            }
            request = requests.recv(), if tasks.len() < MAX_CONCURRENT_REQUESTS_PER_SESSION => {
                let Some(delivery) = request else {
                    break;
                };
                let request_gateway = gateway.clone();
                let request_scope = caller_scope.clone();
                let request_outbound = outbound.clone();
                let request_transport_failed = transport_failed.clone();
                let mut request_stop = stop.clone();
                let request_trace_links = trace_links.clone();
                tasks.spawn(async move {
                    let GatewayRequestDelivery {
                        request,
                        trace_context,
                        received_at,
                    } = delivery;
                    if let Ok(mut links) = request_trace_links.lock() {
                        links.begin_request(&request, trace_context.as_ref());
                    }
                    let trace_request = request.clone();
                    let method = request.method();
                    let request_id = request.id().clone();
                    let response = tokio::select! {
                        biased;
                        changed = request_stop.changed() => {
                            let _ = changed;
                            return;
                        }
                        response = request_gateway.dispatch_for_caller_scope(&request_scope, request) => response,
                    };
                    if let Ok(mut links) = request_trace_links.lock() {
                        links.complete_request(
                            &trace_request,
                            &response,
                            trace_context.as_ref(),
                        );
                    }
                    write_trace_record(
                        "gateway.rpc.completed",
                        trace_context.as_ref(),
                        serde_json::json!({
                            "rpc.method": method.as_str(),
                            "rpc.requestId": request_id,
                            "durationUs": received_at.elapsed().as_micros().to_string(),
                            "status": if matches!(&response.response, gateway::JsonRpcResponsePayload::Ok { .. }) { "ok" } else { "error" },
                        }),
                    );
                    let text = match serde_json::to_string(&response) {
                        Ok(text) => text,
                        Err(_) => {
                            let _ = request_transport_failed.send(true);
                            return;
                        }
                    };
                    if !enqueue_outbound(&request_outbound, GatewayFrame::text(text)).await {
                        let _ = request_transport_failed.send(true);
                    }
                });
            }
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

async fn queue_json<T: serde::Serialize>(
    outbound: &mpsc::Sender<GatewayFrame>,
    value: &T,
    registration: &mut SessionRegistration,
) -> bool {
    let text = match serde_json::to_string(value) {
        Ok(text) => text,
        Err(_) => return false,
    };
    tokio::select! {
        cancellation = registration.cancelled() => {
            let _ = cancellation;
            false
        }
        sent = enqueue_outbound(outbound, GatewayFrame::text(text)) => sent,
    }
}

async fn enqueue_outbound(outbound: &mpsc::Sender<GatewayFrame>, message: GatewayFrame) -> bool {
    matches!(
        timeout(OUTBOUND_ENQUEUE_TIMEOUT, outbound.send(message)).await,
        Ok(Ok(()))
    )
}

fn close_message(code: u16, reason: impl AsRef<[u8]>) -> GatewayFrame {
    GatewayFrame::Close {
        code,
        reason: String::from_utf8_lossy(reason.as_ref()).into_owned(),
    }
}

fn cancellation_close(cancellation: SessionCancellation) -> GatewayFrame {
    match cancellation {
        SessionCancellation::CredentialRevoked => close_message(1008, "credential_revoked"),
        SessionCancellation::ServerShutdown => close_message(1001, "server_shutdown"),
    }
}

pub(crate) fn protocol_error(
    code: impl Into<String>,
    message: impl Into<String>,
    retryable: bool,
) -> gateway::ProtocolError {
    gateway::ProtocolError {
        code: code.into(),
        message: message.into(),
        retryable,
        details: None,
    }
}

fn json_rpc_error_response(
    jsonrpc: String,
    id: Option<gateway::RequestId>,
    error: gateway::ProtocolError,
) -> gateway::JsonRpcResponse {
    let mut data = error.details.unwrap_or_default();
    data.insert("code".to_string(), serde_json::Value::String(error.code));
    data.insert(
        "retryable".to_string(),
        serde_json::Value::Bool(error.retryable),
    );
    gateway::JsonRpcResponse {
        jsonrpc,
        id,
        response: gateway::JsonRpcResponsePayload::Error {
            error: gateway::RpcError {
                code: -32000,
                message: error.message,
                data: Some(data),
            },
        },
    }
}

struct EventSubscribeDispatcher {
    response: Result<gateway::EventSubscribeResponse, gateway::ProtocolError>,
}

impl gateway::ProtocolServer for EventSubscribeDispatcher {
    fn event_subscribe<'a>(
        &'a self,
        _request: gateway::EventSubscribeRequest,
    ) -> gateway::ProtocolFuture<'a, gateway::EventSubscribeResponse> {
        let response = self.response.clone();
        Box::pin(async move { response })
    }
}

#[derive(Clone, Copy)]
pub(crate) enum SessionCancellation {
    CredentialRevoked,
    ServerShutdown,
}

struct SessionGroup {
    sender: broadcast::Sender<SessionCancellation>,
    active: usize,
    cancelled: bool,
}

struct SessionRegistryState {
    groups: BTreeMap<String, SessionGroup>,
    active: usize,
    shutting_down: bool,
}

pub(crate) struct SessionRegistry {
    state: Mutex<SessionRegistryState>,
    empty: Notify,
    slots: Arc<Semaphore>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CredentialGroupDisconnectOutcome {
    pub(crate) active: usize,
    pub(crate) remaining: usize,
}

impl SessionRegistry {
    pub(crate) fn new(max_sessions: usize) -> Self {
        Self {
            state: Mutex::new(SessionRegistryState {
                groups: BTreeMap::new(),
                active: 0,
                shutting_down: false,
            }),
            empty: Notify::new(),
            slots: Arc::new(Semaphore::new(max_sessions)),
        }
    }

    pub(crate) fn register(
        self: &Arc<Self>,
        credential_id: &str,
    ) -> Result<SessionRegistration, SessionRegistrationError> {
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| SessionRegistrationError::LimitReached)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| SessionRegistrationError::ShuttingDown)?;
        if state.shutting_down {
            return Err(SessionRegistrationError::ShuttingDown);
        }
        let group = state
            .groups
            .entry(credential_id.to_string())
            .or_insert_with(|| {
                let (sender, _) = broadcast::channel(1);
                SessionGroup {
                    sender,
                    active: 0,
                    cancelled: false,
                }
            });
        if group.cancelled {
            return Err(SessionRegistrationError::CredentialRevoked);
        }
        group.active += 1;
        let sender = group.sender.clone();
        let receiver = sender.subscribe();
        state.active += 1;
        Ok(SessionRegistration {
            registry: self.clone(),
            credential_id: credential_id.to_string(),
            sender,
            receiver,
            _permit: permit,
        })
    }

    pub(crate) fn cancel_credential(&self, credential_id: &str) -> usize {
        let cancelled = self.state.lock().ok().and_then(|mut state| {
            state.groups.get_mut(credential_id).map(|group| {
                group.cancelled = true;
                (group.sender.clone(), group.active)
            })
        });
        let Some((sender, active)) = cancelled else {
            return 0;
        };
        if active > 0 {
            let _ = sender.send(SessionCancellation::CredentialRevoked);
        }
        active
    }

    pub(crate) fn shutdown(&self) {
        let senders = match self.state.lock() {
            Ok(mut state) => {
                if state.shutting_down {
                    return;
                }
                state.shutting_down = true;
                std::mem::take(&mut state.groups)
                    .into_values()
                    .map(|group| group.sender)
                    .collect::<Vec<_>>()
            }
            Err(_) => return,
        };
        for sender in senders {
            let _ = sender.send(SessionCancellation::ServerShutdown);
        }
    }

    pub(crate) fn active_count(&self) -> usize {
        self.state.lock().map(|state| state.active).unwrap_or(0)
    }

    pub(crate) fn credential_active_count(&self, credential_id: &str) -> usize {
        self.state
            .lock()
            .ok()
            .and_then(|state| state.groups.get(credential_id).map(|group| group.active))
            .unwrap_or(0)
    }

    pub(crate) async fn cancel_and_wait_credentials(
        &self,
        credential_ids: &BTreeSet<String>,
        duration: Duration,
    ) -> CredentialGroupDisconnectOutcome {
        let active = credential_ids.iter().fold(0_usize, |total, credential_id| {
            total.saturating_add(self.cancel_credential(credential_id))
        });
        let drained = self.wait_credentials_empty(credential_ids, duration).await;
        let remaining = if drained {
            0
        } else {
            credential_ids
                .iter()
                .filter(|credential_id| self.credential_active_count(credential_id) > 0)
                .count()
        };
        CredentialGroupDisconnectOutcome { active, remaining }
    }

    pub(crate) async fn wait_credentials_empty(
        &self,
        credential_ids: &BTreeSet<String>,
        duration: Duration,
    ) -> bool {
        if credential_ids
            .iter()
            .all(|credential_id| self.credential_active_count(credential_id) == 0)
        {
            return true;
        }
        let waited = timeout(duration, async {
            loop {
                self.empty.notified().await;
                if credential_ids
                    .iter()
                    .all(|credential_id| self.credential_active_count(credential_id) == 0)
                {
                    return;
                }
            }
        })
        .await;
        waited.is_ok()
            || credential_ids
                .iter()
                .all(|credential_id| self.credential_active_count(credential_id) == 0)
    }

    pub(crate) async fn wait_empty(&self, duration: Duration) -> bool {
        if self.active_count() == 0 {
            return true;
        }
        timeout(duration, async {
            loop {
                self.empty.notified().await;
                if self.active_count() == 0 {
                    return;
                }
            }
        })
        .await
        .is_ok()
    }

    fn unregister(&self, credential_id: &str, sender: &broadcast::Sender<SessionCancellation>) {
        match self.state.lock() {
            Ok(mut state) => {
                if state.active > 0 {
                    state.active -= 1;
                }
                let remove_group = state
                    .groups
                    .get_mut(credential_id)
                    .filter(|group| group.sender.same_channel(sender))
                    .map(|group| {
                        if group.active > 0 {
                            group.active -= 1;
                        }
                        group.active == 0
                    })
                    .unwrap_or(false);
                if remove_group {
                    state.groups.remove(credential_id);
                }
            }
            Err(_) => return,
        }
        self.empty.notify_waiters();
    }
}

pub(crate) struct SessionRegistration {
    registry: Arc<SessionRegistry>,
    credential_id: String,
    sender: broadcast::Sender<SessionCancellation>,
    receiver: broadcast::Receiver<SessionCancellation>,
    _permit: OwnedSemaphorePermit,
}

#[derive(Debug)]
pub(crate) enum SessionRegistrationError {
    ShuttingDown,
    LimitReached,
    CredentialRevoked,
}

impl SessionRegistration {
    pub(crate) async fn cancelled(&mut self) -> SessionCancellation {
        match self.receiver.recv().await {
            Ok(cancellation) => cancellation,
            Err(_) => SessionCancellation::ServerShutdown,
        }
    }
}

impl Drop for SessionRegistration {
    fn drop(&mut self) {
        self.registry.unregister(&self.credential_id, &self.sender);
    }
}
