# Agent 控制已知风险

## 回复时机

回复入口应在任务到达 `done` 或 `failed` 后出现，而不是运行中出现。运行中任务不应显示回复，因为控制路径可能需要 steer 语义，不同 provider 的实现不同。

## 审批时机

审批操作只应出现在 `waiting-approval` 事件上。审批回复属于审批决策路径，不属于普通回复路径。

## Provider 漂移

Remote-control API 可能变化。Provider 专属逻辑应保留在 `AgentInteractionDriver` 和 capability 检查后面，不要把 provider 条件散落到 UI 组件里。

## 验证

- `cargo test --manifest-path src-tauri/Cargo.toml activity_actions_tests`
- `npx vitest run frontend/lib/activity.test.ts`
