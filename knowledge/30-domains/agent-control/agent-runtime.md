# Agent Runtime 检测与配置

## 背景

Finder、Dock 或登录项启动的进程通常没有交互式 shell 的完整 `PATH`。Code Pet 的 Runtime 层负责发现、验证和持久化 Agent executable。对于 Codex，它现在只驱动 remote App Server；Desktop companion 直接连接私有 IPC，不依赖 executable resolver。

## 目标

- 用 `AgentRuntime` DTO 统一表达 Codex、Claude Code 和 OpenCode 的检测与诊断。
- 让用户配置优先，并从 PATH、登录 shell、macOS Spotlight 或 Windows 安装布局发现候选。
- 在保存前验证文件、执行权限和限时版本命令。
- Codex executable 变化时只刷新 remote App Server Provider。
- 让 remote runtime 与 Desktop companion 的 unavailable 状态互不影响。

## 非目标

- 不用 executable resolver 决定 Desktop IPC 是否可用。
- 不在 App Server unavailable 时改用 Desktop IPC 承担远程能力，也不在 IPC unavailable 时用 App Server 驱动桌宠。
- 不从前端暴露任意命令、参数或通用 shell 执行。
- 本阶段不实现 Claude Code/OpenCode Provider，也不实现远程网络 transport。

## 当前模型

`src-tauri/src/agent/runtime.rs` 是 executable 检测和解析事实来源。`AgentRuntime` 状态为：

- `ready`：候选通过文件与版本验证。
- `unavailable`：没有可用自动候选。
- `invalid-configured-executable`：保存的手动路径失效，不静默回退。

Codex runtime `ready` 后，`src-tauri/src/lib.rs` 使用解析出的路径 spawn/refresh `CodexRemoteProviderAdapter`。runtime unavailable 或 App Server initialize 失败时，remote Provider unavailable；这不关闭或清空 `CodexDesktopCompanionState`。反之，Desktop socket 不可用也不改变 remote App Server。

## 实现路径

### 解析与验证

候选优先级为：settings 手动路径、descriptor 环境变量、当前 PATH、登录 shell、平台动态发现。自动候选失败后可以继续尝试；手动配置失败则停止，以免显示配置与实际执行不一致。

`SystemExecutableValidator` 检查 metadata 和执行权限，canonicalize 为绝对路径，再用固定 `--version` 做三秒限时探测。前端只提交 provider id 与文件 picker 返回路径。

### Tauri 与双生命周期

运行时命令为 `list_agent_runtimes`、`detect_agent_runtime`、`refresh_agent_runtimes`、`set_agent_runtime_executable` 和 `clear_agent_runtime_executable`。

应用启动、全量刷新、设置或清除 Codex executable 时，后端调用 `refresh_codex_remote_provider`：关闭旧 App Server adapter，基于最新 runtime 注册新的 remote adapter。这个动作只操作 `RuntimeGatewayState`。Desktop companion 在另一份 state/registry/event bus 中，由 socket 自行连接和重连，其 generation、owner、revision 和 projection 不变化。

### UI

主窗口运行时页展示 executable 路径、来源、版本和诊断。这里的 `ready` 表示 executable 已验证，不等于 App Server session 已 initialize，更不等于 Desktop companion 可用。两条 Provider 的诊断应在各自 channel 展示。

## 涉及模块

- `src-tauri/src/agent/runtime.rs`：descriptor、候选发现、验证和 DTO。
- `src-tauri/src/app/settings.rs`：持久化 `agentRuntimes`。
- `src-tauri/src/lib.rs`：运行时 commands 与 remote App Server refresh。
- `src-tauri/src/agent/codex_app_server/`：消费已验证 executable 的 remote session。
- `src-tauri/src/agent/codex_desktop_ipc/`：完全独立的 Desktop socket/owner 生命周期。
- `frontend/App.svelte`、`frontend/lib/api.ts`、`frontend/lib/agentRuntime.ts`：运行时设置 UI。

## 风险与验证

- 登录 shell 或版本命令挂起：三秒超时测试。
- 损坏的高优先级自动候选遮挡有效 app：resolver 测试验证继续尝试。
- 无效手动路径覆盖旧配置：配置事务测试验证 settings 不变。
- refresh 误重启 companion：双生命周期测试与 state 边界静态审查确认只替换 remote adapter。
- executable ready 被误当成 session ready：UI 文案和 Provider handshake 测试分别验证。
- App Server refresh 期间请求中断：旧 adapter 先关闭，调用方得到 remote unavailable/error；不得转发到 companion。

## 测试计划

- Rust resolver：配置优先、无候选、无效配置、自动候选继续和 settings round trip。
- Rust bridge：remote 与 companion registry/lifecycle 独立；分别 unavailable 时另一侧仍工作。
- App Server fake peer：initialize 与完整 remote Provider 请求闭环。
- 前端 runtime 映射、Tauri 参数和 production build。

## 知识沉淀

- remote App Server 见 `codex-app-server.md`。
- Desktop companion 见 `codex-desktop-companion.md`。
- 双链路决策见 `../../50-decisions/codex-remote-and-desktop-companion-dual-channel.md`。

## 未知项

- App Server 子进程退出后的持续 supervisor/reconciliation 尚未实现。
- Claude Code 和 OpenCode 正式 Provider 尚未实现。
- Windows 动态布局本次未做实机验证。
