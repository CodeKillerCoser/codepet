# Agent Control Known Risks

## Reply Timing

Reply should appear after a task reaches `done` or `failed`, not while it is running. Running tasks should not show reply because the control path may need steering semantics that differ by provider.

## Approval Timing

Approval actions should appear only for `waiting-approval` events. Approval replies belong to the approval decision path, not the normal reply path.

## Provider Drift

Remote-control APIs can change. Keep provider-specific logic behind `AgentInteractionDriver` and capability checks rather than spreading provider conditions through UI components.

## Validation

- `cargo test --manifest-path src-tauri/Cargo.toml activity_actions_tests`
- `npx vitest run frontend/lib/activity.test.ts`
