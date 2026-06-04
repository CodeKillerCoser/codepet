# Codex App Server As Primary Reply Path

## Background

Code Pet needs a reliable way to reply from a task card into a Codex conversation.

## Decision

Use Codex app-server as the primary active reply path for eligible Codex events.

## Alternatives Considered

- Terminal paste: brittle and depends on focus, shell state, and terminal app.
- Accessibility paste: broad permission surface and focus-sensitive.
- Hook-only reply: hooks describe activity but are not a reliable active chat transport.

## Rationale

Local probing verified that Codex app-server can send messages into a Codex app thread and show them in UI. The current backend routes Codex replies through `src-tauri/src/agent/codex_app_server.rs`.

## Impact

Reply visibility must require Codex, terminal status, and session id. Activation remains separate because app-server window/open-thread methods were not verified.

## Follow-Up

Revalidate if Codex app-server protocol changes.
