//! Host adapter: assemble a transport profile, then normalize RPC replies and events.
//! Independent codecs/framing/flow control live in codepet-transport.
use super::*;
use codepet_provider_sdk::{MuxMessage, ProviderMessageCodec};
use codepet_transport::{serial, Profile};
use tokio::sync::oneshot;

#[derive(Clone)]
pub(super) enum Connection {
    Mux(codepet_provider_sdk::ProviderMux),
    Serial(Arc<SerialClient>),
}
pub(super) struct SerialClient {
    sender: serial::Sender<ProviderMessageCodec>,
    pending: StdMutex<BTreeMap<String, oneshot::Sender<MuxMessage>>>,
}
struct Pending<'a>(&'a SerialClient, String);
impl Drop for Pending<'_> {
    fn drop(&mut self) {
        self.0
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.1);
    }
}
impl Connection {
    pub(super) fn close(&self) {
        match self {
            Self::Mux(peer) => peer.close(),
            Self::Serial(peer) => {
                peer.sender.close();
                peer.pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clear();
            }
        }
    }
    pub(super) async fn exchange(
        &self,
        message: ProviderWireMessage,
    ) -> Result<MuxMessage, ProtocolError> {
        match self {
            Self::Mux(peer) => peer.exchange(message).await,
            Self::Serial(peer) => {
                let ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(ref request)) =
                    message
                else {
                    return Err(protocol_error(
                        "provider_request_invalid",
                        "Host must send typed requests",
                        false,
                    ));
                };
                let id = request.id().clone();
                let (tx, rx) = oneshot::channel();
                peer.pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id.clone(), tx);
                let _pending = Pending(peer, id.clone());
                peer.sender.send(message).await?;
                rx.await.map_err(|_| {
                    protocol_error(
                        "provider_process_closed",
                        "Provider transport closed before response",
                        true,
                    )
                })
            }
        }
    }
}

pub(super) async fn run(
    stdout: ChildStdout,
    stdin: ChildStdin,
    shared: Arc<RpcShared>,
    control: mpsc::Sender<ProcessCommand>,
    ready: watch::Sender<Option<Result<Connection, ProtocolError>>>,
    max_frame_bytes: usize,
    profile: String,
) {
    let mut limits = codepet_provider_sdk::default_transport_limits();
    limits.max_encoded_message_bytes = max_frame_bytes as u64;
    let result = match Profile::parse(&profile) {
        Ok(Profile::Mux) => run_mux(stdout, stdin, shared.clone(), ready.clone(), limits).await,
        Ok(profile) => {
            run_serial(
                stdout,
                stdin,
                shared.clone(),
                ready.clone(),
                limits,
                profile,
            )
            .await
        }
        Err(error) => Err(error.into()),
    };
    if let Some(Ok(connection)) = ready.borrow().as_ref() {
        connection.close();
    }
    if !shared.is_shutting_down() {
        let error = result.err().unwrap_or_else(|| {
            protocol_error("provider_process_closed", "Provider transport closed", true)
        });
        if ready.borrow().is_none() {
            ready.send_replace(Some(Err(error.clone())));
        }
        fail_transport(&shared, &control, error).await;
    }
}
async fn run_mux(
    stdout: ChildStdout,
    stdin: ChildStdin,
    shared: Arc<RpcShared>,
    ready: watch::Sender<Option<Result<Connection, ProtocolError>>>,
    limits: codepet_provider_sdk::ProviderTransportLimits,
) -> Result<(), ProtocolError> {
    let (peer, mut incoming, mut driver) =
        codepet_provider_sdk::ProviderMux::connect(stdout, stdin, true, limits).await?;
    ready.send_replace(Some(Ok(Connection::Mux(peer.clone()))));
    loop {
        tokio::select! {
            result=&mut driver=>return result.map_err(|e|protocol_error("provider_mux_driver_failed",e.to_string(),true))?,
            next=incoming.recv()=>{
                let Some(mut stream)=next else { return (&mut driver).await.map_err(|e|protocol_error("provider_mux_driver_failed",e.to_string(),true))?; };
                let received=tokio::time::timeout(peer.stream_timeout(),stream.receive()).await
                    .map_err(|_|protocol_error("provider_event_timeout","Provider event stream expired",true))??;
                let (message,_budget)=received.into_parts();
                match message {
                    ProviderWireMessage::Event(event)=>{
                        if !shared.is_shutting_down() {
                            shared.send_inbound(ProviderWireMessage::Event(event), _budget)?;
                            tokio::select! {
                                result=shared.wait_inbound_consumed()=>result?,
                                result=&mut driver=>return result.map_err(|e|protocol_error("provider_mux_driver_failed",e.to_string(),true))?,
                            }
                        }
                        if let Err(error)=stream.acknowledge().await { if !shared.is_shutting_down() { return Err(error); } }
                    },
                    _=>return Err(protocol_error("unexpected_provider_request","Provider may only open event streams",false)),
                }
            }
        }
    }
}
async fn run_serial(
    stdout: ChildStdout,
    stdin: ChildStdin,
    shared: Arc<RpcShared>,
    ready: watch::Sender<Option<Result<Connection, ProtocolError>>>,
    limits: codepet_provider_sdk::ProviderTransportLimits,
    profile: Profile,
) -> Result<(), ProtocolError> {
    let (sender, mut incoming, mut driver) =
        serial::connect::<ProviderMessageCodec, _, _>(stdout, stdin, profile, limits)?;
    let client = Arc::new(SerialClient {
        sender,
        pending: StdMutex::new(BTreeMap::new()),
    });
    ready.send_replace(Some(Ok(Connection::Serial(client.clone()))));
    loop {
        tokio::select! {
            biased;
            // A flushed final response may precede EOF in the same poll. Drain queued input first.
            next=incoming.recv()=>{
                let Some(received)=next else { return (&mut driver).await.map_err(|e|protocol_error("provider_driver_failed",e.to_string(),true))?; };
                if let ProviderWireMessage::Response(ref response)=received.message {
                    let Some(id)=response.id.as_ref() else { return Err(protocol_error("provider_response_invalid","Provider response requires a request ID",false)); };
                    if let Some(sender)=client.pending.lock().unwrap_or_else(|e|e.into_inner()).remove(id) { let _=sender.send(received); }
                    // Responses to cancelled/deadline-expired requests are safely discarded.
                } else {
                    let (message,_budget)=received.into_parts();
                    match message {
                        ProviderWireMessage::Event(event)=>{
                            if !shared.is_shutting_down() {
                                shared.send_inbound(ProviderWireMessage::Event(event), _budget)?;
                                tokio::select! {
                                    result=shared.wait_inbound_consumed()=>result?,
                                    result=&mut driver=>return result.map_err(|e|protocol_error("provider_driver_failed",e.to_string(),true))?,
                                }
                            }
                        },
                        _=>return Err(protocol_error("unexpected_provider_request","Provider may only send responses and events",false)),
                    }
                }
            }
            result=&mut driver=>return result.map_err(|e|protocol_error("provider_driver_failed",e.to_string(),true))?,
        }
    }
}
