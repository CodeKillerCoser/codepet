# Independent JavaScript Code Pet Provider

Developed as a third-party implementation from **only** the exported standalone SDK (current snapshot: `/tmp/codepet-apps-provider-sdk-semantic/v1`): README, JSON Schema, methods/events and profiles. No Host implementation, Rust bridge, repository examples or repository knowledge was read. Public npm dependencies and pi's public RPC documentation were used.

## Importable packages

Select one of these manifest files in Custom Provider Import. Copy the entire parent directory when moving a package:

| Package | Profile | Plugin ID | Instance ID |
| --- | --- | --- | --- |
| `packages/jsonl/codepet-provider.json` | `stdio-jsonrpc-v1` | `independent.echo.jsonl` | `independent-echo-jsonl` |
| `packages/zstd/codepet-provider.json` | `stdio-codepet-zstd-v1` | `independent.echo.zstd` | `independent-echo-zstd` |
| `packages/mux/codepet-provider.json` | `stdio-codepet-mux-v1` | `independent.echo.mux` | `independent-echo-mux` |
| `packages/pi-jsonl/codepet-provider.json` | `stdio-jsonrpc-v1` | `independent.pi.jsonl` | `independent-pi-jsonl` |

The echo packages use instance kind `independent-echo`; pi uses `independent-pi`. All packages require **Node 24+** with built-in zstd. The current development runtime is Node 26.9.0. The manifest launches `node provider.mjs` with the installed package as its working directory. Set `command` to an absolute Node executable if the Host's PATH does not contain Node.

Each `provider.mjs` is one bundled JavaScript entry point containing all JavaScript dependencies. **No node_modules installation is required to run an importable package.** The adjacent `contract/` contains public JSON resources used at runtime to derive method/event validation. stdout is exclusively protocol bytes.

## Reproduce the bundles

```sh
npm ci
npm test
npm run build
PROVIDER_TEST_DIRECTORY="$PWD/packages/jsonl" npm test
```

`npm run build` bundles all runtime dependencies with esbuild, recreates the three echo packages, and creates the optional pi package. `package-lock.json` pins dependencies. `provider.mjs`, `mux.mjs`, `pi.mjs`, `contracts.mjs`, `selection.mjs`, `build.mjs`, `make-package.mjs`, tests and `contract/` are the editable source distribution.

For a second package with a new identity:

```sh
node make-package.mjs --out packages/echo-second \
  --plugin-id independent.echo.second --instance-id echo-second \
  --transport stdio-codepet-zstd-v1
```

Options also include `--backend pi` and `--node /absolute/path/to/node`. The generated manifest changes both the installation identity and the reported plugin identity. Instance identity is Host-owned routing: edit the manifest's `instanceId` (or use the generator), and the Provider uses the exact route supplied in `instance.create`. It never overrides a Host route.

For direct launch of the source or a bundle, configure `CODEPET_PROVIDER_TRANSPORT` and `ECHO_PLUGIN_ID`, then execute `node provider.mjs`. The Host owns `CODEPET_PROVIDER_TRANSPORT` and `CODEPET_PROVIDER_PACKAGE_DIRECTORY`; package manifests never set either variable. `--transport` is a standalone testing fallback only when the Host transport variable is absent. Unsupported transports fail explicitly.

## Implemented behavior

- `provider.initialize`, `provider.describe`, heartbeat state validation, shutdown flush and EOF cleanup.
- Instance create/start/stop/destroy/capabilities, configured instance kinds and Host-supplied routes.
- Conversation create/list/get, pagination, interaction acquisition/release.
- `turn.start` accepts text, deduplicates identical `clientRequestId` work and rejects changed reuse; accepted work runs asynchronously and echoes text unchanged. `turn.interrupt` changes execution to interrupted and stops subsequent output.
- Stable item/content identity between `event.turnOutputDelta` and `event.conversationItemUpserted`; turn and conversation upsert events are emitted in execution order.
- Schema validation is generated at runtime from the exported methods/events tables rather than a duplicated table of DTO definitions. Capability advertisement includes only the implemented business methods.
- Echo conversations and accepted request identities live in process memory. This package is a conformance fixture, so they do not survive restart. Unsupported project/query/read-state APIs fail explicitly.

JSONL uses byte LF framing and fatal UTF-8 decoding after complete records. The independent CPRF implementation uses built-in Node zstd level 1 with decoded-output bounds; it has no Yamux dependency in its wire path. Corrupt framing/decompression fails the process.

Mux uses the public `@bbk47/yamux` implementation, not project code: 12-byte Yamux wire-v0 headers, server/even stream IDs, 256 KiB initial windows, CPMX four-stage bootstrap, explicit mandatory features, negotiated frame size, per-message GRANT, lane/budget checks, raw/zstd threshold, same-stream responses (control stays reserved; ordinary replies choose small/normal from their own encoded and decoded size), event ACK serialization, stream reset and half-close. The adapter implements the Code Pet envelope itself from the public guide. The fixture is not a substitute for exhaustive adversarial transport testing; native Host interoperability is a separate acceptance step.

## Test controls

The following env variables have corresponding `--kebab-case` arguments (e.g. `--describe-delay-ms 1500`). Add the env entries to a copied manifest to preserve them through import.

| Environment | Default | Effect |
| --- | ---: | --- |
| `ECHO_PLUGIN_ID` | `independent.echo.jsonl` | Reported descriptor identity; must match manifest. CLI `--plugin-id` takes precedence. |
| `ECHO_DESCRIBE_DELAY_MS` | 0 | Delay only describe; ping can complete concurrently. |
| `ECHO_TURN_DELAY_MS` | 30 | Delay before each echo chunk. |
| `ECHO_CHUNK_SIZE` | 16 | Echo characters per delta. |
| `ECHO_LARGE_EVENT_BYTES` | 0 | Append one delta of this many ASCII `x` bytes after the echo; capped below the decoded message bound. |
| `ECHO_PARTIAL_BYTES` | 0 | Write this many initial physical bytes, then the remainder. Applies to all stdout writes, including mux bootstrap/frames. |
| `ECHO_PARTIAL_DELAY_MS` | 0 | Pause between the two parts. |
| `ECHO_STALL_AFTER_PARTIAL` | 0 | Value `1` never writes the remainder; use with positive `ECHO_PARTIAL_BYTES`. |

Instance `settings` can override `turnDelayMs`, `chunkSize` and `largeEventBytes`. A 1 MiB compressed large-event test can use `{"largeEventBytes":1048576}`. A damaged package can use `ECHO_PARTIAL_BYTES=7` and `ECHO_STALL_AFTER_PARTIAL=1`; launch another unchanged package to check process/connection isolation. A delayed describe test can use `ECHO_DESCRIBE_DELAY_MS=2000` while sending ping on the same connection.

Do not put large-event or fault controls on the pi package. A `turn.start` request must use the revision returned by `instance.capabilities`; revisions are `independent-echo-v2` and `independent-pi-v2`. Shared domain resource IDs use `providerId=providerInstanceId`, while request/delta resource IDs contain the full Host route, as required by the contract.

## Optional real pi adapter

Install/configure pi independently, then import `packages/pi-jsonl/codepet-provider.json`. Set `ECHO_PI_COMMAND` in the manifest if `pi` is not on the Host's PATH. The adapter starts one real `pi --mode rpc --no-session` process per created conversation, in `conversation.create.workspaceRoot` or the Host-assigned initialization data directory.

The adapter does not open pi settings or authentication files, does not pass keys on argv, does not forward raw pi objects/stderr, and exposes only the configured model's provider/id plus assistant text events. **Pi itself owns configuration and authentication.** No provider/model is hardcoded or passed on the command line; the default comes from pi's configuration. On this machine, a real read-only RPC `get_state` returned `codepet-deepseek/deepseek-flash`. A single real text run completed with the expected exact reply; no keys were printed or inspected. Model/network acceptance beyond that is left to the Host owner.

The configured model is already exposed through the public `Conversation.model` field: JSON-RPC `conversation.create` returns `result.conversation.model`; `conversation.get` returns the same field; `conversation.list` returns it on each conversation summary. Its value is the allowlisted pi `get_state.data.model.provider` and `get_state.data.model.id` joined with `/`, for example `codepet-deepseek/deepseek-flash`. The minimal adapter advertises no model-switching catalog. It declares one configured access mode and returns `selection: {"accessModeId":"default"}`; `Conversation.model` identifies the configured model, and the undeclared model-selection dimension is omitted. Host acceptance can assert `Conversation.model` before starting a real turn without private methods or a model prompt.

`prompt` maps to async turn output, `abort` maps to interrupt, and shutdown/EOF kills owned pi processes. Pi errors become a generic failed turn instead of exposing arbitrary upstream diagnostics. Extension dialogs are cancelled. This minimal adapter exports assistant text; it does not expose tool approval, tool output, attachments or persistent pi session resume. Histories are Provider-owned process memory, and pi uses `--no-session`.

Public integration reference: pi package `@mariozechner/pi-coding-agent@0.73.1`, `docs/rpc.md`, commands `get_state`, `prompt`, `abort`, and events `message_update`/`agent_end`.

## Validation and contract observations

`npm test` exercises all three profiles using standalone processes and a public Yamux client. It covers fragmented input, correlation, async events, UTF-8/U+2028/newline echo, duplicate request handling, turn interruption, heartbeat invariants, failed initialization, corrupt/truncated/oversized headers, shutdown and independent connections. The same tests run against the bundled entry point with `PROVIDER_TEST_DIRECTORY`. A fake public pi RPC process tests field allowlisting without calling a model. Runtime dependency audit currently reports zero vulnerabilities.

One initially questioned wording was resolved without private implementation information: an empty manifest `instances` array does not conflict with descriptor `instanceKinds` being non-empty; the first controls automatic instantiation and the second declares supported kinds. No remaining contract ambiguity blocks the fixture. Native Host acceptance results should be recorded by the Host owner separately.

### Mux large-history response regression

Native Host acceptance found that a small `conversation.get` request incorrectly constrained a 128 KiB history response to the small lane, returning `message_too_large`. The response callback inherited the entire request class. It now retains only the control reserve; ordinary responses choose small or normal from their own encoded and decoded size. The public Yamux client regression sends a 128 KiB turn, waits for ordered completion, then checks that the small history request receives an intact normal-lane zstd response. Ping/shutdown replies are also checked to remain on control. The test failed against the prior implementation and passes after this change.

### Provider-managed runtime dependencies

Both Echo and Pi explicitly return `{"selectionRequired":false,"harnessList":[],"selected":null,"scanning":false}` from the public `runtime.getInstalled` method, including refresh requests. No `scanError` is emitted. The optional flag opts these plugins out of Host runtime selection; omitted flags retain the public contract's default `true` for other Providers. The bundled Echo runtime still needs the manifest's Node process dependency, and Pi still needs its configured external CLI. This metadata neither starts Pi nor asserts it is installed; actual startup failures continue through existing instance/conversation diagnostics.

The full public contract snapshot under `contract/` was refreshed from `/tmp/codepet-provider-contract-sdk-selection`, including its standalone README, wire schema, manifest schema, methods/events, profiles and canonical schemas. `test/runtime-selection.test.mjs` launches each backend, initializes it, checks the exact emitted response for ordinary and refresh calls, then shuts down. Pi's test intentionally points at a nonexistent CLI to prove runtime inventory does not launch it.

Validation for this runtime-selection update: source suite 13/13 passed; the same full suite passed 13/13 independently against each of `packages/jsonl`, `packages/zstd`, `packages/mux` and `packages/pi-jsonl`. JSONL, zstd and mux interoperability tests assert the emitted runtime-selection response. No real pi prompt or authentication-file access was performed for this update.

### Canonical SDK v1 manifest migration

The active contract snapshot is the complete public `/tmp/codepet-canonical-sdk/v1` export. `contracts.mjs` reads `contract/sdk-manifest.json`, discovers communication schemas below its `sdkSchemaRoot`, and resolves request/response/event references relative to the registry file named in the manifest. Initialize/Describe/Ping/Shutdown now correctly resolve through the lifecycle schema, while business calls resolve through the messages schema. No method DTOs or interfaces are copied into the loader. Plugin installation manifests are validated separately against `manifestSchema`, outside communication schema generation. Mux receives limits/defaults from the manifest-discovered handshake schema and compiles its shape validator from the same public source.

Existing plugin IDs, instance kinds, three transport profiles, Echo/Pi behavior and provider-managed runtime selection remain unchanged. The current guide explicitly requires connection-snapshot order stability at unchanged heartbeat revision and unique mux features; their validation now follows that rule. New tests resolve every method/event reference, distinguish lifecycle and business schemas, reject malformed payloads, validate the four installation manifests independently, and reject reordered same-revision heartbeat snapshots. This update performs no real-model call.

Canonical SDK migration validation: `npm run build` completed; `npm test` passed 15/15; `PROVIDER_TEST_DIRECTORY=/tmp/codepet-independent-provider/packages/<jsonl|zstd|mux|pi-jsonl> npm test` passed 15/15 separately for every bundle. The source and all four bundled contract snapshots were compared file-for-file by SHA256 against the public canonical SDK export and match exactly, with no leftover legacy schema tree.

### Final canonical SDK refresh

The active snapshot is now copied exclusively from `/tmp/codepet-canonical-sdk-final/v1`, replacing the previous contract directory in full. Its publication manifest includes the canonical source digest, installation manifest filename and complete transport machine descriptions. The manifest-driven loader and Provider business behavior are unchanged; the final export's DTOs and profile IDs remain compatible. The exported source digest is preserved verbatim in every package's `contract/sdk-manifest.json`. No paid model was called and no Host/repository source was read.

Final export validation: source 15/15 and each of the four rebuilt bundles 15/15 passed. The source `contract/` and all four package snapshots match `/tmp/codepet-canonical-sdk-final/v1` exactly by directory membership and per-file SHA256. Final SDK sourceDigest: `sha256:2ba20be7f4a19b375e277fc04a874df06ee707d707104e732908bb744097f8e1`.

### Selection consistency and manual history pagination repair

Evidence: Echo advertised `turnSend.accessMode.defaultId=default` and flat model `echo`, but created/acquired conversations returned `{}` and `turn.start` persisted caller selections unchanged, including `{}` or unknown values. Shape-only schema validation did not enforce relationships between these objects. The source now uses `selection.mjs` to fill declared defaults and reject unknown, disabled, mismatched or undeclared choices with stable `invalid_selection` errors before accepting work. Echo returns `{accessModeId:"default",model:{kind:"flat",modelId:"echo"}}` from create/acquire and effective turn selection. Pi explicitly advertises a single configured/default access mode and returns `{accessModeId:"default"}`; model selection is absent because the model stays in pi's configuration, while its allowlisted identifier remains in `Conversation.model`. The create selection directory matches the turn directory. Revisions advanced to `independent-echo-v2` / `independent-pi-v2`; plugin IDs and package layout are unchanged.

The current public semantic SDK snapshot at `/tmp/codepet-apps-provider-sdk-semantic/v1` includes the cross-object selection contract. Do not bypass client capability validation. Omitted declared choices resolve to the declared defaults; explicit invalid choices are rejected consistently, and rejected clientRequestIds can be retried with valid input because no work was accepted.

History `conversation.get(limit=1)` selects the latest native turn and its complete user/assistant items. With two turns it emits a non-empty `pageInfo.nextCursor`; manually requesting that cursor returns the earlier native turn and omits nextCursor at exhaustion. Cursor binding now uses ordered identity values rather than JSON object property order, so an equivalent reordered resource remains valid. Existing native-turn pagination already returned the right turn count; this change adds conformance coverage rather than replacing the algorithm.

Regression tests exercise defaults, create/acquire/turn consistency, persistent selection, unknown/unavailable/undeclared choices, repeatable rejection, and two-turn manual earlier-page retrieval. Echo is tested over JSONL/zstd/mux; Pi uses an executable public-RPC fixture, never a paid model. The new suite fails against the previous bundle and passes against repaired source. All snapshots are replaced from the semantic public SDK; no Host or client implementation is read.

For 32–64 KiB Chinese integration prompts, set manifest env `ECHO_CHUNK_SIZE="4096"` and `ECHO_TURN_DELAY_MS="1"` (or `"0"`) to produce a few deltas. Chunk size counts UTF-16 units and extends a boundary by one unit when necessary to preserve a surrogate pair. The selection/history regression also sends a large Chinese/emoji turn across such a boundary, asserting independently valid Unicode deltas, exact reassembly, complete-item equality and a small event count.

Selection/pagination repair validation: source 19/19 and each of the four rebuilt bundle suites 19/19 passed. Semantic SDK sourceDigest: `sha256:421531739813b7f3252d8b894a60412722cc29c4425f40d556273f983d0a07d3`. Source and all bundled snapshot trees match the semantic export exactly. Chinese/emoji streaming, earlier-page retrieval and selection validation ran without any real model call.
