# CodePet protocol IDL

## Single source of truth

Everything under `protocol/` is language-neutral, handwritten protocol input. Generated Rust and TypeScript files are outputs only; they must not be edited as an alternative model source.

The v1 layers are:

- `core/v1` — stable IDs, timestamps, versions, pagination, errors, JSON primitives, and four-part routed resource identity (`deviceId + providerPluginId + providerInstanceId + nativeResourceId`).
- `pet/v1` — Desktop Companion-driven `PetTask`, `PetApproval`, `PetAction`, snapshot, and patch contracts. It does not reference Provider conversation, turn, or approval models.
- `provider/v1` — public Host ↔ independent Provider binary JSON-RPC 2.0 over newline-delimited stdio. It owns initialize, describe, instance lifecycle/capability, conversation, turn, approval, event, and shutdown contracts.
- `gateway/v1` — Host ↔ Remote Client methods and replayable events. Resources use `deviceId + providerPluginId + providerInstanceId + nativeResourceId`; the gateway exposes neither plugin process lifecycle nor pet-private state.

`codegen.json` declares packages, dependency direction, output targets, and the shared `codepet.protocol.codegen/v1` adapter interface for Rust, TypeScript, Dart, and Python. Rust is active for all four v1 layers. TypeScript covers core, Gateway v1, and the Runtime Gateway compatibility surface. Dart and Python remain explicitly registered but unimplemented planned targets; selecting them fails closed before generation.

## Versioning and discriminators

Every public v1 initialize/handshake request carries an explicit supported `VersionRange`, and the response selects one `ProtocolVersion`. Method and event names live in each layer's manifest rather than Rust code.

- Pet and gateway use CodePet envelopes discriminated by `method` and `event`.
- Provider uses JSON-RPC 2.0 requests discriminated by `method`, strict result/error responses, and notification events also discriminated by `method`. Its generated `JsonLineCodec` enforces a caller-selected frame limit and classifies inbound request, response, notification, and declared event messages.
- Gateway v1 events carry an opaque `eventCursor` for replay.
- Union-like domain DTOs such as `PetAction` retain an explicit `kind`; receivers validate kind-specific optional fields.

The generator supports a deliberately small JSON Schema Draft 2020-12 subset. Unsupported keywords, unresolved references, duplicate method/event names, invalid fixtures, or undeclared cross-layer dependencies fail generation.

## Gateway v1 LAN generation contract

The LAN transport uses three fixed routes without adding them as CodePet-envelope methods:

- `POST /remote/v1/pairings/{pairingId}/exchange`
- `GET /remote/v1/gateway` with WebSocket upgrade
- `DELETE /remote/v1/credentials/current`

Their bodies are generated from the same `gateway/v1/schema.json` as the Gateway envelopes. `PairingExchangeRequest` contains `pairingSecret`, the existing handshake `clientId`, `clientName`, and `platform`; `pairingId` remains the REST path parameter. `PairingExchangeResponse` contains the stable `device`, `gatewayUrl`, and opaque `credential`. `CurrentCredentialDeleteResponse` is the minimal current-credential revocation result. These types are not JSON-RPC or Gateway envelope methods.

`PairingQrPayload` is the only encodable QR wire payload. Its fields are exactly `version`, `hostDeviceId`, `displayName`, `httpsBaseUrl`, `certSha256`, `pairingId`, `pairingSecret`, and `expiresAt`. The plaintext pairing secret may flow from Host memory into the QR encoder, but must not be rendered as ordinary UI text or logged. Pairing display state and countdown values remain Host/UI implementation state rather than remote schema fields.

DNS-SD advertises `_codepet._tcp.local.` and its TXT record is limited to `id`, `name`, `vmin`, `vmax`, and `pair`. Discovery supplies an endpoint only; it does not establish trust and must never publish a certificate fingerprint, pairing secret, credential, Provider, project, or conversation data.

The WSS upgrade authenticates the opaque bearer in the future LAN listener. The first business request is `protocol.handshake`, and its existing `clientId` must equal the client identity bound to that credential. `HandshakeResponse.device` is a required `RemoteHostIdentity { deviceId, displayName, identityFingerprint }`; `identityFingerprint` and QR `certSha256` are the 64-character lowercase hexadecimal SHA-256 of the leaf certificate DER. A client verifies the actual TLS peer certificate and then checks the handshake device identity against the paired identity. `ProviderGatewayService` receives only this transport-neutral host identity and never receives or validates bearer credentials.

## Gateway v1 snapshot and live-event boundary

`EventCursor` is an opaque replay token. A client may persist it, compare it for equality, and return it to the Gateway, but must never parse, order, or increment it. In particular, clients must pass the exact last applied cursor as `event.subscribe.afterCursor`; they must not calculate a numeric `+1`. The Gateway acknowledges that exact boundary as `subscribedAfterCursor`, then the transport session delivers events after it. Whether one WebSocket may call `event.subscribe` more than once is a future transport-session policy, not part of the v1 IDL or Host service contract.

`conversation.list` and `conversation.get` return `snapshotCursor`. The Host captures this cursor immediately before issuing the corresponding Provider query, never after the query completes. A client can therefore obtain the snapshot and subscribe after its returned cursor without losing events that arrived while the Provider query was in flight.

Committed snapshot content and live output deltas have separate ownership. Gateway v1 `conversation.get` does not return in-progress response body text that may still be changed by `turn.outputDelta`; the current `Conversation` and `TurnTask` DTOs carry metadata and stable summary/state only. Live `turn.outputDelta` events exclusively carry the ongoing body stream. This phase does not add an authoritative body projection or revision-delta model. Metadata upserts remain idempotent and may restate state already visible in the snapshot.

## Generated SDKs

Rust packages are located at:

- `sdk/rust/codepet-core-sdk`
- `sdk/rust/codepet-pet-sdk`
- `sdk/rust/codepet-provider-sdk`
- `sdk/rust/codepet-gateway-sdk`

Service SDKs contain serde DTOs, method/event enums, async server traits, dispatchers, typed client/transport shells, wire envelopes, and codecs. Provider additionally generates `ProtocolRequest::from_method_params`, so a transport can turn the generated method plus typed-client params into the exact JSON-RPC request enum without maintaining a second method/envelope match. Provider/Gateway capability enums and `ProtocolMethod::capability()` are generated from checked manifest/schema metadata. Provider descriptors expose protocol validation for non-empty supported instance kinds, and `instance.create` plus returned instances carry the selected `instanceKind`. The generated packages remain transport contracts rather than business runtimes. The consuming implementation is now `crates/codepet-host`: it owns device/instance persistence, Provider process supervision, Plugin Manager behavior, and an internal Gateway v1 service without copying protocol DTOs back into the host.

The four Rust SDK manifests are packageable crates rather than permanently private workspace crates. Their local path dependencies also declare version `0.1.0`, allowing local workspace development while preserving a publishable dependency graph. Repository tests are excluded from crate tarballs because they read the canonical fixtures outside each crate under `protocol/`; the tests still run from the SDK workspace, while published source remains self-contained without copying fixture facts. The Provider SDK is consumed by the standalone `codepet-provider-codex` and `codepet-provider-claude` binaries; their all-target dependency graphs do not include Host or Gateway. The repository currently has no license file; `cargo package` content checks can run, but the project should not publish until the repository owner makes an explicit license decision.

## Runtime Gateway compatibility

The existing in-process Runtime Gateway and Desktop Companion still use the unchanged v0 wire profile while v1 is introduced. That profile now lives at `gateway/v1/compat-v0.*` and is generated into `codepet-gateway-sdk::compat_v0` plus the TypeScript compatibility SDK. The Tauri and frontend files named `generated` are thin re-export shims only.

This compatibility path preserves current dual-channel behavior: remote App Server operations run only through Provider v1 and `codepet-host`, then map to the existing remote v0 Tauri surface; Desktop IPC remains on the companion bus. The compat layer is stateless, uses resource-carried four-part identity, and never forwards Provider events into companion/Pet channels.

## Commands

```sh
npm run protocol:generate
npm run protocol:check
cargo test --manifest-path sdk/rust/Cargo.toml
npm test --prefix sdk/typescript/codepet-gateway-sdk
node tools/protocol-codegen/generate.mjs --target=rust --check
```

`protocol:check` validates schema/manifest consistency, fixtures, schema and manifest dependency direction, capability mappings, target fail-closed behavior, and generated-file freshness. Runtime compatibility is covered by `runtime_gateway_protocol_tests` and `runtime_gateway_core_tests` in the Tauri crate.

## Current limits

- A reusable Plugin Manager and process supervisor exists in `crates/codepet-host`, and Codex remote operations run through the standalone Provider binary. Signature, marketplace, sandbox, and automatic restart policy remain deliberately out of scope.
- Gateway v1 now defines the LAN identity, QR, pairing REST bodies, and current-credential delete response, while `RemoteAccessManager` supplies the unconnected TLS/pairing/credential core. No LAN listener, HTTP/WSS route, mDNS publisher, remote UI wiring, or persistent event-cursor store exists yet.
- The v0 compatibility profile remains in use by the desktop process until a later phase wires gateway v1 sessions and a separate pet-protocol adapter.
