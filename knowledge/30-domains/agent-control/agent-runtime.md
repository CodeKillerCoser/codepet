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
- 本页不提供远程网络 transport 配置；LAN 接入由独立 Remote 模块负责。

## 当前模型

Host 连接页通过 Provider 的 `runtime.getInstalled` 读取当前安装及选择；`src-tauri/src/agent/runtime.rs` 保留本机 Runtime DTO 与旧 resolver 工具。`AgentRuntime` 状态为：

- `loading`：Provider 尚未完成初始化，列表将在状态变化后自动重读。
- `ready`：候选通过文件与版本验证。
- `unavailable`：没有可用自动候选。
- `invalid-configured-executable`：保存的手动路径失效，不静默回退。

Runtime 的安装发现、版本校验和当前选择由各 Provider 的 `runtime.getInstalled` / `runtime.select` 实现。Host 将协议结果投影为卡片，并持久化用户选择；Server 或每 turn CLI 由对应 Provider 根据自己的实例生命周期启动。Runtime 诊断与 Provider 连接、Harness 就绪分别显示。Codex Desktop companion 使用独立 IPC，不参与本机 Runtime 选择。

## 实现路径

### 解析与验证

Provider 负责候选发现和版本验证，Host 通过 `runtime.select` 校验用户选择。`src-tauri/src/agent/runtime.rs` 仍保留 descriptor、限时探测及 resolver 测试，但当前连接页的安装列表来自 Provider API。前端只提交 Provider ID 和文件 picker 返回的路径。

### Tauri 与双生命周期

运行时命令为 `list_agent_runtimes`、`detect_agent_runtime`、`refresh_agent_runtimes`、`set_agent_runtime_executable` 和 `clear_agent_runtime_executable`。

`provider_manager_config` 将持久化选择放入 `PluginManagerConfig.runtime_selections`；启动时由 manager 调用插件的 `runtime.select`。列表读取与重新检测只查询 Provider，不重启插件。设置路径时，`ProviderHostState::select_runtime` 先通过 Provider 校验，记住选择并重启对应插件；恢复自动检测从 Provider 返回的安装中选择默认候选，再清除持久化手动路径。重启期间的连接事件驱动卡片自动重读。

这些动作不操作 companion、Hook 或 Pet。Codex Desktop companion 在独立 state/registry/event bus 中，由 socket 自行连接和重连。支持 Server 模式的 Harness 由 Provider SDK 的连接心跳协调生命周期，详见 `../../10-architecture/remote-and-provider-connections.md`。

### UI

主窗口运行时页展示 executable 路径、来源、版本和诊断。这里的 `ready` 表示 executable 已验证，不等于 Provider session 已 initialize，更不等于 Desktop companion 可用。三个 remote Provider 的诊断应在各自 channel 展示。

连接状态由 `frontend/lib/providerRuntimes.ts` 在主窗口统一订阅；Provider 连接状态或 generation 变化会使 Runtime 列表失效并自动重读，普通 Harness 状态变化仅更新标签。首次快照及 Runtime 查询均有晚到响应保护，初始化等待不显示 `provider-unavailable`。排查证据见 `../../40-runbooks/host-provider-startup-stale-runtime.md`。

## 涉及模块

- `src-tauri/src/agent/runtime.rs`：descriptor、候选发现、验证和 DTO。
- `src-tauri/src/app/settings.rs`：持久化 `agentRuntimes`。
- `src-tauri/src/lib.rs`、`src-tauri/src/runtime_gateway/tauri_bridge.rs`：运行时 commands、Provider API 投影与选择后的插件重启。
- `crates/providers/codepet-provider-codex/`：负责自己的 Runtime API 与共享 App Server。
- `crates/providers/codepet-provider-claude/`：负责自己的 Runtime API 与官方 CLI stream-json session。
- `crates/providers/codepet-provider-opencode/`：负责自己的 Runtime API 与共享 OpenCode Server。
- `src-tauri/src/agent/codex_desktop_ipc/`：完全独立的 Desktop socket/owner 生命周期。
- `frontend/App.svelte`、`frontend/lib/api.ts`、`frontend/lib/agentRuntime.ts`：运行时设置 UI。

## 风险与验证

- 登录 shell 或版本命令挂起：三秒超时测试。
- 损坏的高优先级自动候选遮挡有效 app：resolver 测试验证继续尝试。
- 无效手动路径覆盖旧配置：配置事务测试验证 settings 不变。
- refresh 串 Provider 或误重启 companion：Codex、Claude 与 OpenCode 使用独立 generation/lock，只 restart 目标 Provider；state 边界静态审查确认不操作 companion。
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

- 真实安装版各平台发现候选的完整布局未全部覆盖；心跳重试与实例恢复边界见连接架构文档。
- Windows 动态布局本次未做实机验证。
