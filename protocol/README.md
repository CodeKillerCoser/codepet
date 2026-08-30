# CodePet protocol IDL

## Single source of truth

Everything under `protocol/` is language-neutral, handwritten protocol input. Generated Rust and TypeScript files are outputs only; they must not be edited as an alternative model source.

The v1 layers are:

- `core/v1` — stable IDs, timestamps, versions, pagination, errors, JSON primitives, and routed resource identity.
- `pet/v1` — Desktop Companion-driven `PetTask`, `PetApproval`, `PetAction`, snapshot, and patch contracts. It does not reference Provider conversation, turn, or approval models.
- `provider/v1` — public Host ↔ independent Provider binary JSON-RPC 2.0 over newline-delimited stdio. It owns initialize, describe, instance lifecycle/capability, conversation, turn, approval, event, and shutdown contracts.
- `gateway/v1` — Host ↔ Remote Client methods and replayable events. Resources use `deviceId + providerInstanceId + nativeResourceId`; the gateway exposes neither plugin process lifecycle nor pet-private state.

`codegen.json` declares packages, dependency direction, output targets, and the shared `codepet.protocol.codegen/v1` adapter interface for Rust, TypeScript, Dart, and Python. Rust is active for all four v1 layers. TypeScript currently covers core plus the Runtime Gateway compatibility surface. Dart and Python remain explicitly registered but unimplemented planned targets; selecting them fails closed before generation.

## Versioning and discriminators

Every public v1 initialize/handshake request carries an explicit supported `VersionRange`, and the response selects one `ProtocolVersion`. Method and event names live in each layer's manifest rather than Rust code.

- Pet and gateway use CodePet envelopes discriminated by `method` and `event`.
- Provider uses JSON-RPC 2.0 requests discriminated by `method`, strict result/error responses, and notification events also discriminated by `method`. Its generated `JsonLineCodec` enforces a caller-selected frame limit and classifies inbound request, response, notification, and declared event messages.
- Gateway v1 events carry an opaque `eventCursor` for replay.
- Union-like domain DTOs such as `PetAction` retain an explicit `kind`; receivers validate kind-specific optional fields.

The generator supports a deliberately small JSON Schema Draft 2020-12 subset. Unsupported keywords, unresolved references, duplicate method/event names, invalid fixtures, or undeclared cross-layer dependencies fail generation.

## Generated SDKs

Rust packages are located at:

- `sdk/rust/codepet-core-sdk`
- `sdk/rust/codepet-pet-sdk`
- `sdk/rust/codepet-provider-sdk`
- `sdk/rust/codepet-gateway-sdk`

Service SDKs contain serde DTOs, method/event enums, async server traits, dispatchers, typed client/transport shells, wire envelopes, and codecs. Provider additionally generates `ProtocolRequest::from_method_params`, so a transport can turn the generated method plus typed-client params into the exact JSON-RPC request enum without maintaining a second method/envelope match. Provider/Gateway capability enums and `ProtocolMethod::capability()` are generated from checked manifest/schema metadata. Provider descriptors expose protocol validation for non-empty supported instance kinds, and `instance.create` plus returned instances carry the selected `instanceKind`. The generated packages remain transport contracts rather than business runtimes. The consuming implementation is now `crates/codepet-host`: it owns device/instance persistence, Provider process supervision, Plugin Manager behavior, and an internal Gateway v1 service without copying protocol DTOs back into the host.

The four Rust SDK manifests are packageable crates rather than permanently private workspace crates. Their local path dependencies also declare version `0.1.0`, allowing local workspace development while preserving a publishable dependency graph. Repository tests are excluded from crate tarballs because they read the canonical fixtures outside each crate under `protocol/`; the tests still run from the SDK workspace, while published source remains self-contained without copying fixture facts. The repository currently has no license file; `cargo package` content checks can run, but the project should not publish until the repository owner makes an explicit license decision.

## Runtime Gateway compatibility

The existing in-process Runtime Gateway and Desktop Companion still use the unchanged v0 wire profile while v1 is introduced. That profile now lives at `gateway/v1/compat-v0.*` and is generated into `codepet-gateway-sdk::compat_v0` plus the TypeScript compatibility SDK. The Tauri and frontend files named `generated` are thin re-export shims only.

This compatibility path preserves current dual-channel behavior: remote App Server events remain on the remote gateway bus, Desktop IPC remains on the companion bus, and neither channel is migrated into the public Provider plugin protocol in this phase. Provider Host events have a third, v1-only service boundary and never fall back into either compatibility channel.

## Commands

```sh
npm run protocol:generate
npm run protocol:check
cargo test --manifest-path sdk/rust/Cargo.toml
node tools/protocol-codegen/generate.mjs --target=rust --check
```

`protocol:check` validates schema/manifest consistency, fixtures, schema and manifest dependency direction, capability mappings, target fail-closed behavior, and generated-file freshness. Runtime compatibility is covered by `runtime_gateway_protocol_tests` and `runtime_gateway_core_tests` in the Tauri crate.

## Current limits

- A reusable Plugin Manager and process supervisor exists in `crates/codepet-host`; no production Provider binary migration, signature, marketplace, sandbox, or automatic restart policy exists yet.
- No LAN listener, pairing, remote authentication, remote UI, or persistent event-cursor store exists yet.
- The v0 compatibility profile remains in use by the desktop process until a later phase wires gateway v1 sessions and a separate pet-protocol adapter.
