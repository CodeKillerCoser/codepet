# CodePet protocol IDL

## Single source of truth

Everything under `protocol/` is language-neutral, handwritten protocol input. Generated Rust and TypeScript files are outputs only; they must not be edited as an alternative model source.

The v1 layers are:

- `core/v1` — stable IDs, timestamps, versions, pagination, errors, JSON primitives, and routed resource identity.
- `pet/v1` — Desktop Companion-driven `PetTask`, `PetApproval`, `PetAction`, snapshot, and patch contracts. It does not reference Provider conversation, turn, or approval models.
- `provider/v1` — public Host ↔ independent Provider binary JSON-RPC 2.0 over newline-delimited stdio. It owns initialize, describe, instance lifecycle/capability, conversation, turn, approval, event, and shutdown contracts.
- `gateway/v1` — Host ↔ Remote Client methods and replayable events. Resources use `deviceId + providerInstanceId + nativeResourceId`; the gateway exposes neither plugin process lifecycle nor pet-private state.

`codegen.json` declares packages, dependency direction, output targets, and the shared `codepet.protocol.codegen/v1` interface for Rust, TypeScript, Dart, and Python. Rust is active for all four v1 layers. TypeScript currently covers core plus the Runtime Gateway compatibility surface; Dart and Python remain planned targets.

## Versioning and discriminators

Every public v1 initialize/handshake request carries an explicit supported `VersionRange`, and the response selects one `ProtocolVersion`. Method and event names live in each layer's manifest rather than Rust code.

- Pet and gateway use CodePet envelopes discriminated by `method` and `event`.
- Provider uses JSON-RPC 2.0 requests discriminated by `method`, JSON-RPC responses, and notification events also discriminated by `method`.
- Gateway v1 events carry an opaque `eventCursor` for replay.
- Union-like domain DTOs such as `PetAction` retain an explicit `kind`; receivers validate kind-specific optional fields.

The generator supports a deliberately small JSON Schema Draft 2020-12 subset. Unsupported keywords, unresolved references, duplicate method/event names, invalid fixtures, or undeclared cross-layer dependencies fail generation.

## Generated SDKs

Rust packages are located at:

- `sdk/rust/codepet-core-sdk`
- `sdk/rust/codepet-pet-sdk`
- `sdk/rust/codepet-provider-sdk`
- `sdk/rust/codepet-gateway-sdk`

Service SDKs contain serde DTOs, method/event enums, async server traits, dispatchers, typed client/transport shells, wire envelopes, and codecs. They contain no Provider manager, process supervisor, registry, business handler, authentication, or UI behavior.

## Runtime Gateway compatibility

The existing in-process Runtime Gateway and Desktop Companion still use the unchanged v0 wire profile while v1 is introduced. That profile now lives at `gateway/v1/compat-v0.*` and is generated into `codepet-gateway-sdk::compat_v0` plus the TypeScript compatibility SDK. The Tauri and frontend files named `generated` are thin re-export shims only.

This compatibility path preserves current dual-channel behavior: remote App Server events remain on the remote gateway bus, Desktop IPC remains on the companion bus, and neither channel is migrated into the public Provider plugin protocol in this phase.

## Commands

```sh
npm run protocol:generate
npm run protocol:check
cargo test --manifest-path sdk/rust/Cargo.toml
```

`protocol:check` validates schema/manifest consistency, fixtures, layer dependencies, future language target declarations, and generated-file freshness. Runtime compatibility is covered by `runtime_gateway_protocol_tests` and `runtime_gateway_core_tests` in the Tauri crate.

## Current limits

- No Plugin Manager, Provider binary migration, signature, marketplace, or sandbox exists yet.
- No LAN listener, pairing, remote authentication, remote UI, or persistent event-cursor store exists yet.
- The v0 compatibility profile remains in use by the desktop process until a later phase wires gateway v1 sessions and a separate pet-protocol adapter.
