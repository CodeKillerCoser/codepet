# Hook Ingestion

## Current Flow

`src-tauri/src/agent/hooks.rs` installs a managed Node command for each supported Agent hook event. The command runs `code-pet-hook.mjs` with `--agent <id>` and, when available, `--event <event>`.

The hook script posts payloads to `http://127.0.0.1:47621/hook`. If the app is unavailable, events can be spooled to `~/.code-pet/spool/events.jsonl` and replayed on startup.

## Provider Configs

Agent registry data lives in `src-tauri/src/agent/registry.rs`. README documents the current config files for Codex, Claude Code, Qoder, and Cursor.

## Windows Risk

Windows command quoting is platform-specific. `hooks.rs` escapes `%` and quotes in the Windows implementation of `command_arg_quote()`. If Windows hook execution fails, inspect the generated config command first.

## Validation

- `cargo test --manifest-path src-tauri/Cargo.toml hook_config_tests`
- `cargo test --manifest-path src-tauri/Cargo.toml hook_script_tests`
