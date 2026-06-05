# 通知与审批

## 通知行为

通知由 `settings.notifications` 控制。权限、失败和完成状态可以分别控制是否响铃。等待审批状态可以重复提醒，直到被处理或达到重复限制。

`frontend/lib/sound.ts` 决定新事件是否响铃，以及重复通知是否继续。

## 审批行为

权限请求事件会在 `src-tauri/src/activity/events.rs` 中变成 `TaskStatus::WaitingApproval`。`src-tauri/src/app/state.rs` 保存待审批项，并提供异步等待路径。

collector 暴露 `/approvals/:event_id/wait`；桌宠 UI 通过 `resolve_activity_approval` 处理待审批决策。

## 用户可见规则

审批控件只应在任务等待审批时出现。回复控件不应在审批阶段出现，除非 UI 明确通过审批流程发送审批消息。

## 验证

- 运行 `cargo test --manifest-path src-tauri/Cargo.toml activity_actions_tests`。
- 运行 `npx vitest run frontend/lib/activity.test.ts frontend/lib/sound.test.ts`。
