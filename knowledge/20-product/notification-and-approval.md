# 通知与审批

## 通知行为

设置保留 `settings.notifications` 与声音试听。当前桌宠通过 Pet Gateway 读取任务，并未接入事件响铃或重复审批提醒；这些是旧事件驱动桌宠的行为。

`frontend/lib/sound.ts` 保留主窗口使用的声音试听和桌面抽打音效。未被调用的旧事件响铃判断与重复通知函数已在桌宠清理中删除。

## 审批行为

权限请求事件会在 `src-tauri/src/activity/events.rs` 中变成 `TaskStatus::WaitingApproval`。`src-tauri/src/app/state.rs` 保存待审批项，并提供异步等待路径。

collector 暴露 `/approvals/:event_id/wait`；后端保留 `resolve_activity_approval`。当前桌宠不提供审批入口，旧前端调用封装已删除；不能把后端接口存在视为 UI 已接入。

## 用户可见规则

桌宠可以展示等待审批状态，但不展示未经当前协议支持的审批或回复控件。

## 验证

- 运行 `cargo test --manifest-path src-tauri/Cargo.toml activity_actions_tests`。
- 运行 `npx vitest run frontend/lib/pet/model.test.ts frontend/lib/sound.test.ts`。
