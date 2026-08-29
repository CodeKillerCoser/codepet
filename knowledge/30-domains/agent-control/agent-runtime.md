# Agent Runtime 检测与配置

## 背景

主 App 从 Finder、Dock 或登录项启动时，进程继承的 `PATH` 往往不同于用户的交互式终端。Code Pet 保留一个独立的 Runtime 层，用于发现、验证和持久化本机 Agent 可执行文件。这个模型现在是安装信息与未来 Provider 的公共基础设施，不再是 Codex Desktop Provider 的连接前置条件。

Codex Desktop Provider 的当前事实来源是 `~/.codex/ipc/ipc.sock` 上的私有 Owner/Follower IPC。它的可用性由 socket 安全检查和 initialize/握手决定；Codex CLI 是否被检测到、手动路径是否有效，都不能改变该 Provider 的状态或数据来源。

## 目标

- 用同一 `AgentRuntime` DTO 表达 Codex、Claude Code 和 OpenCode 的检测、配置和诊断状态。
- 用户配置优先；自动检测覆盖当前 `PATH`、登录 shell、macOS Spotlight 和 Windows 本地安装布局等动态来源。
- 保存前验证路径存在、是文件、具有执行权限，并在限时内通过固定的轻量版本探测。
- 明确区分 executable 检测状态与 Provider 连接状态，避免把“未找到 CLI”显示成 Desktop IPC unavailable。
- executable 配置变化只更新 Runtime 设置和检测结果，不重启、不替换 Codex Desktop IPC Provider，也不切换任务数据源。

## 非目标

- 本阶段不实现 Claude Code 或 OpenCode 的 Provider 协议。
- 不从前端暴露任意命令、参数或通用 shell 执行接口。
- 不承诺通过固定应用名或固定绝对安装目录覆盖所有第三方分发方式。
- 不用 executable resolver 启动 Codex App Server，也不把它作为 Desktop IPC 的 fallback。

## 现状理解

`src-tauri/src/agent/runtime.rs` 是检测和解析事实来源。Provider-specific 信息只存在于 descriptor，包括命令名、可选环境变量、版本参数、bundle identifier、bundle 内相对 executable，以及 Windows 动态安装布局。Codex descriptor 当前使用 bundle identifier `com.openai.codex` 和相对资源 `Contents/Resources/codex`；Spotlight 返回实际 app 位置，detector 不假设 app 名或 `/Applications/Codex.app`。

`AgentRuntime` 包含 `providerId`、`displayName`、`status`、`resolvedExecutable`、`source`、`configuredExecutable`、`version` 和结构化 `diagnostic`。状态分为：

- `ready`：候选通过文件与版本验证。
- `unavailable`：没有可用自动候选。
- `invalid-configured-executable`：已保存的手动路径失效；此时不静默回退自动检测，用户需要修复或清除配置。

这些状态只回答“Code Pet 是否找到并验证了某个 executable”，不回答“对应 Provider 是否已连接”。例如 Codex runtime 可以是 `unavailable`，而 Codex Desktop IPC Provider 仍然 `ready`；也可以检测到 Codex CLI，但 Desktop 未运行，因而 Provider 是 `unavailable`。

Claude Code 和 OpenCode 进入相同列表、检测和配置模型，但 UI 明确标注 Provider 协议未接入。

## 实现路径

### 解析优先级

1. `AppSettings.agentRuntimes.byProvider.<provider>.configuredExecutable`。
2. descriptor 声明的兼容环境变量；Codex 保留 `CODE_PET_CODEX_BIN`。
3. 当前进程 `PATH` 中实际存在的命令文件。
4. 登录 shell 的 `command -v` 结果。
5. 平台动态发现：macOS 使用 Spotlight 按 bundle identifier 查询 app，再拼接 descriptor 的 bundle 内相对资源；Windows 从 `LOCALAPPDATA` 按 descriptor 布局扫描版本目录和 Store package 目录。

自动候选只有实际存在时才进入验证。一个自动候选失败后可以继续尝试后续来源，例如损坏的 PATH wrapper 不应阻止有效的 macOS app 资源。手动配置失败则停止解析，以免 UI 显示的配置与实际使用路径不一致。

### 验证与持久化

`SystemExecutableValidator` 先检查 metadata 和执行权限，再 canonicalize 为绝对路径，最后使用 descriptor 固定的 `--version` 参数做三秒限时探测。前端只能提交 provider id 和文件 picker 返回的路径，不能提交命令参数。

`set_agent_runtime_executable` 只有在完整验证成功后才写入 settings；失败保留原配置。`clear_agent_runtime_executable` 删除该 provider 的手动项并重新自动检测。通用 `update_app_settings` 会保留后端已有的 `agentRuntimes`，避免绕过专用验证 API 修改 executable 输入。

### Tauri 与生命周期

主 App 暴露固定动作：

- `list_agent_runtimes`
- `detect_agent_runtime`
- `refresh_agent_runtimes`
- `set_agent_runtime_executable`
- `clear_agent_runtime_executable`

文件选择复用 Tauri dialog plugin，后端仍负责最终验证。全量检测或单个 executable 配置变化只刷新 `AgentRuntime` DTO 和持久化设置，不关闭、重启或替换 Runtime Gateway 中的 Codex Desktop adapter。

Codex Desktop adapter 独立解析和连接 IPC socket。socket 缺失、类型或权限不安全、initialize/握手失败时，由 adapter 公开 Provider unavailable 及诊断原因；这一判断不读取 `resolvedExecutable`，也不存在 App Server 或裸字符串 `codex` fallback。

### UI

`frontend/App.svelte` 的独立“运行时”Tab 展示三类 executable 的检测状态、当前路径、来源、版本、手动配置和诊断，提供检测、选择路径、恢复自动检测和全量重新检测。这些控件可以保留，但 Codex 卡片必须说明它们不影响 Desktop IPC 连接；Provider 连接诊断由 Runtime Gateway 状态展示。

## 涉及模块

- `src-tauri/src/agent/runtime.rs`：descriptor、候选发现、优先级、验证、DTO 和配置事务。
- `src-tauri/src/app/settings.rs`：持久化 `agentRuntimes`，并为旧 settings 提供 serde default。
- `src-tauri/src/lib.rs`：受控 Tauri commands 和 settings 写边界；runtime 配置动作不得驱动 Codex Provider 生命周期。
- `src-tauri/src/agent/codex_desktop_ipc/`：独立于 executable resolver 的 Desktop socket、协议和 follower 状态。
- `src-tauri/src/runtime_gateway/`：从 Codex Desktop adapter 获取 Provider 状态和安全诊断，不读取 runtime 路径判断可用性。
- `frontend/App.svelte`、`frontend/lib/api.ts`、`frontend/lib/agentRuntime.ts`：运行时 Tab、IPC 和展示状态映射。

## 风险

- 登录 shell rc 脚本或版本命令挂起：发现命令和版本探测均有三秒超时；测试 timeout/error 诊断路径。
- 损坏的高优先级自动候选遮挡有效 app：resolver 测试覆盖跳过无效自动候选。
- 无效手动路径覆盖原配置：配置事务测试确认验证失败后 settings 不变。
- UI 把 executable `ready` 误写成 Provider 已连接：文案和测试必须区分 Runtime 状态与 Runtime Gateway Provider 状态。
- runtime 配置动作误触发 Codex adapter 重启：生命周期测试或静态检查确认两条路径解耦。
- UI 长路径和诊断挤压操作：原生窗口检查三张卡、文件 picker、按钮可见性；浏览器检查 760px 断点和横向溢出。
- 平台发现元数据漂移：descriptor 保持集中，Windows/macOS 需要在对应平台运行 detector 和 `cargo check`。

## 测试计划

- Rust：resolver 配置优先、无候选、无效手动路径、不覆盖旧配置、自动候选继续和 settings DTO round trip；另验证 Codex Desktop Provider 不读取 resolved path、runtime 配置变化不重启 adapter。
- 前端：runtime status/source 映射、单项替换、恢复自动检测可用性和 Tauri command 参数。
- 构建：`cargo check --manifest-path src-tauri/Cargo.toml`、相关 Rust tests、全量 Vitest、`npm run build`。
- 人工：在原生 Tauri App 中确认 Codex 通过 Spotlight 解析实际资源并显示版本；Claude/OpenCode 显示检测状态；文件选择器只能选择文件；窄屏无横向溢出。

## 知识沉淀

本页是 Agent executable 检测与配置的领域事实来源。settings 字段继续在 settings 文档维护；Codex Desktop IPC 的连接、状态和诊断见 `codex-app-server.md`，两者不得重新耦合。

## 未知项

- Claude Code 和 OpenCode 的正式 Provider 接入及其平台 app bundle metadata 尚未实现。
- Windows 动态布局保留既有模型，但本次未在 Windows 实机验证。
- macOS Spotlight 被用户关闭或索引未完成时，仍需依赖 PATH、登录 shell 或手动选择。
- Codex executable 检测在长期产品中是否仍需保留，尚未决定；删除或保留都不能改变 Desktop IPC Provider 行为。
