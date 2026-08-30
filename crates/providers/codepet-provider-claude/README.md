# Claude Provider

`codepet-provider-claude` is an independent Provider Protocol v1 binary. The Host starts it from the adjacent `codepet-provider.json` manifest and injects the absolute, resolver-validated Claude executable as the instance setting `claudeExecutable`.

The Provider runs the official Claude Code CLI in non-interactive `--print` mode with newline-delimited `stream-json` input and output. It creates provider-managed session IDs, resumes only those sessions with `--resume`, maps main-agent text deltas and terminal results, and uses a bounded SIGINT-to-SIGKILL sequence for interruption on Unix.

Every turn uses `--safe-mode`, an empty `--setting-sources`, `--strict-mcp-config`, and an explicit empty `--mcp-config`. This excludes user/project/local settings and all ordinary or plugin MCP configuration. Read-only conversations additionally use `--restricted` and expose only `Read`, `Glob`, and `Grep`; the Provider rejects a `system/init` frame that reports any MCP server or non-read-only tool. Managed policy settings remain authoritative in Claude Code, so the Provider does not claim that managed hooks are disabled and publishes no `hooksDisabled` metadata.

One background reaper owns each `Child`; controls hold only its PID/process group and exit notification. A result becomes terminal only after the process really exits. Instance stop/destroy/shutdown and interrupt use bounded process-group termination. Claude physical output lines are limited to 4 MiB, while outward text is split into at most 64 KiB chunks before entering the generated Provider codec's 1 MiB frame limit.

The currently documented CLI does not expose an App Server-equivalent session API. Consequently this Provider does not advertise conversation list/get, external session discovery, turn steering, approval callbacks, or process-restart session reconstruction. Unsupported Provider Protocol methods return `capability_unsupported`.

## Development install

1. Build with `cargo build --manifest-path crates/Cargo.toml -p codepet-provider-claude`.
2. Create `provider-plugins/claude/` under the configured Code Pet application data directory.
3. Copy `crates/target/debug/codepet-provider-claude` (or the Windows `.exe`) and this directory's `codepet-provider.json` into that directory.

Do not put a Claude executable path in the manifest. Configure or detect it through Code Pet's Agent Runtime resolver; the Host replaces the instance setting before create/start and when the configured runtime changes.

Run the focused checks with:

```text
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-claude --all-targets
```

`tests/fixtures/claude-2.1.251-no-auth.ndjson` is a path/UUID-sanitized capture from an actual `claude` 2.1.251 non-persistent `stream-json` run on an unauthenticated local installation. It preserves the observed event shapes, including the counterintuitive `subtype: "success"` plus `is_error: true` authentication result.
