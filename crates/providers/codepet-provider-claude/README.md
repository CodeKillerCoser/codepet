# Claude Provider

`codepet-provider-claude` is an independent Provider Protocol v1 binary. The Host starts it from the adjacent `codepet-provider.json` manifest and injects the absolute, resolver-validated Claude executable as the instance setting `claudeExecutable`.

The binary implements the generated `Provider` trait and delegates stdio transport to `codepet-provider-sdk::serve_stdio`. The common SDK owns mux bootstrap before business initialization, independent request/response streams, JSON/raw/zstd encoding, encoded/decoded budgets, normal/small/control lanes, backpressure, ordered typed events, terminal cleanup, and shutdown drain; this crate owns only Claude CLI process state and mapping. Provider implementations do not read or write the physical transport themselves.

The Provider runs the official Claude Code CLI in non-interactive `--print` mode with bidirectional newline-delimited `stream-json`. It creates provider-managed session IDs, resumes only those sessions with `--resume`, maps main-agent text deltas and terminal results, handles `can_use_tool` control requests through `--permission-prompt-tool stdio`, and uses a bounded SIGINT-to-SIGKILL sequence for interruption on Unix.

Every turn intentionally uses Claude Code's normal configuration discovery. The Provider does not pass `--safe-mode`, `--setting-sources`, `--settings`, MCP restriction flags, or tool restrictions, and it does not rewrite Claude auto-memory environment variables. User/project/local/managed settings, MCP servers, hooks, plugins, and memory therefore behave as they do when the same Claude executable is launched normally in that workspace. The selected CodePet access mode is passed as Claude's permission mode; `manual` is the default, and permission prompts are surfaced to Remote through Provider Protocol approval events.

The legacy conversation-create path accepts the Provider Protocol `workspace-write` label as the compatibility entry for inherited Claude defaults. It does not force Claude's `acceptEdits` mode. Strong `read-only` and `full-access` modes are not accepted because inheriting arbitrary local configuration cannot guarantee those semantics.

One background reaper owns each `Child`; controls hold its PID/process group, exit notification, and synchronized stdin writer used for permission responses. A result becomes terminal only after the process really exits. Instance stop/destroy/shutdown, fatal Provider stdio failures, normal Provider stdin EOF, and interrupt all use bounded process-group termination. Pending approvals expire when the owning turn exits. Fatal cleanup never depends on Provider stdout remaining writable: the SDK first disables and drops unobservable cleanup events, completes bounded Provider cleanup, then closes the failed mux connection within the bounded drain deadline. Claude physical output lines are limited to 4 MiB, while outward text is split into at most 64 KiB chunks before it enters the SDK-owned mux transport. A message body retains the CPRF header inside its mux stream. Encoded messages are limited to 16 MiB and decoded JSON to 128 MiB. Mux is the only STDIO mode, including when no transport environment variable is set.

The CLI does not expose an App Server-equivalent global session API, so this Provider does not claim external session discovery, turn steering, or process-restart session reconstruction. Provider-managed conversation create/list/get, `turn.start`/Gateway `turn.send`, Unix interruption, and approval resolution are available. Unsupported methods return `capability_unsupported`.

## Development install

1. Build with `cargo build --manifest-path crates/Cargo.toml -p codepet-provider-claude`.
2. Create `provider-plugins/claude/` under the configured Code Pet application data directory.
3. Copy `crates/target/debug/codepet-provider-claude` (or the Windows `.exe`) and this directory's `codepet-provider.json` into that directory.

Do not put a Claude executable path in the manifest. Configure or detect it through Code Pet's Agent Runtime resolver; the Host replaces the instance setting before create/start and when the configured runtime changes.

Run the focused checks with:

```text
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-claude --all-targets
cargo test --manifest-path crates/Cargo.toml -p codepet-host --test builtin_provider_integration
```

`tests/fixtures/claude-2.1.251-no-auth.ndjson` is a path/UUID-sanitized capture from an actual `claude` 2.1.251 non-persistent `stream-json` run on an unauthenticated local installation. It preserves the observed event shapes, including the counterintuitive `subtype: "success"` plus `is_error: true` authentication result.

Item mappers apply the SDK's optional `truncate_tool_item_text` helper only to `kind: tool`: text fields retain UTF-8-safe head/tail within 256 KiB and record paths and byte counts in optional item `_meta.truncations`. Other item variants stay complete. Runtime does not apply this policy or change pagination. See `knowledge/60-rules/provider-item-text-and-pagination.md`.
