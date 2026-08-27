# Agent Runtime 检测与配置

## 背景

主 App 从 Finder、Dock 或登录项启动时，进程继承的 `PATH` 往往不同于用户的交互式终端。把 `/Applications/...` 等候选路径写进 Codex Provider 只能覆盖某一种安装形态，也会把“本机如何找到 Agent”与 Provider 协议实现耦合。Code Pet 因此需要一个独立的 Runtime 层：统一发现、验证和持久化本机 Agent 可执行文件，Provider 只消费解析结果。

## 目标

- 用同一 `AgentRuntime` DTO 表达 Codex、Claude Code 和 OpenCode 的检测、配置和诊断状态。
- 用户配置优先；自动检测覆盖当前 `PATH`、登录 shell、macOS Spotlight 和 Windows 本地安装布局等动态来源。
- 保存前验证路径存在、是文件、具有执行权限，并在限时内通过固定的轻量版本探测。
- Codex App Server 只用 resolver 返回的绝对路径启动；找不到 runtime 时公开具体 unavailable 原因。
- 配置变更后替换 Code Pet 管理的 Codex Provider session，不要求修改环境变量或重启整个 App。

## 非目标

- 本阶段不实现 Claude Code 或 OpenCode 的 Provider 协议。
- 不从前端暴露任意命令、参数或通用 shell 执行接口。
- 不承诺通过固定应用名或固定绝对安装目录覆盖所有第三方分发方式。

## 现状理解

`src-tauri/src/agent/runtime.rs` 是检测和解析事实来源。Provider-specific 信息只存在于 descriptor，包括命令名、可选环境变量、版本参数、bundle identifier、bundle 内相对 executable，以及 Windows 动态安装布局。Codex descriptor 当前使用 bundle identifier `com.openai.codex` 和相对资源 `Contents/Resources/codex`；Spotlight 返回实际 app 位置，detector 不假设 app 名或 `/Applications/Codex.app`。

`AgentRuntime` 包含 `providerId`、`displayName`、`status`、`resolvedExecutable`、`source`、`configuredExecutable`、`version` 和结构化 `diagnostic`。状态分为：

- `ready`：候选通过文件与版本验证。
- `unavailable`：没有可用自动候选。
- `invalid-configured-executable`：已保存的手动路径失效；此时不静默回退自动检测，用户需要修复或清除配置。

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

文件选择复用 Tauri dialog plugin，后端仍负责最终验证。全量刷新或 Codex 配置变更会关闭旧的兼容回复 session，创建新的 `CodexProviderAdapter` 并替换 registry 项；被替换 adapter 会先标记 retired，避免旧进程退出事件覆盖新 Provider 状态。新的 adapter 将状态变化通过 Runtime Gateway event bus 发出。

`CodexAppServerSession::spawn()` 调用 runtime service，并将 `resolvedExecutable` 传给固定的 `app-server --listen stdio://` 命令构造器。没有 resolved path 时 session 返回带 runtime 诊断的 `Spawn` 错误；Provider 以 `unavailable` 状态及 extension 中的 `unavailableReason` 对外展示。不存在裸字符串 `codex` fallback。

### UI

`frontend/App.svelte` 的独立“运行时”Tab 展示三类 runtime 的状态、当前路径、来源、版本、手动配置和诊断，提供检测、选择路径、恢复自动检测和全量重新检测。Codex 卡片说明配置会重启 Code Pet 管理的 session；Claude/OpenCode 卡片只承诺检测与配置。

## 涉及模块

- `src-tauri/src/agent/runtime.rs`：descriptor、候选发现、优先级、验证、DTO 和配置事务。
- `src-tauri/src/app/settings.rs`：持久化 `agentRuntimes`，并为旧 settings 提供 serde default。
- `src-tauri/src/lib.rs`：受控 Tauri commands、settings 写边界和 Provider 刷新协调。
- `src-tauri/src/agent/codex_app_server/`：消费 resolved path、公开 unavailable 原因、退休旧 session。
- `src-tauri/src/runtime_gateway/`：替换 Codex adapter，并把具体 unavailable 原因传给调用方。
- `frontend/App.svelte`、`frontend/lib/api.ts`、`frontend/lib/agentRuntime.ts`：运行时 Tab、IPC 和展示状态映射。

## 风险

- 登录 shell rc 脚本或版本命令挂起：发现命令和版本探测均有三秒超时；测试 timeout/error 诊断路径。
- 损坏的高优先级自动候选遮挡有效 app：resolver 测试覆盖跳过无效自动候选。
- 无效手动路径覆盖原配置：配置事务测试确认验证失败后 settings 不变。
- Codex 配置变化后旧 adapter 发送迟到状态：retired 标志抑制旧事件；Provider 测试覆盖 unavailable 状态映射。
- UI 长路径和诊断挤压操作：原生窗口检查三张卡、文件 picker、按钮可见性；浏览器检查 760px 断点和横向溢出。
- 平台发现元数据漂移：descriptor 保持集中，Windows/macOS 需要在对应平台运行 detector 和 `cargo check`。

## 测试计划

- Rust：resolver 配置优先、无候选、无效手动路径、不覆盖旧配置、自动候选继续、settings DTO round trip、Codex command 使用 resolved path、Provider unavailable 原因。
- 前端：runtime status/source 映射、单项替换、恢复自动检测可用性和 Tauri command 参数。
- 构建：`cargo check --manifest-path src-tauri/Cargo.toml`、相关 Rust tests、全量 Vitest、`npm run build`。
- 人工：在原生 Tauri App 中确认 Codex 通过 Spotlight 解析实际资源并显示版本；Claude/OpenCode 显示检测状态；文件选择器只能选择文件；窄屏无横向溢出。

## 知识沉淀

本页是 Agent Runtime 的领域事实来源。settings 字段继续在 settings 文档维护；Codex App Server 文档只记录它消费 resolver 结果，不再维护候选路径。

## 未知项

- Claude Code 和 OpenCode 的正式 Provider 接入及其平台 app bundle metadata 尚未实现。
- Windows 动态布局保留既有模型，但本次未在 Windows 实机验证。
- macOS Spotlight 被用户关闭或索引未完成时，仍需依赖 PATH、登录 shell 或手动选择。
