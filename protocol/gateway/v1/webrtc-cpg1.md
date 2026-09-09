# Gateway WebRTC CPG1 channel profile

Status: implemented as an opt-in LAN-signaled RTC adapter. The existing
`websocket-text` profile remains the default. This profile changes transport
framing only; Gateway JSON-RPC methods, DTOs, request IDs, trace metadata and
event cursors retain their existing definitions.

## Admission and negotiation

An already paired client POSTs a standard `{type: "offer", sdp: "..."}` to
`https://<paired-host>/remote/v1/webrtc/offer`, using the retained TLS leaf
certificate pin and bearer credential. The response is `{type: "answer", sdp: "..."}`.
The pin authenticates the answer and its ephemeral DTLS fingerprint; the bearer
authenticates the offer's client. SDP/credentials must not be logged.

One application media section is accepted; no audio or video. The existing
64 KiB REST body limit applies and the decoded SDP is limited to 60 KiB.
Candidates are gathered into each SDP (no trickle in this bootstrap version).
The Host has an 8 second negotiation deadline and a 15 second channel-open
deadline. Remote applies a 25 second total connect deadline.

Both peers create one **negotiated, reliable, ordered** DataChannel:
ID 0, label `codepet.gateway.v1`, protocol `codepet.gateway.cpg1`.
No retransmit/lifetime limit. Remotely opened DCEP channels are closed.
A PeerConnection represents one shared Gateway session and credential slot.
The first complete JSON message must still be `protocol.handshake` with the
credential's clientId. TLS signaling is not the business data path.

The bootstrap has no STUN/TURN servers and still needs a reachable LAN signaling
endpoint. Public rendezvous, TURN credentials and ICE restart are separate
delivery work; do not describe this version as Internet-ready.

## Wire format

Each native DataChannel message is binary, maximum **16,384 bytes including
header**. Integers are unsigned, big-endian.

| Offset | Bytes | Meaning |
| --- | --- | --- |
| 0 | 4 | ASCII `CPG1` |
| 4 | 4 | Total complete JSON byte length; zero means close control |
| 8 | 4 | Byte offset of this fragment |
| 12 | 1–16,372 | Payload |

Messages cannot interleave. Offset starts at zero and must equal accumulated
payload length; total must remain equal through the message. Empty fragments,
overruns, gaps, duplicate offsets, changed total, invalid UTF-8/JSON and text
native messages fail the connection. A partial message has a fixed 5 second
assembly deadline, not extended by additional fragments.

Complete JSON limits are 256 KiB Remote→Host and 4 MiB Host→Remote. Responses are
assembled before being given to the unchanged Gateway client. The RTC response
cap is a known limit; callers must continue existing pagination. No compression,
files, stream multiplexing or yamux is part of this profile.

Close control uses total=0, offset=0, payload=u16 code followed by UTF-8 reason,
maximum 125 payload bytes. It may interrupt a partial message. Codes preserve
the existing Gateway close semantics (e.g. 1008 credential revocation, 1001
shutdown/heartbeat timeout, 1013 congestion). A transport failure may prevent
delivery of close control; cleanup must also handle native channel/peer closure.

Golden vector for a complete `{}` JSON message:
`43504731 00000002 00000000 7b7d`.

## Flow control and lifecycle

Both senders serialize fragments and stop when native buffered bytes exceed
64 KiB, resuming below or at 16 KiB. They poll native buffer state at 10 ms while
blocked; each send is bounded by its existing 15 second operation deadline.
Host raw ingress has a 32-fragment queue and fails on overflow. Remote caps
pending requests at 64 and queued encoded bytes at 2 MiB; it does not queue
unlimited calls while a peer is blocked.

Host retains the shared session limits, dispatch concurrency and event
subscription implementation. Credential revocation and server shutdown cancel
both LAN and RTC sessions. Remote preserves outcomeUnknown for an interrupted
write; no automatic business request replay. Reconnection creates a fresh peer
and reuses the application's existing session generation/cursor behavior.

## Verification

- Rust framing tests: Unicode across boundaries, invalid lengths/offsets,
  interleaving and bounded close control.
- Real Rust peer integration: pinned HTTPS admission, malformed offer cleanup,
  Gateway handshake, >64 KiB bidirectional message and simultaneous LAN/RTC
  credential revocation.
- Flutter adapter tests: fragmentation, native backpressure, out-of-order
  responses, timeout/close cleanup, late peer creation and malformed frames.
- Native Android probe: `integration_test/rtc_gateway_test.dart` in Remote,
  using the ignored Host `rtc_android_probe_host` fixture and ephemeral config.
  Actual execution results belong in the delivery record.
