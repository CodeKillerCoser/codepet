# Agent Control

This domain covers actions that control or interact with provider sessions after an event appears.

## Source Modules

- `src-tauri/src/agent/actions.rs`: driver abstraction for activation, replies, and approvals.
- `src-tauri/src/agent/codex_app_server.rs`: Codex app-server reply path.
- `frontend/lib/agentInteractions.ts`: frontend capability display rules.
- `src-tauri/src/app/state.rs`: pending approval storage and resolution.

## Principle

Provider capability controls UI visibility. A task card action must not be visible unless the backend has a verified path for that action and task state.
