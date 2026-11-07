pub(crate) mod diagnostics;
mod framing;
mod path_policy;
use diagnostics::Diagnostic;
use path_policy::PathPolicy;
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
    registration: SessionRegistration,
    configuration: RTCConfiguration,
) -> HostResult<RTCSessionDescription> {
    answer_offer_with_policy(
        offer,
        gateway,
        access,
        credential,
        registration,
        configuration,
        PathPolicy::Auto,
        None,
    )
    .await
}

async fn answer_offer_with_policy(
    mut offer: RTCSessionDescription,
    gateway: Arc<ProviderGatewayService>,
    access: Arc<RemoteAccessManager>,
    credential: RemoteCredential,
    mut registration: SessionRegistration,
    mut configuration: RTCConfiguration,
    path_policy: PathPolicy,
    mut updates: Option<
        mpsc::Receiver<HostResult<Vec<webrtc::ice_transport::ice_candidate::RTCIceCandidateInit>>>,
    >,
) -> HostResult<RTCSessionDescription> {
    let settings = path_policy.configure(&mut configuration);
    let compression = framing::supports_compression(&offer.sdp);
    let supports_control = offer.sdp.lines().any(|line| line.trim() == "a=x-codepet-channel-control:1");
    // Early SDP starts this phase earlier than legacy full gathering. Preserve
    // the race's 55s budget so a genuinely late useful candidate is not lost.
    let trickle_deadline = updates
        .as_ref()
        .map(|_| Instant::now() + Duration::from_secs(55));
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
    // Reparse after filtering: RTCSessionDescription also caches parsed SDP.
    offer = RTCSessionDescription::offer(
        path_policy.filter_sdp_with_empty(&offer.sdp, updates.is_some())?,
    )
    .map_err(|_| rtc_error("invalid RTC offer SDP"))?;
    let peer = Arc::new(
        APIBuilder::new()
            .with_setting_engine(settings)
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
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let Some(peer) = weak_peer.upgrade() else {
                break;
            };
            stats_diag.stats(&peer, "periodic").await;
            if peer.connection_state()
                == webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Closed
            {
                break;
            }
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
        let (channel, closed) = channel_adapter(dc.clone(), diagnostic.clone(), compression);
        peer.set_remote_description(offer)
            .await
            .map_err(|_| rtc_error("apply RTC offer"))?;
        if let Some(updates) = updates.take() {
            apply_remote_updates(&peer, updates, path_policy, diagnostic.clone());
        }
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
        let mut answer_sdp = path_policy.filter_sdp(&answer.sdp)?;
        if compression {
            answer_sdp.push_str(framing::COMPRESSION_SDP);
            answer_sdp.push_str("\r\n");
        }
        if supports_control { answer_sdp.push_str("a=x-codepet-channel-control:1\r\n"); }
        let answer = RTCSessionDescription::answer(answer_sdp)
            .map_err(|_| rtc_error("invalid RTC answer SDP"))?;
        diagnostic.description("localAnswer", &answer.sdp);
        Ok::<_, HostError>((answer, dc, channel, closed))
    };
    let (answer, dc, channel, mut closed) = tokio::select! {
        result = timeout(negotiation_timeout, setup) => match result {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => { diagnostic.emit("negotiation.failed",json!({"cause":"setupError","code":error.code,"reason":error.message})); diagnostic.stats(&peer,"negotiationFailed").await; return Err(error); },
            Err(_) => { diagnostic.emit("negotiation.failed",json!({"cause":"timeout"})); diagnostic.stats(&peer,"negotiationTimeout").await; return Err(rtc_error("RTC negotiation timed out")); },
        },
        _ = registration.cancelled() => return Err(rtc_error("RTC admission cancelled")),
    };
    tokio::spawn(async move {
        let _lease = lease;
        let opened = tokio::select! {
            result = timeout(trickle_deadline.map(|deadline| deadline.saturating_duration_since(Instant::now())).unwrap_or(OPEN_TIMEOUT), async {
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
            run_gateway_channel(channel, gateway, access, credential, registration, supports_control).await;
        }
        diagnostic.stats(&_lease.0, "closing").await;
        diagnostic.emit("channel.close", json!({}));
    });
    Ok(answer)
}

fn apply_remote_updates(
    peer: &Arc<RTCPeerConnection>,
    mut updates: mpsc::Receiver<
        HostResult<Vec<webrtc::ice_transport::ice_candidate::RTCIceCandidateInit>>,
    >,
    path_policy: PathPolicy,
    diagnostic: Arc<Diagnostic>,
) {
    let weak = Arc::downgrade(&peer);
    let update_diag = diagnostic;
    tokio::spawn(async move {
        loop {
            let Some(peer) = weak.upgrade() else {
                break;
            };
            use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState as State;
            if matches!(
                peer.connection_state(),
                State::Connected | State::Closed | State::Failed
            ) {
                break;
            }
            let batch = match timeout(Duration::from_secs(1), updates.recv()).await {
                Err(_) => continue,
                Ok(None) => break,
                Ok(Some(Ok(batch))) => batch,
                Ok(Some(Err(_))) => {
                    update_diag.emit(
                        "ice.update.failed",
                        json!({"cause":"invalidOrUnavailableUpdate"}),
                    );
                    let _ = peer.close().await;
                    break;
                }
            };
            for candidate in batch {
                if !candidate.candidate.is_empty() && !path_policy.accepts(&candidate.candidate) {
                    continue;
                }
                if peer.add_ice_candidate(candidate).await.is_err() {
                    update_diag.emit("ice.update.failed", json!({"cause":"applyCandidate"}));
                }
            }
        }
    });
}

fn rtc_error(message: &str) -> HostError {
    HostError::new("remote_rtc_negotiation_failed", message).retryable(true)
}

#[cfg(test)]
mod trickle_tests {
    use super::*;
    #[tokio::test]
    async fn native_peer_accepts_late_candidates_after_an_empty_offer() {
        let api = APIBuilder::new().build();
        let client = Arc::new(
            api.new_peer_connection(RTCConfiguration::default())
                .await
                .unwrap(),
        );
        let host = Arc::new(
            api.new_peer_connection(RTCConfiguration::default())
                .await
                .unwrap(),
        );
        let _client_lease = PeerLease(client.clone());
        let _host_lease = PeerLease(host.clone());
        let options = || {
            Some(RTCDataChannelInit {
                negotiated: Some(0),
                ordered: Some(true),
                ..Default::default()
            })
        };
        let dc = client.create_data_channel("test", options()).await.unwrap();
        let remote = host.create_data_channel("test", options()).await.unwrap();
        let mut gathered = client.gathering_complete_promise().await;
        client
            .set_local_description(client.create_offer(None).await.unwrap())
            .await
            .unwrap();
        timeout(Duration::from_secs(5), gathered.recv())
            .await
            .unwrap();
        let offer = client.local_description().await.unwrap();
        let candidates: Vec<_> = offer
            .sdp
            .lines()
            .filter(|l| l.starts_with("a=candidate:"))
            .map(
                |l| webrtc::ice_transport::ice_candidate::RTCIceCandidateInit {
                    candidate: l.trim_start_matches("a=").into(),
                    sdp_mid: Some("0".into()),
                    sdp_mline_index: Some(0),
                    ..Default::default()
                },
            )
            .collect();
        assert!(!candidates.is_empty());
        let empty = offer
            .sdp
            .lines()
            .filter(|l| !l.starts_with("a=candidate:") && *l != "a=end-of-candidates")
            .collect::<Vec<_>>()
            .join("\r\n")
            + "\r\n";
        host.set_remote_description(RTCSessionDescription::offer(empty).unwrap())
            .await
            .unwrap();
        let (tx, rx) = mpsc::channel(4);
        apply_remote_updates(&host, rx, PathPolicy::Auto, Diagnostic::new(&offer.sdp));
        let mut ready = host.gathering_complete_promise().await;
        host.set_local_description(host.create_answer(None).await.unwrap())
            .await
            .unwrap();
        timeout(Duration::from_secs(5), ready.recv()).await.unwrap();
        // Inject after Host has prepared its answer, through the production worker.
        tx.send(Ok(candidates)).await.unwrap();
        client
            .set_remote_description(host.local_description().await.unwrap())
            .await
            .unwrap();
        timeout(Duration::from_secs(8), async {
            while dc.ready_state() != RTCDataChannelState::Open
                || remote.ready_state() != RTCDataChannelState::Open
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }
}

fn channel_adapter(
    dc: Arc<RTCDataChannel>,
    diagnostic: Arc<Diagnostic>,
    compression: bool,
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
            traffic: diagnostic.traffic.clone(),
            sink: Box::new(RtcSink(dc, diagnostic.clone(), compression)),
            source: Box::new(RtcSource {
                rx,
                closed: closed.clone(),
                decoder: {
                    let mut decoder = Decoder::new(REQUEST_BYTES);
                    decoder.allow_compression = compression;
                    decoder
                },
                diagnostic,
                frames: 0,
            }),
        },
        closed,
    )
}

struct RtcSink(Arc<RTCDataChannel>, Arc<Diagnostic>, bool);
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
                    if bytes.len() >= FRAME_BYTES {
                        self.1
                            .emit("message.send.start", json!({"bytes":bytes.len()}));
                    }
                    if bytes.is_empty() || bytes.len() > RESPONSE_BYTES {
                        return false;
                    }
                    let original_size = bytes.len();
                    let (bytes, compressed) = framing::compress_message(&bytes, self.2);
                    if compressed {
                        self.1.emit(
                            "message.compressed",
                            json!({"originalBytes":original_size,"wireBytes":bytes.len()}),
                        );
                    }
                    for (index, part) in bytes.chunks(FRAME_BYTES - HEADER_BYTES).enumerate() {
                        if !self
                            .fragment(framing::encode_fragment_compressed(
                                bytes.len(),
                                index * (FRAME_BYTES - HEADER_BYTES),
                                part,
                                compressed,
                            ))
                            .await
                        {
                            return false;
                        }
                        self.1.traffic.sent(part.len());
                    }
                    if bytes.len() >= FRAME_BYTES || started.elapsed() >= Duration::from_secs(1) {
                        self.1.emit(
                            "message.send.complete",
                            json!({"bytes":bytes.len(),"durationMs":started.elapsed().as_millis()}),
                        );
                    }
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
    diagnostic: Arc<Diagnostic>,
    frames: u64,
}
impl GatewaySource for RtcSource {
    fn recv(&mut self) -> ChannelFuture<'_, Option<GatewayFrame>> {
        Box::pin(async move {
            loop {
                if *self.closed.borrow() {
                    return None;
                }
                // The session heartbeat owns liveness. A bounded, incomplete
                // message must survive RPC deadlines and slow delivery.
                let bytes = tokio::select! {
                    _ = self.closed.changed() => return None,
                    bytes = self.rx.recv() => bytes?,
                };
                self.frames += 1;
                let decoded = self.decoder.push(&bytes);
                if decoded.is_ok() && !matches!(&decoded, Ok(Some(GatewayFrame::Close { .. }))) {
                    self.diagnostic.traffic.received(bytes.len() - HEADER_BYTES);
                }
                match decoded {
                    Ok(Some(message)) => {
                        let size = match &message {
                            GatewayFrame::Text(bytes) => bytes.len(),
                            _ => 0,
                        };
                        if self.frames > 1 {
                            self.diagnostic.emit(
                                "message.receive.complete",
                                json!({"bytes":size,"frames":self.frames}),
                            );
                        }
                        self.frames = 0;
                        return Some(message);
                    }
                    Ok(None) => {
                        if self.frames == 1 || self.frames % 64 == 0 {
                            self.diagnostic.emit("message.receive.progress",json!({"received":self.decoder.received_bytes(),"total":self.decoder.total_bytes(),"frames":self.frames}));
                        }
                    }
                    Err(()) => return Some(GatewayFrame::Invalid),
                }
            }
        })
    }
}

#[cfg(test)]
mod source_tests {
    use super::*;

    #[tokio::test]
    async fn slow_partial_message_survives_until_completed_or_connection_closed() {
        let (tx, rx) = mpsc::channel(4);
        let (closed_tx, closed) = watch::channel(false);
        let mut source = RtcSource {
            rx,
            closed,
            decoder: Decoder::new(REQUEST_BYTES),
            diagnostic: Diagnostic::new("slow-source-test"),
            frames: 0,
        };
        tx.send(encode_fragment(4, 0, b"ab").into()).await.unwrap();
        // Cancelling recv (as the session select does) must retain framing state.
        assert!(timeout(Duration::from_millis(5100), source.recv())
            .await
            .is_err());
        assert_eq!(source.diagnostic.traffic.snapshot().received_bytes, 2);
        tx.send(encode_fragment(4, 2, b"cd").into()).await.unwrap();
        assert!(matches!(source.recv().await, Some(GatewayFrame::Text(value)) if value == b"abcd"));
        assert_eq!(source.diagnostic.traffic.snapshot().received_bytes, 4);
        tx.send(encode_fragment(4, 0, b"ab").into()).await.unwrap();
        assert!(timeout(Duration::from_millis(10), source.recv())
            .await
            .is_err());
        closed_tx.send(true).unwrap();
        assert!(source.recv().await.is_none());
    }
}
