pub(crate) mod diagnostics;
mod framing;
use diagnostics::Diagnostic;
use serde_json::json;
pub(crate) mod cloud;

use super::session::{
    run_gateway_channel, ChannelFuture, GatewayChannel, GatewayFrame, GatewaySink, GatewaySource,
    SessionRegistration,
};
use crate::{HostError, HostResult, ProviderGatewayService, RemoteAccessManager, RemoteCredential};
use bytes::Bytes;
use framing::{encode_fragment, Decoder, FRAME_BYTES, HEADER_BYTES, REQUEST_BYTES, RESPONSE_BYTES};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::time::{timeout, Instant};
use webrtc::api::APIBuilder;
use webrtc::data_channel::{
    data_channel_init::RTCDataChannelInit, data_channel_message::DataChannelMessage,
    data_channel_state::RTCDataChannelState, RTCDataChannel,
};
use webrtc::peer_connection::{
    configuration::RTCConfiguration,
    sdp::{sdp_type::RTCSdpType, session_description::RTCSessionDescription},
    RTCPeerConnection,
};

const NEGOTIATION_TIMEOUT: Duration = Duration::from_secs(8);
const OPEN_TIMEOUT: Duration = Duration::from_secs(15);
const FRAGMENT_TIMEOUT: Duration = Duration::from_secs(5);
const BUFFER_HIGH: usize = 64 * 1024;
const BUFFER_LOW: usize = 16 * 1024;

/// Close even if the HTTP request is cancelled while SDP is being prepared.
struct PeerLease(Arc<RTCPeerConnection>);
impl Drop for PeerLease {
    fn drop(&mut self) {
        let peer = self.0.clone();
        tokio::spawn(async move {
            let _ = peer.close().await;
        });
    }
}

/// LAN bootstrap signaling only. Caller authenticates the pinned HTTPS request
/// and registers its credential before allocating ICE/DTLS resources.
pub(crate) async fn answer_offer(
    offer: RTCSessionDescription,
    gateway: Arc<ProviderGatewayService>,
    access: Arc<RemoteAccessManager>,
    credential: RemoteCredential,
    registration: SessionRegistration,
) -> HostResult<RTCSessionDescription> {
    answer_offer_configured(
        offer,
        gateway,
        access,
        credential,
        registration,
        RTCConfiguration::default(),
    )
    .await
}

pub(crate) async fn answer_offer_configured(
    offer: RTCSessionDescription,
    gateway: Arc<ProviderGatewayService>,
    access: Arc<RemoteAccessManager>,
    credential: RemoteCredential,
    mut registration: SessionRegistration,
    configuration: RTCConfiguration,
) -> HostResult<RTCSessionDescription> {
    let diagnostic = Diagnostic::new(&offer.sdp);
    diagnostic.emit("connect.start", json!({"iceServerCount":configuration.ice_servers.len(),"policy":format!("{:?}",configuration.ice_transport_policy)}));
    diagnostic.description("remoteOffer", &offer.sdp);
    let negotiation_timeout = if configuration.ice_servers.is_empty() {
        NEGOTIATION_TIMEOUT
    } else {
        Duration::from_secs(20)
    };
    let media = offer
        .sdp
        .lines()
        .filter(|line| line.starts_with("m="))
        .collect::<Vec<_>>();
    if offer.sdp_type != RTCSdpType::Offer
        || offer.sdp.len() > 60 * 1024
        || media.len() != 1
        || !media[0].starts_with("m=application ")
    {
        return Err(rtc_error("invalid RTC offer"));
    }
    let peer = Arc::new(
        APIBuilder::new()
            .build()
            .new_peer_connection(configuration)
            .await
            .map_err(|_| rtc_error("create RTC peer"))?,
    );
    let lease = PeerLease(peer.clone());
    let peer_diag = diagnostic.clone();
    peer.on_peer_connection_state_change(Box::new(move |state| {
        peer_diag.emit("peer.state", json!({"state":state.to_string()}));
        Box::pin(async {})
    }));
    let ice_diag = diagnostic.clone();
    peer.on_ice_connection_state_change(Box::new(move |state| {
        ice_diag.emit("ice.state", json!({"state":state.to_string()}));
        Box::pin(async {})
    }));
    let gather_diag = diagnostic.clone();
    peer.on_ice_gathering_state_change(Box::new(move |state| {
        gather_diag.emit("ice.gathering", json!({"state":state.to_string()}));
        Box::pin(async {})
    }));
    let candidate_diag = diagnostic.clone();
    peer.on_ice_candidate(Box::new(move |candidate| {
        if let Some(candidate) = candidate {
            if let Ok(value) = candidate.to_json() {
                candidate_diag.emit(
                    "ice.localCandidate",
                    diagnostics::candidate(&value.candidate, &candidate_diag.offer_id),
                );
            }
        }
        Box::pin(async {})
    }));
    // Weak ownership: observing a peer must not keep a cancelled connection alive.
    let weak_peer = Arc::downgrade(&peer);
    let stats_diag = diagnostic.clone();
    tokio::spawn(async move {
        let mut sample = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(if sample < 30 { 2 } else { 30 })).await;
            let Some(peer) = weak_peer.upgrade() else {
                break;
            };
            stats_diag.stats(&peer, "periodic").await;
            if peer.connection_state()
                == webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Closed
            {
                break;
            }
            sample += 1;
        }
    });
    peer.on_data_channel(Box::new(|channel| {
        Box::pin(async move {
            let _ = channel.close().await;
        })
    }));
    let setup = async {
        let dc = peer
            .create_data_channel(
                "codepet.gateway.v1",
                Some(RTCDataChannelInit {
                    ordered: Some(true),
                    negotiated: Some(0),
                    protocol: Some("codepet.gateway.cpg1".into()),
                    ..Default::default()
                }),
            )
            .await
            .map_err(|_| rtc_error("create RTC channel"))?;
        let (channel, closed) = channel_adapter(dc.clone(), diagnostic.clone());
        peer.set_remote_description(offer)
            .await
            .map_err(|_| rtc_error("apply RTC offer"))?;
        let answer = peer
            .create_answer(None)
            .await
            .map_err(|_| rtc_error("create RTC answer"))?;
        let mut gathering = peer.gathering_complete_promise().await;
        peer.set_local_description(answer)
            .await
            .map_err(|_| rtc_error("apply RTC answer"))?;
        let _ = gathering.recv().await;
        let answer = peer
            .local_description()
            .await
            .ok_or_else(|| rtc_error("missing RTC answer"))?;
        diagnostic.description("localAnswer", &answer.sdp);
        Ok::<_, HostError>((answer, dc, channel, closed))
    };
    let (answer, dc, channel, mut closed) = tokio::select! {
        result = timeout(negotiation_timeout, setup) => match result {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => { diagnostic.emit("negotiation.failed",json!({"cause":"setupError"})); diagnostic.stats(&peer,"negotiationFailed").await; return Err(error); },
            Err(_) => { diagnostic.emit("negotiation.failed",json!({"cause":"timeout"})); diagnostic.stats(&peer,"negotiationTimeout").await; return Err(rtc_error("RTC negotiation timed out")); },
        },
        _ = registration.cancelled() => return Err(rtc_error("RTC admission cancelled")),
    };
    tokio::spawn(async move {
        let _lease = lease;
        let opened = tokio::select! {
            result = timeout(OPEN_TIMEOUT, async {
                while dc.ready_state() != RTCDataChannelState::Open {
                    if dc.ready_state() == RTCDataChannelState::Closed { return false; }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                true
            }) => result.unwrap_or(false),
            _ = registration.cancelled() => false,
            _ = closed.changed() => false,
        };
        diagnostic.emit("channel.openResult", json!({"opened":opened}));
        diagnostic.stats(&_lease.0, "openResult").await;
        if opened {
            run_gateway_channel(channel, gateway, access, credential, registration).await;
        }
        diagnostic.stats(&_lease.0, "closing").await;
        diagnostic.emit("channel.close", json!({}));
    });
    Ok(answer)
}

fn rtc_error(message: &str) -> HostError {
    HostError::new("remote_rtc_negotiation_failed", message).retryable(true)
}

fn channel_adapter(
    dc: Arc<RTCDataChannel>,
    diagnostic: Arc<Diagnostic>,
) -> (GatewayChannel, watch::Receiver<bool>) {
    // Queue fragments rather than complete messages, bounding native ingress.
    let (tx, rx) = mpsc::channel(32);
    let (closed_tx, closed) = watch::channel(false);
    let ended = closed_tx.clone();
    let close_diag = diagnostic.clone();
    dc.on_close(Box::new(move || {
        close_diag.emit("dataChannel.closed", json!({}));
        let ended = ended.clone();
        Box::pin(async move {
            let _ = ended.send(true);
        })
    }));
    let failed = closed_tx.clone();
    let error_diag = diagnostic.clone();
    dc.on_error(Box::new(move |_| {
        error_diag.emit("dataChannel.error", json!({}));
        let failed = failed.clone();
        Box::pin(async move {
            let _ = failed.send(true);
        })
    }));
    let ingress_diag = diagnostic.clone();
    dc.on_message(Box::new(move |message: DataChannelMessage| {
        let ingress_diag = ingress_diag.clone();
        let tx = tx.clone();
        let failed = closed_tx.clone();
        Box::pin(async move {
            if message.is_string
                || message.data.len() > FRAME_BYTES
                || tx.try_send(message.data).is_err()
            {
                ingress_diag.emit(
                    "receive.rejected",
                    json!({"cause":"nonBinaryOversizedOrQueueFull"}),
                );
                let _ = failed.send(true);
            }
        })
    }));
    (
        GatewayChannel {
            sink: Box::new(RtcSink(dc, diagnostic.clone())),
            source: Box::new(RtcSource {
                rx,
                closed: closed.clone(),
                decoder: Decoder::new(REQUEST_BYTES),
                deadline: None,
                diagnostic,
                last_fragment: None,
                frames: 0,
            }),
        },
        closed,
    )
}

struct RtcSink(Arc<RTCDataChannel>, Arc<Diagnostic>);
impl RtcSink {
    async fn fragment(&self, bytes: Vec<u8>) -> bool {
        if self.0.buffered_amount().await > BUFFER_HIGH {
            let started = Instant::now();
            self.1.emit(
                "send.backpressure.start",
                json!({"bufferedBytes":self.0.buffered_amount().await}),
            );
            while self.0.buffered_amount().await > BUFFER_LOW {
                if self.0.ready_state() != RTCDataChannelState::Open {
                    return false;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            self.1.emit(
                "send.backpressure.end",
                json!({"waitMs":started.elapsed().as_millis()}),
            );
        }
        self.0.send(&Bytes::from(bytes)).await.is_ok()
    }
}
impl GatewaySink for RtcSink {
    fn send(&mut self, frame: GatewayFrame) -> ChannelFuture<'_, bool> {
        Box::pin(async move {
            match frame {
                GatewayFrame::Text(bytes) => {
                    let started = Instant::now();
                    self.1
                        .emit("message.send.start", json!({"bytes":bytes.len()}));
                    if bytes.is_empty() || bytes.len() > RESPONSE_BYTES {
                        return false;
                    }
                    for (index, part) in bytes.chunks(FRAME_BYTES - HEADER_BYTES).enumerate() {
                        if !self
                            .fragment(encode_fragment(
                                bytes.len(),
                                index * (FRAME_BYTES - HEADER_BYTES),
                                part,
                            ))
                            .await
                        {
                            return false;
                        }
                    }
                    self.1.emit(
                        "message.send.complete",
                        json!({"bytes":bytes.len(),"durationMs":started.elapsed().as_millis()}),
                    );
                    true
                }
                GatewayFrame::Close { code, reason } => {
                    if reason.len() > 123 {
                        return false;
                    }
                    let mut payload = code.to_be_bytes().to_vec();
                    payload.extend_from_slice(reason.as_bytes());
                    if !self.fragment(encode_fragment(0, 0, &payload)).await {
                        return false;
                    }
                    while self.0.buffered_amount().await > 0 {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    true
                }
                _ => false,
            }
        })
    }
}

struct RtcSource {
    rx: mpsc::Receiver<Bytes>,
    closed: watch::Receiver<bool>,
    decoder: Decoder,
    deadline: Option<Instant>,
    diagnostic: Arc<Diagnostic>,
    last_fragment: Option<Instant>,
    frames: u64,
}
impl GatewaySource for RtcSource {
    fn recv(&mut self) -> ChannelFuture<'_, Option<GatewayFrame>> {
        Box::pin(async move {
            loop {
                if *self.closed.borrow() {
                    return None;
                }
                let deadline = self
                    .deadline
                    .unwrap_or_else(|| Instant::now() + FRAGMENT_TIMEOUT);
                let bytes = tokio::select! {
                    _ = tokio::time::sleep_until(deadline), if self.deadline.is_some() => {
                        self.diagnostic.emit("message.receive.timeout",json!({"received":self.decoder.received_bytes(),
                            "total":self.decoder.total_bytes(),"frames":self.frames,
                            "lastFragmentAgeMs":self.last_fragment.map(|t|t.elapsed().as_millis())}));
                        return Some(GatewayFrame::Invalid);
                    },
                    _ = self.closed.changed() => return None,
                    bytes = self.rx.recv() => bytes?,
                };
                self.last_fragment = Some(Instant::now());
                self.frames += 1;
                match self.decoder.push(&bytes) {
                    Ok(Some(message)) => {
                        let size = match &message {
                            GatewayFrame::Text(bytes) => bytes.len(),
                            _ => 0,
                        };
                        self.diagnostic.emit(
                            "message.receive.complete",
                            json!({"bytes":size,"frames":self.frames}),
                        );
                        self.frames = 0;
                        self.deadline = None;
                        return Some(message);
                    }
                    Ok(None) => {
                        if self.frames == 1 || self.frames % 64 == 0 {
                            self.diagnostic.emit("message.receive.progress",json!({"received":self.decoder.received_bytes(),"total":self.decoder.total_bytes(),"frames":self.frames}));
                        }
                        if self.deadline.is_none() {
                            self.deadline = Some(Instant::now() + FRAGMENT_TIMEOUT);
                        }
                    }
                    Err(()) => return Some(GatewayFrame::Invalid),
                }
            }
        })
    }
}
