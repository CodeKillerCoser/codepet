# Agent Control

## Hook Control

`src-tauri/src/agent/control.rs` lists agents and toggles enabled state. It delegates JSON config mutation to `src-tauri/src/agent/hooks.rs`.

`hooks.rs` writes `code-pet-hook.mjs` into local app data, then installs managed hook entries for each supported event. It recognizes existing managed entries by legacy marker, script name, or script path.

## Activity Control

`src-tauri/src/agent/actions.rs` provides activation, reply, and approval behavior.

- Codex replies use `src-tauri/src/agent/codex_app_server.rs` when the event is completed or failed and has a session id.
- Qoder currently has no verified existing-local-session send path.
- Approval resolution uses the collector wait path for `waiting-approval` events.
- Activation is platform-dependent and can use app names, bundle ids, paths, or macOS terminal session automation.

## Frontend Capabilities

`frontend/lib/agentInteractions.ts` mirrors user-visible capabilities:

- Codex can reply only for `done` or `failed` events with a session id.
- Qoder can approve waiting approvals, but cannot reply to existing local sessions yet.
- Running tasks should not expose reply.

## Validation

- Run `cargo test --manifest-path src-tauri/Cargo.toml agent_control_tests`.
- Run `npx vitest run frontend/lib/activity.test.ts`.
