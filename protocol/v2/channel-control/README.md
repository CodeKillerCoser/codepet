# Transport ownership control v1

This is a transport-independent channel contract. It contains no Gateway business
methods, Provider routes, credentials or conversation data. `messages.schema.json` is the
source of `ControlPrepare`, `ControlActivate` and `ControlStatus` in the generated
Rust/Dart SDKs. The envelope/framing contract below belongs to the channel adapter.

Capability negotiation precedes any control frame:

- WSS: client offers `Sec-WebSocket-Protocol: codepet-control-v1`; Host selects it.
- RTC: offer and answer both contain `a=x-codepet-channel-control:1`, under the
  existing authenticated signaling/peer binding.
- Absence means legacy mode. No unknown control frames are sent. A legacy active
  path cannot be upgraded while healthy; ordinary failure recovery still works.

The canonical compact JSON envelope starts with `{"channelControl":1,` (first
member, no whitespace before the next comma). This permits bounded control-frame
recognition without decoding large Gateway payloads twice. Maximum frame size is
2048 bytes; correlation IDs are nonempty strings up to 128 bytes. Request fields
are `channelControl`, `jsonrpc:"2.0"`, `id`, `operation` and `params`. Response
fields are the discriminator, JSON-RPC version, `id`, and either typed `result`
(`ControlStatus`) or `error:{code:"channel_control_conflict"}`. A control envelope
never enters the Gateway generated dispatcher or Gateway SDK decoder.

1. `prepare` with `ControlPrepare`, before Gateway handshake: binds a fresh
   connection to an authenticated credential plus a logical session ID. Host
   issues a unique connection token. Prepared connections do not appear online
   and cannot execute business requests.
2. Ordinary Gateway handshake validates the peer's protocol and supplies business
   metadata. Prepared connections may send protocol heartbeat afterward.
3. `status` with empty params returns that scope's authoritative active token.
4. `activate` with `ControlActivate` compares `expectedActive` (omitted means none)
   and commits this connection. `switchId` identifies the switch; retrying the
   latest committed switch for the same connection is idempotent. Query status
   after a lost response. Never infer a rollback from a timeout.

Host checks ownership at both request admission and dispatch from its bounded
queue. Already accepted work is not replayed or revoked by changing ownership.
The client pauses new calls and bounds healthy drain to one second; if old calls
have not drained it defers the upgrade. Old traffic/events stop being authoritative
after commit. The new connection resumes the opaque event cursor before completing
the handover; a replay gap requires a business snapshot reset.

Scopes are independent for distinct logical sessions/credentials; closing an old
socket cannot delete the new owner. Empty groups are removed. Host restart creates
fresh tokens; clients query status again. Confirmation that cannot be resolved
fences the local session and reports an explicit failure instead of sending writes
to two possibly active connections.

Verification: `npm run protocol:check`, Host ownership unit tests, and
`cargo test --manifest-path crates/Cargo.toml -p codepet-host --test remote_lan_listener negotiated_channel_handover`.
