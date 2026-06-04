# Notification And Approval

## Notification Behavior

Notifications are controlled by `settings.notifications`. Permission, failure, and done states can each ring. Waiting approval can repeat until resolved or until repeat limits stop it.

`frontend/lib/sound.ts` decides whether a new event should ring and whether repeat notification should continue.

## Approval Behavior

Permission request events become `TaskStatus::WaitingApproval` in `src-tauri/src/activity/events.rs`. `src-tauri/src/app/state.rs` stores pending approvals and exposes an async wait path.

The collector exposes `/approvals/:event_id/wait`, and `resolve_activity_approval` resolves the pending decision from the pet UI.

## User-Facing Rule

Approval controls should appear only while a task is waiting for approval. Reply controls should not appear during approval unless the UI is intentionally sending an approval message through the approval flow.

## Validation

- Run `cargo test --manifest-path src-tauri/Cargo.toml activity_actions_tests`.
- Run `npx vitest run frontend/lib/activity.test.ts frontend/lib/sound.test.ts`.
