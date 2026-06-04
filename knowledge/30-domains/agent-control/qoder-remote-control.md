# Qoder Remote Control

## Current Status

Qoder has a remote-control daemon direction, but Code Pet does not currently have a verified local API to send messages into an existing local Qoder session.

## Implemented Boundary

- Backend `QoderDriver` in `src-tauri/src/agent/actions.rs` returns `ReplyStrategy::Unsupported`.
- Frontend `qoderInteraction` in `frontend/lib/agentInteractions.ts` sets `canReply` to false.
- Qoder approval still uses the collector wait path when the task is waiting for approval.

## Evidence

Prior probing found that Qoder remote-control registers the machine as a remote environment through a cloud broker. The local `127.0.0.1:52345` MCP-like endpoint did not expose chat/session send APIs.

## Rule

Do not expose Qoder existing-session reply until a stable official or verified broker/API path can send a message and make it appear in Qoder UI.
