# Agent 控制

## Hook 控制

`src-tauri/src/agent/control.rs` 维护 Agent 列表并切换启用状态；具体 JSON 配置改写委托给 `src-tauri/src/agent/hooks.rs`。

`hooks.rs` 会把 `code-pet-hook.mjs` 写入本地 app data，然后为每个已勾选的支持事件安装托管 hook。它通过旧 marker、脚本名或脚本路径识别已有托管项。

Agent 开关和 hook 事件勾选是两个层级：开关决定是否安装 Code Pet 托管项，勾选项决定安装哪些事件。默认勾选全部支持事件；当已启用 Agent 的勾选项变化时，后端会立即同步对应 JSON 配置，不需要重启应用。

## 活动控制

`src-tauri/src/agent/actions.rs` 提供激活、回复和审批行为。

- Codex 回复：当事件已完成或失败且带 session id 时，使用 `src-tauri/src/agent/codex_app_server.rs`。
- Codex 激活：当事件有 session id 时，使用 `codex://threads/<thread-id>` deeplink。
- Qoder 当前没有经过验证的“向现有本机会话发送消息”路径。
- 审批处理通过 collector 的等待路径解决 `waiting-approval` 事件。
- 激活能力依赖平台，可使用应用名、bundle id、路径或 macOS 终端会话自动化。

## 前端能力边界

`frontend/lib/agentInteractions.ts` 映射用户可见能力：

- Codex 只有在 `done` 或 `failed` 且带 session id 时可以回复。
- Qoder 可以审批等待授权事件，但暂时不能回复现有本机会话。
- 运行中的任务不应暴露回复入口。

## 验证

- 运行 `cargo test --manifest-path src-tauri/Cargo.toml agent_control_tests`。
- 运行 `cargo test --manifest-path src-tauri/Cargo.toml hook_config_tests`。
- 运行 `npx vitest run frontend/lib/activity.test.ts`。
