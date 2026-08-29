# Agent 控制领域

这个领域覆盖事件出现后，对 provider session 进行控制或交互的动作。

## 源码模块

- `src-tauri/src/agent/actions.rs`：激活、回复和审批的 driver 抽象。
- `codex-app-server.md`：文件名为链接兼容保留；内容记录 Codex Desktop 私有 IPC、Owner/Follower、revision，以及仅限 follower 公告或显式已知且已 bootstrap owner 会话的 start/steer/interrupt 与 command/file 二元审批边界。
- `src-tauri/src/agent/codex_desktop_ipc/`：私有 IPC transport、Owner/Follower bootstrap 和标准状态映射；私有 DTO 不得越过该边界。
- `src-tauri/src/agent/runtime.rs`：Agent executable 检测、验证、配置和诊断。
- `frontend/lib/agentInteractions.ts`：前端能力展示规则。
- `src-tauri/src/app/state.rs`：待审批项存储和处理。

## 原则

Provider capability 决定 UI 可见性。除非后端对当前动作和任务状态有已验证路径，否则任务卡片不应显示该操作。

Agent Runtime 的解析优先级、平台发现和设置边界见 `agent-runtime.md`。

Codex executable 检测可以保留为独立安装信息，但 Codex Provider 的可用性只由 Desktop socket 和握手决定。
