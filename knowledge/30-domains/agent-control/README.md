# Agent 控制领域

这个领域覆盖事件出现后，对 provider session 进行控制或交互的动作。

## 源码模块

- `src-tauri/src/agent/actions.rs`：激活、回复和审批的 driver 抽象。
- `src-tauri/src/agent/codex_app_server.rs`：Codex app-server 回复路径。
- `frontend/lib/agentInteractions.ts`：前端能力展示规则。
- `src-tauri/src/app/state.rs`：待审批项存储和处理。

## 原则

Provider capability 决定 UI 可见性。除非后端对当前动作和任务状态有已验证路径，否则任务卡片不应显示该操作。
