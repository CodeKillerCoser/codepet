# Agent Runtime 检测与配置

## 背景

Finder、Dock 或登录项启动的进程通常没有交互式 shell 的完整 `PATH`。Code Pet 的 Runtime 层负责发现、验证和持久化 Agent executable。Codex resolver 只驱动 remote App Server Provider，Claude resolver 只驱动 Claude CLI Provider，OpenCode resolver 只驱动 remote OpenCode Server Provider；Codex Desktop companion 直接连接私有 IPC，不依赖 executable resolver，Claude 与 OpenCode 没有 companion 支路。

## 目标

- 用 `AgentRuntime` DTO 统一表达 Codex、Claude Code 和 OpenCode 的检测与诊断。
- 让用户配置优先，并从 PATH、登录 shell、macOS Spotlight 或 Windows 安装布局发现候选。
- 在保存前验证文件、执行权限和限时版本命令。
- Codex、Claude 或 OpenCode executable 变化时只刷新各自的 remote Provider。
- 让 remote runtime 与 Desktop companion 的 unavailable 状态互不影响。

## 非目标

- 不用 executable resolver 决定 Desktop IPC 是否可用。
- 不在 App Server unavailable 时改用 Desktop IPC 承担远程能力，也不在 IPC unavailable 时用 App Server 驱动桌宠。
- 不从前端暴露任意命令、参数或通用 shell 执行。
- 本阶段不实现远程网络 transport。

## 当前模型

`src-tauri/src/agent/runtime.rs` 是 executable 检测和解析事实来源。`AgentRuntime` 状态为：

- `ready`：候选通过文件与版本验证。
- `unavailable`：没有可用自动候选。
- `invalid-configured-executable`：保存的手动路径失效，不静默回退。

Codex/OpenCode runtime `ready` 后，Tauri 把 resolver 的绝对路径分别作为 `appServerExecutable` / `serverExecutable` 注入对应 manifest instance setting；只有独立的 `codepet-provider-codex` / `codepet-provider-opencode` 会使用它启动 Server。runtime unavailable 或 Server initialize 失败时，对应 remote Provider unavailable。Codex 侧故障不关闭或清空 `CodexDesktopCompanionState`，Desktop socket 不可用也不改变 remote App Server；OpenCode 不进入该 companion 生命周期。

Claude runtime `ready` 后，同一 Host boundary 把 resolver 的绝对路径作为 `claudeExecutable` 注入 `dev.codepet.claude` 的 `claude` instance；只有 `codepet-provider-claude` 使用它启动官方 CLI。Provider 内不再探测 PATH、VS Code 扩展或用户目录。Claude runtime unavailable 只让 Claude remote Provider unavailable，不触发 Hook/transcript、Pet 或 companion fallback。

## 实现路径

### 解析与验证

候选优先级为：settings 手动路径、descriptor 环境变量、当前 PATH、登录 shell、平台动态发现。自动候选失败后可以继续尝试；手动配置失败则停止，以免显示配置与实际执行不一致。

`SystemExecutableValidator` 检查 metadata 和执行权限，canonicalize 为绝对路径，再用固定 `--version` 做三秒限时探测。前端只提交 provider id 与文件 picker 返回路径。

### Tauri 与双生命周期

运行时命令为 `list_agent_runtimes`、`detect_agent_runtime`、`refresh_agent_runtimes`、`set_agent_runtime_executable` 和 `clear_agent_runtime_executable`。

应用启动时，`configured_provider_runtime` 在 Catalog 注册前分别用 resolver 结果覆盖 Codex、Claude 与 OpenCode instance setting。全量刷新、设置或清除 executable 时，`ProviderHostState::refresh_runtime_in_background` 通过每个 Provider 独立的 generation/lock 更新对应 setting，并只重启 `dev.codepet.codex`、`dev.codepet.claude` 或 `dev.codepet.opencode` 中的目标插件。这个动作不创建 Tauri 内 Server/CLI adapter，也不操作 companion、Hook 或 Pet。Codex Desktop companion 在另一份 state/registry/event bus 中，由 socket 自行连接和重连，其 generation、owner、revision 和 projection 不变化。

### UI

主窗口运行时页展示 executable 路径、来源、版本和诊断。这里的 `ready` 表示 executable 已验证，不等于 Provider session 已 initialize，更不等于 Desktop companion 可用。三个 remote Provider 的诊断应在各自 channel 展示。

## 涉及模块

- `src-tauri/src/agent/runtime.rs`：descriptor、候选发现、验证和 DTO。
- `src-tauri/src/app/settings.rs`：持久化 `agentRuntimes`。
- `src-tauri/src/lib.rs`、`src-tauri/src/runtime_gateway/tauri_bridge.rs`：运行时 commands、Host instance setting 更新与插件 refresh。
- `crates/providers/codepet-provider-codex/`：消费 Host 注入 executable 的 remote App Server session。
- `crates/providers/codepet-provider-claude/`：消费 Host 注入 executable 的官方 CLI stream-json session。
- `crates/providers/codepet-provider-opencode/`：消费 Host 注入 executable 的 remote OpenCode Server session。
- `src-tauri/src/agent/codex_desktop_ipc/`：完全独立的 Desktop socket/owner 生命周期。
- `frontend/App.svelte`、`frontend/lib/api.ts`、`frontend/lib/agentRuntime.ts`：运行时设置 UI。

## 风险与验证

- 登录 shell 或版本命令挂起：三秒超时测试。
- 损坏的高优先级自动候选遮挡有效 app：resolver 测试验证继续尝试。
- 无效手动路径覆盖旧配置：配置事务测试验证 settings 不变。
- refresh 串 Provider 或误重启 companion：Codex、Claude 与 OpenCode 使用独立 generation/lock，只更新/restart 目标 Provider；state 边界静态审查确认不操作 companion。
- executable ready 被误当成 session ready：UI 文案和 Provider handshake 测试分别验证。
- App Server refresh 期间请求中断：旧 Provider process 先关闭，调用方得到 remote unavailable/error；不得转发到 companion。

## 测试计划

- Rust resolver：配置优先、无候选、无效配置、自动候选继续和 settings round trip。
- Rust bridge：Provider Host 与 companion registry/lifecycle 独立；分别 unavailable 时另一侧仍工作。
- 真实 fixture subprocess：Host 从 manifest 启动 Provider binary，完成 initialize 与完整 remote Provider 请求闭环。
- 前端 runtime 映射、Tauri 参数和 production build。

## 知识沉淀

- remote App Server 见 `codex-app-server.md`。
- Desktop companion 见 `codex-desktop-companion.md`。
- 双链路决策见 `../../50-decisions/codex-remote-and-desktop-companion-dual-channel.md`。
- Provider 配置权威与安装见 `../../10-architecture/codex-provider-plugin-runtime.md`。
- Claude 最小能力与机器接口见 `../../10-architecture/claude-provider-plugin-runtime.md`。
- OpenCode Server 能力与安装见 `../../10-architecture/opencode-provider-plugin-runtime.md`。

## 未知项

- App Server 子进程退出后的持续 supervisor/reconciliation 尚未实现。
- Windows 动态布局本次未做实机验证。
