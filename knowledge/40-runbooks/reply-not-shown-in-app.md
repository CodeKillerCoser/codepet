# Reply Not Shown In App

## Symptom

The pet card reply action appears to submit, but the message does not show in the provider app.

## Evidence To Collect

- Provider, event id, status, and session id.
- Whether frontend capability exposed reply for that exact event.
- Backend result from `send_activity_reply`.
- Provider-specific logs or app-server stderr when available.
- Manual confirmation that provider UI displays the message.

## Checks

1. Confirm the event is `done` or `failed`.
2. Confirm the event has a non-empty session id.
3. Confirm provider is Codex. Qoder existing-session reply is intentionally unsupported.
4. For Codex, inspect `src-tauri/src/agent/codex_app_server.rs` behavior and app-server startup path.
5. Confirm the reply path is not being confused with approval resolution.

## Validation After Fix

- `cargo test --manifest-path src-tauri/Cargo.toml activity_actions_tests agent_control_tests`
- `npx vitest run frontend/lib/activity.test.ts`
- Manual Codex app UI check that the sent text appears in the thread.
