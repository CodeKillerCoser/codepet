# Capability Boundaries

## Frontend Contract

`frontend/lib/agentInteractions.ts` decides visible card actions. It must stay aligned with backend behavior in `src-tauri/src/agent/actions.rs`.

## Current Capabilities

- Activate: generally visible, but backend can return unsupported on platforms that cannot activate by the requested target type.
- Codex activate: uses a thread deeplink when a session id is available, otherwise falls back to the generic provider target.
- Reply: Codex only, done/failed only, session id required.
- Approval: waiting-approval only, resolved through collector state.

## Risk

If the frontend exposes an action the backend rejects, the pet window feels broken even if the backend is technically correct.

## Validation

- Update both frontend capability tests and Rust action tests when changing a provider capability.
- Check running, waiting approval, done, failed, missing session id, and unsupported provider cases.
