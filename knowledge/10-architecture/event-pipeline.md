# Event Pipeline

## Source

Agent hook payloads arrive through `src-tauri/hooks/code-pet-hook.mjs` and are posted to the local collector at `http://127.0.0.1:47621/hook`.

## Normalization

`src-tauri/src/activity/events.rs` converts raw payloads into `PetEvent` values:

- provider, kind, status, title, message, session id, cwd, tool name, source metadata, and ring flag.
- Cursor event names are canonicalized into the shared event vocabulary.
- Idle notification boilerplate is suppressed.
- Failure signals can convert terminal notifications into failed task events.

## Enrichment And Storage

`src-tauri/src/activity/title_resolver.rs` improves task titles. `src-tauri/src/app/state.rs` stores recent events, caps frontend output, and tracks pending approvals.

## Frontend Merge

`frontend/lib/activity.ts` groups events by provider plus session or cwd, filters internal/background events, hides configured filter matches, drops stale active work, and keeps terminal cards only when they belong to visible activity.

## Risks

- Changing event identity can merge unrelated tasks or split one task into multiple cards.
- Changing terminal event handling can reintroduce orphan completed cards.
- Filtering must persist hidden keys across incremental batches, or filtered background tasks can reappear.

## Validation

- Run `npx vitest run frontend/lib/activity.test.ts`.
- Run relevant Rust normalizer tests under `src-tauri/tests/event_normalizer_tests.rs`.
