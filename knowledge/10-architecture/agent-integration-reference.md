# Agent 接入参考

桌宠活动感知和远程会话控制是不同能力。当前完整边界见 [运行拓扑](runtime-topology.md)。

## 支持的 Agent

| Agent | 配置文件 | 事件覆盖 |
| --- | --- | --- |
| Codex Desktop | `~/.codex/ipc/ipc.sock` | 独立 Desktop Companion，依赖本机 Desktop IPC 可用；不通过 Hook 接入 |
| Claude Code | `~/.claude/settings.json` | `SessionStart`、`UserPromptSubmit`、`PreToolUse`、`PostToolUse`、`PostToolUseFailure`、`PermissionRequest`、`Stop` |
| Qoder | `~/.qoder/settings.json` | `SessionStart`、`UserPromptSubmit`、`PreToolUse`、`PostToolUse`、`PostToolUseFailure`、`PermissionRequest`、`Notification`、`Stop` |
| Cursor | `~/.cursor/hooks.json` | `sessionStart`、`beforeSubmitPrompt`、`preToolUse`、`postToolUse`、`beforeShellExecution`、`afterShellExecution`、`beforeMCPExecution`、`afterMCPExecution`、`afterFileEdit`、`stop` |

启用 Claude Code、Qoder 或 Cursor 时，应用会把托管的 `code-pet-hook.mjs` 写入对应配置。关闭时会移除托管项，并清理该 Agent 的当前事件。托管命令使用 `node <script> --agent <id>` 形式传递 Agent 信息，并保留对旧版 `CODE_PET_AGENT=...` 托管项的识别和升级能力。应用启动时会移除 `~/.codex/hooks.json` 中由 Code Pet 管理的遗留 Codex Hook，且 collector 不接收 Codex Hook 或 spool 事件。

## Collector

应用内置一个本地 collector：

```text
http://127.0.0.1:47621/hook
```

hook 脚本会把 Agent 事件转发到这个地址。前端在浏览器预览无法使用 Tauri IPC 时，也会尝试读取：

```text
http://127.0.0.1:47621/events
```

collector 只绑定 `127.0.0.1`。

## 独立 Provider 通道

Codex、Claude 与 OpenCode 的远程能力经 Provider Host / Gateway 调用本机 runtime。Provider 事件不会直接进入桌宠活动列表；Claude runtime 继承的用户 Hook 可独立形成桌宠事件。

Codex Companion 仅观察已发现的 Desktop 任务，不提供完整任务目录。当前 transport 使用 Unix socket，Windows 不支持；私有 IPC 不兼容时显示不可用，不回退到 audit 或 Hook。详见 [Desktop Companion](../30-domains/agent-control/codex-desktop-companion.md)。

## 维护依据与验证

Hook 声明以 `src-tauri/src/agent/registry.rs` 为准，配置写入与 collector 测试位于 `src-tauri/tests/`。修改接入说明时同时核对 `frontend/lib/PetSources.svelte` 的状态展示、Companion transport 与 Provider 通道隔离测试；不可把远程会话能力写成桌宠支持。
