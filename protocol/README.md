# CodePet protocol IDL

## Single source of truth

Everything under `protocol/` is language-neutral, handwritten protocol input. Generated Rust, TypeScript, and Dart files are outputs only; they must not be edited as an alternative model source.

The current layers are:

- `core/v1` — stable IDs, timestamps, versions, pagination, errors, JSON primitives, and opaque client resource identity (`providerId + nativeResourceId`).
- `agent/v1` — types-only shared Agent domain: projects, conversations, turns, approvals, conversation items, tool/content payloads, selections, authentication, and usage. It declares no methods or transport.
- `pet/v1` — Desktop Companion-driven `PetTask`, `PetApproval`, `PetAction`, snapshot, and patch contracts. It does not reference Provider conversation, turn, or approval models.
- `provider/v1` — public Host ↔ independent Provider binary JSON-RPC 2.0 over newline-delimited stdio. It owns initialize, describe, instance lifecycle/capability, business method/event envelopes, Provider-private four-part `ProviderResourceId`, and shutdown; client-visible business objects come from `agent/v1`.
- `gateway/v1` — Host ↔ Remote Client JSON-RPC 2.0 methods and replayable notifications over a channel-provided WebSocket. Resources use opaque `providerId + nativeResourceId`; the gateway exposes neither plugin process lifecycle nor pet-private state.
- `channel/lan/v1` — LAN admission DTOs for QR pairing, TLS identity and credential revocation. It declares no Gateway business methods.

The legacy Gateway protocol has been removed. Host/Remote integration has one Gateway business protocol: Gateway v1. Discovery, channel establishment, trust admission and Gateway business RPC remain separate runtime layers.

`codegen.json` declares packages, dependency direction, output targets, and the shared `codepet.protocol.codegen/v1` adapter interface. Agent depends only on Core; Provider and Gateway depend on Core plus Agent. Rust is active for Core, Agent, Pet, Provider, Gateway v1, LAN admission and the desktop-only v0 contract. TypeScript covers Core, Agent, Provider, Gateway v1 and the desktop-only v0 contract. Dart is active for Core, Agent, Gateway v1 and LAN admission. Python remains explicitly registered but unimplemented; selecting it fails closed before generation.

## Versioning and discriminators

Every public initialize/handshake request carries an explicit supported `VersionRange`, and the response selects one `ProtocolVersion`. Method and event names live in each layer's manifest rather than language-specific code.

- Pet and the desktop-only v0 contract use CodePet envelopes discriminated by `method` and `event`; Gateway v1 uses standard JSON-RPC 2.0 requests/responses and event notifications.
- Provider uses JSON-RPC 2.0 requests discriminated by `method`, strict result/error responses, and notification events also discriminated by `method`. Its generated `JsonLineCodec` enforces a caller-selected frame limit and classifies inbound request, response, notification, and declared event messages.
- Gateway v1 events carry an opaque `eventCursor` for replay.
- Domain unions use a required singleton `kind` on every closed `oneOf` variant. The generator rejects ambiguous or open variants before emitting Rust, Dart, or TypeScript models. Agent conversation items, tool inputs, tool outcomes, and content blocks use this shape so Provider and Gateway share one canonical definition and each full payload has one canonical owner.

The generator supports a deliberately small JSON Schema Draft 2020-12 subset. Unsupported keywords, unresolved references, duplicate method/event names, invalid fixtures, or undeclared cross-layer dependencies fail generation.

## LAN admission v1 and Gateway v1 contract

The LAN transport uses three fixed routes without adding them as CodePet-envelope methods:

- `POST /remote/v1/pairings/{pairingId}/exchange`
- `GET /remote/v1/gateway` with WebSocket upgrade
- `DELETE /remote/v1/credentials/current`

REST/QR bodies are generated from `channel/lan/v1/schema.json`; Gateway business DTOs and methods are generated from `gateway/v1`. `PairingExchangeRequest` contains `pairingSecret`, the existing stable `clientId`, and one required `DeviceDescriptor { deviceName, operatingSystem, systemVersion }`; `pairingId` remains the REST path parameter. `PairingExchangeResponse` contains the stable LAN Host identity, Gateway URL, and opaque credential. These admission types are not JSON-RPC methods.

`PairingQrPayload` is the only encodable QR wire payload. Its fields are exactly `version`, `hostDeviceId`, `displayName`, `httpsBaseUrl`, `certSha256`, `pairingId`, `pairingSecret`, and `expiresAt`. The plaintext pairing secret may flow from Host memory into the QR encoder, but must not be rendered as ordinary UI text or logged. Pairing display state and countdown values remain Host/UI implementation state rather than remote schema fields.

DNS-SD advertises `_codepet._tcp.local.` and its TXT record is limited to `id`, `name`, `vmin`, `vmax`, and `pair`. `pair` is `1` only while the Host actively offers one-time pairing and is otherwise `0`. Identity, explicit IP, and actual TLS port all come from the same validated listener handle; the IP must belong to a local interface, and Host start/update succeeds only after the active daemon generation reports an announcement for the target fullname. A changed `pair` value unregisters the old generation before starting the new one, so discovery has a short non-atomic gap; an unchanged value is a no-op. Discovery does not establish trust and must never publish a certificate fingerprint, pairing secret, credential, Provider, project, or conversation data.

The WSS upgrade authenticates the opaque bearer. The first business request is `protocol.handshake`; its `clientId` must equal the client identity bound to that credential. `identityFingerprint` and QR `certSha256` belong only to LAN channel/admission and are checked against the actual leaf certificate DER. Gateway `HandshakeResponse.device` contains only `deviceId + descriptor`; `ProviderGatewayService` never receives a bearer, pairing secret or certificate fingerprint.

## Gateway v1 snapshot and live-event boundary

`EventCursor` is an opaque replay token. A client may persist it, compare it for equality, and return it to the Gateway, but must never parse, order, or increment it. In particular, clients must pass the exact last applied cursor as `event.subscribe.afterCursor`; they must not calculate a numeric `+1`. The Gateway acknowledges that exact boundary as `subscribedAfterCursor`, then the transport session delivers events after it. Whether one WebSocket may call `event.subscribe` more than once is a future transport-session policy, not part of the v1 IDL or Host service contract.

`conversation.list`, `conversation.search`, and `conversation.get` return `snapshotCursor`. The Host captures this cursor immediately before issuing the corresponding Provider query, never after the query completes. A client can therefore obtain the snapshot and subscribe after its returned cursor without losing events that arrived while the Provider query was in flight. `conversation.search` is a separate, route-scoped Provider query with its own cursor; clients must not merge search pages into the recent `conversation.list` cursor stream.

`conversation.get.items` is the ordered, provider-neutral committed history projection. Each item carries its routed native item identity, turn, and owning conversation; the Host rejects any item whose conversation does not exactly match the requested routed conversation. Each content block has a deterministic `contentId` derived from the native item ID plus its stable semantic position. `turn.outputDelta` carries the same `itemId/contentId/kind`, so clients first install the committed snapshot, then replay after `snapshotCursor`, and upsert or discard replayed deltas whose content ID is already committed instead of appending duplicate text. In-progress mutable bodies stay live-only until their authoritative item completes. Metadata upserts remain idempotent and may restate state already visible in the snapshot.

The common history surface covers user and assistant messages, readable reasoning summaries, command/file/tool activity, and approval records actually observed by a Provider. It never exposes a native Provider DTO. Raw reasoning content is excluded, unknown native items become safe `unknown` activity without raw payload, and command completion must not be used to invent an approval that was never requested.

Conversation history uses closed, `kind`-discriminated item variants rather than one object with mutually incompatible optional fields. A message owns message content; a reasoning item owns readable summaries; a tool/command item owns one typed invocation; file-change, approval, and unknown activity expose only their branch-specific fields. Unknown activity never carries a native Provider payload. UI placement such as inline expansion, a drawer, or a detail page is not a protocol concern.

A tool invocation has exactly one input variant: `command` for a semantic command execution, `structured` for reliably parsed JSON parameters, or `opaque` for input whose semantics cannot be understood reliably. Size is orthogonal to this classification. Its terminal outcome is a `success` or `failure` variant and owns the only canonical result content blocks; full command/output text is not copied into item contents, a command facet, structured result, or error message. Failure messages are bounded summaries, while stdout, stderr, diagnostics, diffs, and JSON results remain typed content blocks. Provider-only extensions do not exist on shared Agent business objects. Provider request/instance/capability extensions remain private; Host validates and forwards Agent objects without field-by-field copying.

Content blocks carry stable content identity and structured truncation metadata when incomplete, including original and retained byte counts plus the truncation strategy. Command, structured, and opaque tool inputs use the same truncation metadata, so semantic input kind remains independent from payload size. Absence of that metadata means complete content. The first content/page budget is applied before the Provider serializes its stdout response, because the Provider-to-Host 16 MiB JSON-line is the earliest hard boundary; Host/Gateway may apply an additional downstream budget but cannot repair an already oversized Provider frame. `conversation.itemUpserted` replaces the same routed item as it moves from pending/running to a terminal result, while `turn.outputDelta` remains the append-only text channel. These contract corrections remain in Provider/Gateway v1 because there is no deployed compatibility boundary requiring a v2 dual stack.

## Provider descriptors and existing-conversation turns

Every Provider instance exposes two separate identities: adapter metadata (`pluginId`, `displayName`, and adapter `version`) and a required `harness { id, displayName, version? }` supplied by the Provider runtime. Gateway maps this descriptor as data and never branches on a harness name. Capabilities carry a required opaque `revision`; a client must send that exact value with `turn.send`, and a changed runtime/catalog revision makes an older selection stale.

`turnSend` contains only controls that the current Provider can honestly execute. Access mode and reasoning effort are option sets with stable IDs, user-facing labels, optional disabled state, and optional defaults. A model catalog is an explicit discriminated union: flat catalogs and selections use `kind: "flat"`, while grouped catalogs and selections use `kind: "grouped"` plus `providerId`. Remote clients render only advertised controls and return the selected shape unchanged; they must not infer a catalog shape from fields or from the harness descriptor.

Gateway `turn.send` starts a new turn in an existing, idle conversation. Its Provider route is derived from the routed conversation, avoiding two independently supplied route values. The request also contains `clientRequestId`, capability revision, typed text input, and a complete selection object. Gateway validates route ownership, instance readiness, advertised method, revision, option membership/enabled state, and catalog/selection kind before calling Provider `turn.start`.

Duplicate suppression is deliberately bounded to one running `ProviderGatewayService`: it retains the latest 1,024 `(callerScope, route, clientRequestId)` outcomes in memory. WSS derives the opaque scope from the authenticated logical `clientId`, while local compatibility callers use their own stable scope; bearer material is never part of this key. Within one scope and cache window, an identical replay returns the cached success or error without sending another Provider request, while reusing the same key with different payload is `client_request_conflict`. `turn.send` remains `nonIdempotent`: Host restart, eviction, or an unknown transport outcome can remove this protection. A Remote client must not promise an automatic safe retry; it should refresh the conversation and live state first. An explicit same-ID retry is only best-effort deduplication within the current process/cache window, never exactly-once execution.

The current advertised execution matrix is intentionally conservative:

- Codex discovers its live flat model catalog through official `model/list`, publishes only the reasoning efforts common to every advertised model, includes the App Server version from initialize, advertises `turn.start`/Gateway `turn.send`, and applies selected access mode, model, and reasoning effort to official `turn/start`. Its immediate start acknowledgement may return `userItem: null`; item notifications and later snapshots provide the authoritative history.
- OpenCode 1.18.25 retains its existing Provider-internal prompt/control implementation, but does not advertise `turn.start` or Gateway `turn.send` because a complete user-selectable model/reasoning discovery contract is not yet established.
- Claude Code retains its direct CLI execution implementation, but does not advertise `turn.start` or Gateway `turn.send`; its CLI stream surface has no App Server-equivalent discovery/control contract that can honestly populate these selectors.

## Generated SDKs

Rust packages are located at:

- `sdk/rust/codepet-core-sdk`
- `sdk/rust/codepet-desktop-sdk`
- `sdk/rust/codepet-pet-sdk`
- `sdk/rust/codepet-provider-sdk`
- `sdk/rust/codepet-gateway-sdk`
- `sdk/rust/codepet-lan-channel-sdk`

Dart packages are located at:

- `sdk/dart/codepet-core-sdk`
- `sdk/dart/codepet-agent-sdk`
- `sdk/dart/codepet-gateway-sdk`
- `sdk/dart/codepet-lan-channel-sdk`

The Dart Gateway package is pure null-safe Dart and re-exports Core plus Agent. It contains immutable generated DTOs, strict unknown-field and schema-constraint validation, `kind`-discriminated sealed unions, secret-redacted diagnostics, manifest-derived method/event metadata, JSON-RPC 2.0 codecs, and a transport-neutral typed `ProtocolClient`. WebSocket framing, request correlation storage, TLS pinning, credentials, reconnect/retry policy, and event persistence remain consumer-runtime concerns.

Service SDKs contain serde DTOs, method/event enums, async server traits, dispatchers, typed client/transport shells, wire envelopes, and codecs. Provider additionally generates `ProtocolRequest::from_method_params`, request method/id accessors, and `ProtocolMethod::dispatch_lane()`, so Host and Provider runtimes do not maintain second method/envelope/control-lane tables. Provider/Gateway capability enums and `ProtocolMethod::capability()` are generated from checked manifest/schema metadata. Provider descriptors expose protocol validation for non-empty supported instance kinds, and `instance.create` plus returned instances carry the selected `instanceKind`.

The Rust Provider SDK also exposes a stable, handwritten `serve_stdio` runtime around the generated contract. It owns bounded JSON-lines, concurrent normal/control dispatch, typed event publication, serialized stdout, overload responses, terminal I/O handling, shutdown and drain; it does not own Provider business state or Host process supervision. `crates/codepet-host` remains the process and routing owner, while Provider binaries implement only the generated server trait plus harness-specific behavior.

`cp-sdk-gen --package <provider|gateway|lan-channel> --role <client|server|both|models> --lang <rust|dart> --output <sdk-dir>` reads canonical adjacent schema/manifest/fixtures, rebuilds normalized typed IR, and writes a standalone SDK plus protocol digest lock. Provider and Gateway exports recursively include Core and Agent; LAN admission includes Core only. Provider supports Rust server; Gateway supports Dart client and Rust client/server/both; LAN admission supports Dart/Rust models. The Bun-compiled native executable and canonical `protocol/{core,agent,provider,gateway,channel}` tree ship under App `provider-sdk/` resources.

The six Rust SDK manifests are packageable crates rather than permanently private workspace crates. Their local path dependencies also declare version `0.1.0`, allowing local workspace development while preserving a publishable dependency graph. Repository tests are excluded from crate tarballs because they read the canonical fixtures outside each crate under `protocol/`; the tests still run from the SDK workspace, while published source remains self-contained without copying fixture facts. The Provider SDK runtime is consumed by the standalone Codex, Claude, and OpenCode Provider binaries; their all-target dependency graphs do not include Host or Gateway. The repository currently has no license file; `cargo package` content checks can run, but the project should not publish until the repository owner makes an explicit license decision.

## Runtime Gateway compatibility

The existing in-process Runtime Gateway and Desktop Companion still use the unchanged desktop-only v0 wire profile. That profile lives at `desktop/v0` and is generated into `codepet-desktop-sdk` plus the TypeScript desktop SDK. It is not a Gateway version and is independent from the production Remote path, which uses Gateway v1 JSON-RPC. The Tauri and frontend files named `generated` are thin re-export shims only.

This compatibility path preserves current dual-channel behavior: remote App Server operations run only through Provider v1 and `codepet-host`, then map to the existing remote v0 Tauri surface; Desktop IPC remains on the companion bus. The compat layer is stateless, uses resource-carried four-part identity, and never forwards Provider events into companion/Pet channels.

## Commands

```sh
npm run protocol:generate
npm run protocol:check
cargo test --manifest-path sdk/rust/Cargo.toml
npm test --prefix sdk/typescript/codepet-desktop-sdk
dart analyze sdk/dart/codepet-core-sdk sdk/dart/codepet-gateway-sdk sdk/dart/codepet-lan-channel-sdk
# Run `dart test` with sdk/dart/codepet-gateway-sdk as the working directory.
node tools/protocol-codegen/generate.mjs --target=rust --check
npm run cp-sdk-gen -- --package provider --role server --lang rust --output /tmp/codepet-provider-sdk
python3 scripts/test_codex_provider_stdio.py --provider crates/target/debug/codepet-provider-codex --app-server crates/target/debug/codex-app-server-fixture
```

`protocol:check` validates schema/manifest consistency, positive and negative fixtures, schema and manifest dependency direction, capability mappings, target fail-closed behavior, and generated-file freshness. Runtime compatibility is covered by `runtime_gateway_protocol_tests` and `runtime_gateway_core_tests` in the Tauri crate.

## Current limits

- A reusable Plugin Manager and process supervisor exists in `crates/codepet-host`, and Codex remote operations run through the standalone Provider binary. Signature, marketplace, sandbox, and automatic restart policy remain deliberately out of scope.
- LAN admission v1 defines identity, QR, pairing REST bodies and credential revocation; `codepet-host` owns the TLS/HTTP/WSS listener and mDNS lifecycle. The production Remote business session uses Gateway v1 JSON-RPC after admission. A persistent cross-process event-cursor store remains absent.
- The v0 compatibility profile remains in use only by the desktop process until a separate Pet-protocol migration; it is not the Remote network protocol.
- `turn.send` is non-idempotent. Same-ID duplicate suppression is caller-scoped, process-local, and bounded; no persistent exactly-once ledger exists.
