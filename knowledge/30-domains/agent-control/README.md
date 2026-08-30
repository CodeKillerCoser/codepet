# Agent 控制领域

这个领域覆盖事件出现后，对 provider session 进行控制或交互的动作。

## 源码模块

- `src-tauri/src/agent/actions.rs`：激活、回复和审批的 driver 抽象。
- `codex-app-server.md`：独立 Codex App Server 的 remote session、完整 Provider 能力与 runtime 生命周期。
- `codex-desktop-companion.md`：Codex Desktop 私有 IPC、Owner/Follower、revision、本地投影与安全动作边界。
- `crates/providers/codepet-provider-codex/`：官方 App Server stdio wire client、Provider v1 mapper 和独立 remote Provider binary。
- `src-tauri/src/runtime_gateway/provider_host_compat.rs`：既有 remote 调用面到 Provider Gateway 的薄适配。
- `src-tauri/src/agent/codex_desktop_ipc/`：私有 IPC transport、Owner/Follower bootstrap 和标准状态映射；私有 DTO 不得越过该边界。
- `src-tauri/src/agent/runtime.rs`：Agent executable 检测、验证、配置和诊断。
- `frontend/lib/agentInteractions.ts`：前端能力展示规则。
- `src-tauri/src/app/state.rs`：待审批项存储和处理。

## 原则

Provider capability 决定 UI 可见性。除非后端对当前动作和任务状态有已验证路径，否则任务卡片不应显示该操作。

Agent Runtime 的解析优先级、平台发现和设置边界见 `agent-runtime.md`。

Codex executable 检测和配置只驱动 remote App Server。Desktop companion 的可用性只由 Desktop socket 和握手决定；任一路 unavailable 不改变另一条 lifecycle 或数据。

桌宠只消费 companion 专用 snapshot/replay/event。remote App Server 的 snapshot/event 即使是标准协议对象，也不得进入桌宠 activity store。
