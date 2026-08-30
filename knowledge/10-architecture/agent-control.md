# Agent 控制

## Hook 控制

`src-tauri/src/agent/control.rs` 维护 Agent 列表并切换启用状态；具体 JSON 配置改写委托给 `src-tauri/src/agent/hooks.rs`。

Codex 只保留无 Hook 的 disabled 占位项。Hook 控制命令不会为它写入配置；列举 Agent 时会清理旧 Code Pet 托管项。下述 Hook 安装与审批等待行为仅适用于仍使用旧 collector 的 Claude Code、Qoder 和 Cursor。

`hooks.rs` 会把 `code-pet-hook.mjs` 写入本地 app data，然后为每个已勾选的支持事件安装托管 hook。它通过旧 marker、脚本名或脚本路径识别已有托管项。

Agent 开关和 hook 事件勾选是两个层级：开关决定是否安装 Code Pet 托管项，勾选项决定安装哪些事件。默认勾选全部支持事件；当已启用 Agent 的勾选项变化时，后端会立即同步对应 JSON 配置，不需要重启应用。

## 活动控制

`src-tauri/src/agent/actions.rs` 提供激活、回复和审批行为。

- 旧 Codex `PetEvent` 的回复和审批路径仍为 unsupported；Hook/audit/transcript 不得成为 Codex fallback。
- Codex Remote 通过独立 App Server Provider 提供 list/get/create、turn 和 approval，面向远程控制；它的事件不进入桌宠。
- Codex Desktop Companion 通过专用 Tauri channel 声明 get、start/steer/interrupt 与 command/file 二元审批；动作只对已 bootstrap 且 owner 仍有效的 Desktop 会话受理，Provider 状态不参与其判定。
- Qoder 当前没有经过验证的“向现有本机会话发送消息”路径。
- 审批处理通过 collector 的等待路径解决 `waiting-approval` 事件。
- 激活能力依赖平台，可使用应用名、bundle id、路径或 macOS 终端会话自动化。

## 前端能力边界

`frontend/lib/agentInteractions.ts` 映射用户可见能力：

- Codex 旧 `PetEvent` 卡片不暴露操作能力；桌宠只接受 namespace 为 `codepet.codex-desktop` 且 source 为 `codex-desktop-private-ipc` 的 companion capability，remote App Server capability 即使同为 provider id `codex` 也失败关闭。
- Qoder 可以审批等待授权事件，但暂时不能回复现有本机会话。
- 运行中的任务仅在 Provider 声明 `turn.send` 且当前会话满足受理前置条件时暴露回复入口。

## 验证

- 运行 `cargo test --manifest-path src-tauri/Cargo.toml agent_control_tests`。
- 运行 `cargo test --manifest-path src-tauri/Cargo.toml hook_config_tests`。
- 运行 `npx vitest run frontend/lib/activity.test.ts`。
