# Event Normalization

## Responsibility

`src-tauri/src/activity/events.rs` converts different provider payload shapes into the shared `PetEvent` model.

## Protected Behavior

- Cursor event names are mapped into shared lifecycle names.
- Idle notifications are treated as completed/idle terminal states with empty boilerplate message.
- Failure indicators in terminal-looking events can produce failed tasks.
- Source metadata under `code_pet` or `codePet` is carried forward for terminal/app source display and activation.

## Risks

- Provider payloads are unstable. New fields should be additive.
- Title or message extraction changes can alter filtering and card display.
- `sessionId` and `cwd` affect activity grouping.

## Validation

- Run `cargo test --manifest-path src-tauri/Cargo.toml event_normalizer_tests`.
- Run `npx vitest run frontend/lib/activity.test.ts` if normalized fields affect card grouping or labels.
