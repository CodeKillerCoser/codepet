# Independent Code Pet Provider fixtures — business 4/4

These JavaScript providers are implemented from the exported public SDK and public language dependencies. The active snapshot is `/tmp/codepet-notice-sdk-release-20261116/v2`. No Host, desktop, Android, repository internals or private configuration are used to implement the fixtures.

The business protocol is **fixed 4/4**. Initialize accepts `protocol:{version,minVersion}`, selects the highest intersection, and rejects old 2/2 or 3/2 peers. Software is `4.0.0`; SDK directory/Manifest remain v2 and transport profiles remain v1. There is no legacy business mode. `ECHO_PROTOCOL_VERSION` or `--protocol-version` values other than 4 fail startup; the package generator likewise rejects old versions.

## Installable packages

Import the package's `codepet-provider.json`; copy its complete parent directory when moving it. All entry points are one bundled `provider.mjs` plus the public `contract/` resources and licenses. **Node 24+ is required; no runtime node_modules installation is needed.** Existing Agent plugin identities are retained.

| Directory under packages/ | Plugin ID | Backend / features | Profile |
| --- | --- | --- | --- |
| jsonl | independent.echo.jsonl | Echo / agent | stdio-jsonrpc-v1 |
| zstd | independent.echo.zstd | Echo / agent | stdio-codepet-zstd-v1 |
| mux | independent.echo.mux | Echo / agent | stdio-codepet-mux-v1 |
| pi-jsonl | independent.pi.jsonl | Pi / agent | stdio-jsonrpc-v1 |
| notice-jsonl | independent.notice.jsonl | Notice / observation | stdio-jsonrpc-v1 |
| notice-zstd | independent.notice.zstd | Notice / observation | stdio-codepet-zstd-v1 |
| notice-mux | independent.notice.mux | Notice / observation | stdio-codepet-mux-v1 |

Notice manifests have `instances:[]`, and their descriptors have `instanceKinds:[]`. No Agent instance is invented or created. Features are declared only in Initialize/Describe, not duplicated in the installation Manifest. No fixture claims runtimeInventory; `runtime.getInstalled` returns `selectionRequired:false`, an empty inventory and selected:null.

The full public Manifest Schema is compiled with open-source Ajv. Missing icon/command/transport, Manifest v1, the old executable field, or malformed icon URLs fail standard validation. There is no loading/import/discovery exception. Echo/Notice use the public Node.js favicon `https://nodejs.org/static/images/favicons/favicon.png`; Pi uses `https://pi.dev/logo.svg`. These HTTPS image URLs contain no authentication information. The generator validates before writing a package.

## Build and test

```sh
npm ci
npm run build
npm test
PROVIDER_TEST_DIRECTORY="$PWD/packages/notice-mux" npm test
```

Repeat the final command for each of the seven directories to exercise that bundled entry point. `package-lock.json` pins public dependencies. Editable modules are `provider.mjs` (business dispatch and serial framing), `mux.mjs` (public Yamux adapter), `contracts.mjs` (manifest-driven Schema/registry loader), `selection.mjs`, `pi.mjs`, `notices.mjs`, the build/generator scripts, icons and tests. The public schemas are retained without modifications or alias patches. Manifest validation is separate from communication-schema loading.

```sh
node make-package.mjs --out packages/custom-notice \
  --plugin-id independent.notice.custom --backend notice \
  --transport stdio-codepet-zstd-v1
```

The generator also supports Echo/Pi, custom instance identity, Node executable and a schema-valid `--icon`. New package IDs avoid replacing a user's existing installation. No script in this distribution installs into Host configuration or starts Host.

## Duplex Notice lifecycle

Notice is **business JSON-RPC in both directions**, not an event notification or an extra network listener. After the Initialize response is physically written, the Notice backend opens a Provider-to-Host request with a distinct string ID and `notice.create`. It waits for the correlated Host response, then publishes `notice.update` and `notice.finish` serially for the same source. Stable `creationKey`/`operationId` values are independent of JSON-RPC IDs. Updates/finish use the exact revision returned by Host. Finish always sends its final status; no disconnect or observation toggle fabricates a final state.

Default status values (`fixture_queued_blue`, `fixture_working_violet`, `fixture_done_copper`) are arbitrary source-defined strings, not Host lifecycle enums. Content includes title plus optional progress text/percent. The activation action has only the structured `{kind:"uri",uri:"https://nodejs.org/"}` target; the **Host platform adapter** owns activation. This plugin never executes an activation command or opens a URL.

The reader continues processing Host requests/responses while an outbound mutation awaits Host. JSONL/zstd demultiplex IDs over the same framed pipe; mux uses a bidirectional Yamux stream for each business request/response. Provider events retain ordered ACK handling separately. Host ping, describe and observe continue while publication is pending.

`notice.observe` disables future publication steps or resumes them, without starting/stopping Agent instances. An already sent mutation may finish; the next step waits until observation is enabled. Initially disabled observation or `NOTICE_AUTOPUBLISH=0` can be used for manual acceptance.

`notice.action.execute` handles only the fixture's currently registered/enabled reply and permission actions at the expected revision. It returns deterministic local receipts, **without sending any real message or granting any real permission**. Replaying an accepted executionId returns its stored result; changed reuse is `execution_conflict`. Invalid action, disabled action, mismatched input, invalid permission choice, stale revision and plugin-side activation are rejected. A configurable first execution returns `unknown`; that result is retained and is not executed or automatically retried. The per-process ledger is bounded to 1024 executions. It is intentionally a conformance fixture, not a durable action service; receipt/history state does not survive restart. The Host owns durable Notice state.

Outbound RPC timeouts or stream failure are recorded as a result-unknown condition; **no automatic resend** occurs. Re-enabling observation does not restart a failed/unknown lifecycle. On reconnect, stable creationKey can identify a previously finished Host Notice; the fixture does not update it back to open.

### Notice controls

Add env entries to a copied Manifest. Defaults publish one complete lifecycle after Initialize. Numeric values are milliseconds unless noted.

| Variable | Default | Purpose |
| --- | --- | --- |
| ECHO_BACKEND | echo | `notice` selects the pure observer backend. |
| NOTICE_AUTOPUBLISH | 1 | `0` disables the automatic lifecycle. |
| NOTICE_ENABLED | 1 | `0` waits for notice.observe(enabled:true). |
| NOTICE_SOURCE_ID | fixture-source | Stable local source identity. |
| NOTICE_CREATION_KEY | independent-notice-lifecycle-v1 | Stable per-source creation identity. |
| NOTICE_UPDATE_DELAY_MS | 50 | Delay after create before the update gate. |
| NOTICE_FINISH_DELAY_MS | 50 | Delay after update before the finish gate; increase for interactive actions. |
| NOTICE_RPC_TIMEOUT_MS | 3000 | Outbound response timeout; never auto-retry. |
| NOTICE_UNKNOWN_ONCE | 0 | `1` makes the first accepted action return unknown; replay stays unknown. |
| NOTICE_INITIAL_STATUS | fixture_queued_blue | Free-form initial status value. |
| NOTICE_UPDATE_STATUS | fixture_working_violet | Free-form intermediate status value. |
| NOTICE_FINAL_STATUS | fixture_done_copper | Free-form required final status value. |

`notice.observe.message` reports fixture state and the local execution count, using the standard response's diagnostic text field. It is useful for acceptance and is not an extra private method. A slow Host-response test can hold create while sending ping and observe. A dropped-response test should see one publication only and an unknown diagnostic.

## Preserved Agent behavior

Echo returns text unchanged through ordered deltas and complete items; accepted clientRequestId values do not duplicate work and changed known input is rejected. Selection normalization keeps create/acquire/turn consistent with each instance's enabled capability choices and defaults; unknown/disabled/undeclared choices fail before acceptance. Echo uses default access and flat echo model; Pi uses one configured access mode and does not pretend to provide model switching.

`conversation.get(limit=1)` counts native turns, returns all items for the latest turn and supplies nextCursor when earlier turns exist. Manual cursor requests return earlier non-overlapping pages and omit nextCursor at exhaustion. UTF-16 chunk boundaries preserve surrogate pairs so every delta is independently valid Unicode. `ECHO_CHUNK_SIZE=4096` / `ECHO_TURN_DELAY_MS=1` produces a few chunks for a 32–64 KiB Chinese/emoji prompt. Existing partial-output, delay, large-event and shutdown controls remain available through ECHO_* env and corresponding CLI parameters.

Pi uses its public `pi --mode rpc --no-session` subprocess integration. Only allowlisted configured-model provider/id and assistant text are exposed, with the model identifier in `Conversation.model`; no pi config/auth files are opened and raw pi objects/stderr are not forwarded. It requires an independently configured pi CLI (`ECHO_PI_COMMAND` can select it). Tests use only the local fake public-RPC process and never call a real model, log in or contact an account.

## Public SDK conformance notes

The initial Notice export required nonempty instanceKinds despite pure observer zero-instance semantics; this was reported and corrected in the final export now used. Shape validation and cross-object semantics remain separate. The fixture always supplies real update changes and enabled activation targets/permission descriptions/options. The final public Schema now enforces those conditions, and conformance tests reject each missing configuration. All accepted outgoing requests and Host results are validated through the public method registry. Unknown incoming methods get -32601; pluginToHost methods cannot be invoked in the wrong direction. Business objects ignore added fields, known malformed fields still fail, and unsupported command choices are rejected.

## Validation result

`npm test` passes 34/34 against editable source. The complete suite separately passes 34/34 against each of all seven bundled entry points. All eight contract snapshot trees match `/tmp/codepet-notice-sdk-release-20261116/v2` exactly by directory membership and per-file SHA256. Manifest-driven Ajv validation resolves 36 request and 36 response methods plus 12 declared events. SDK sourceDigest: `sha256:1e8635457e8f69b6feb7dae42215f42400f278516eb3242dcd1621525eb22a1d`. Only fake local Host/pi peers are used; no account login, real reply/permission, URL activation, Host process or paid model call is performed.

Final standard conditions are included in the 34 tests: empty update groups are rejected, enabled activation requires a target, enabled permission requires description and a nonempty options list, and final status is mandatory. The final SDK is used for every source and packaged snapshot; no schema aliases or local edits are introduced.

The clean public v4 registry removes the obsolete event.subscribe/event.unsubscribe methods and event.notification payload. Their former handler branches were deleted; registry-validation tests confirm absence and all three transports return -32601 for retired calls. Notice observation exclusively uses notice.observe and Provider-to-Host notice.create/update/finish; ordinary Agent domain events keep their declared ordered-delivery contract.

Clean-registry validation: source and all seven rebuilt bundles each pass 34/34 tests. The retired subscription calls are tested only for protocol rejection, not compatibility behavior. All eight public SDK snapshots exactly match the clean export. No Host/repository/configuration access, real model call or external account action occurred.

The final release normalizes usage.query metadata to canonical hostToPlugin. Wire names, payloads, fixed business 4/4 and fixture behavior are unchanged; no historical-direction compatibility branch is added. All source and package snapshots are replaced from the public release export.

Release validation: source and all seven rebuilt bundles each pass 34/34 tests, including the canonical usage.query direction assertion. All eight contract snapshots exactly match the final public release SDK by member path and SHA256; no compatibility branch or private source input was introduced.
