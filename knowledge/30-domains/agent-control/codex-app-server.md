# Codex App Server

## Current Use

Codex is the primary verified remote reply provider. `src-tauri/src/agent/actions.rs` routes eligible Codex replies to `src-tauri/src/agent/codex_app_server.rs`.

Codex activation is handled by the same provider driver, but it uses a `codex://threads/<thread-id>` deeplink rather than the app-server RPC path.

## Capability Boundary

The frontend exposes reply for Codex only when:

- event status is `done` or `failed`.
- the event has a non-empty `sessionId`.

This mirrors `is_replyable_event()` and `has_session_id()` in the backend.

## Known Capability

Prior local probing verified that Codex app-server can send messages into an existing Codex app thread and show them in the app UI. Window activation/open-thread RPC was not verified in the same path, so activation uses a separate thread deeplink.

## Validation

- Unit tests: `src-tauri/tests/activity_actions_tests.rs`.
- Frontend capability tests: `frontend/lib/activity.test.ts`.
- Manual validation is required when changing the JSON-RPC method sequence or the thread deeplink format.
