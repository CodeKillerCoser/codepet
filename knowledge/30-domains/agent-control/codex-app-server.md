# Codex App Server 回复路径

> 当前状态（2026-08-26）：Codex 原生侧已经改为可长期持有的 stdio JSON-RPC session，并提供 Code Pet Standard Protocol v0 的纯映射边界。Runtime Gateway Provider 尚未接线；旧 Hook、audit 和 transcript 数据源没有恢复。

## 当前用途

Codex 是当前已验证的主要远程回复 provider。`src-tauri/src/agent/actions.rs` 会把符合条件的 Codex 回复路由到 `src-tauri/src/agent/codex_app_server.rs`。当前前端 Codex capability 仍然关闭，因此这是保留的公共兼容入口；新 Runtime Gateway Provider 接线后可以直接持有同一模块提供的 session。

Codex 激活也由同一个 provider driver 处理，但它使用 `codex://threads/<thread-id>` deeplink，而不是 app-server RPC。

## 兼容入口的能力边界

旧前端只在以下条件同时满足时暴露 Codex 回复：

- 事件状态是 `done` 或 `failed`。
- 事件包含非空 `sessionId`。

这与后端保留的 `is_replyable_event()` 和 `has_session_id()` 一致。Codex Hook/audit 活动源和前端 Codex capability 已停用，因此不能把这段兼容判断视为新的生产数据源。

## 已验证能力

此前本机探测已验证：Codex app-server 可以把消息发送到现有 Codex app thread，并在 app UI 上屏。同一路径没有验证窗口激活或打开指定 thread 的 RPC，因此激活继续使用独立 thread deeplink。

## Windows 路径

Windows dev 模式下，Tauri 子进程不能假设 `codex` 一定在 PATH 中。`src-tauri/src/agent/codex_app_server.rs` 需要先尊重 `CODE_PET_CODEX_BIN`，再查找 `%LOCALAPPDATA%\OpenAI\Codex\bin\<hash>\codex.exe` 和 Codex Store 包的 LocalCache 路径，最后才回退到 PATH 中的 `codex`。

## 背景

旧实现只服务桌宠快捷回复：每次回复启动一个 `codex app-server`，同步执行 `initialize`、`thread/resume` 和 `turn/start` 或 `turn/steer`，等到 `turn/completed` 后销毁进程。它不能被未来 Provider 长期管理，也不能并发匹配 response、持续消费通知或接收 App Server 发起的审批请求。

## 当前实现

`src-tauri/src/agent/codex_app_server/` 分为三层：

- `client.rs`：长期 session、stdio JSONL、request ID pending map、持续 reader、stderr drain、进程退出和协议错误转换、显式 `shutdown`，以及可替换的 `JsonRpcReader`/`JsonRpcWriter`。
- `protocol.rs`：只覆盖当前产品需要的 Codex thread、turn、输出和审批字段；未识别原生字段由 serde 忽略，不扩大公共 native API。
- `mapper.rs`：把 typed Codex 数据投影到 `runtime_gateway/generated.rs` 中的 Provider、Conversation、TurnTask、Approval、ProtocolEvent 和 ProtocolError。

`CodexAppServerSession` 在一次 initialize 后可被克隆和长期持有。多个请求共享一个 writer，response 按字符串或整数 request ID 分发；reader 在请求之间持续接收 notification 和 server request。EOF、非法 JSON、非法 envelope、未知 response ID 和 RPC error 都转换为 `CodexAppServerError`。显式 `shutdown` 会关闭 writer、终止受管子进程并解除 pending request。

旧 `send_reply(PetEvent, message)` 与 `codex://threads/<thread-id>` deeplink 保留。回复兼容入口通过进程内共享 session 复用同一个 App Server，不再按操作 spawn；未来 Provider 可以直接持有 `CodexAppServerSession` 并自行决定生命周期。

## 已接入方法

- 初始化：`initialize`，随后发送 `initialized`。
- 会话：`thread/list`、`thread/read`、`thread/resume`、`thread/start`。
- Turn：`turn/start`、`turn/steer`、`turn/interrupt`。
- 审批回应：`item/commandExecution/requestApproval` 和 `item/fileChange/requestApproval` 的 JSON-RPC response。

`thread/start` 的映射规则：

- 有 `workspaceRoot` 的 project chat 写入 `cwd`；normal chat 不发送 `cwd`。
- `read-only` → `sandbox=read-only`、`approvalPolicy=on-request`。
- `workspace-write` → `sandbox=workspace-write`、`approvalPolicy=on-request`。
- `full-access` → `sandbox=danger-full-access`、`approvalPolicy=never`。
- model 写入 `model`；reasoning effort 写入受支持的 thread config，并可在 `turn/start.effort` 覆盖。

## 已接入通知和 server request

- `thread/started` → `conversation.upserted`。
- `turn/started`、`turn/completed` → `turn.upserted`，覆盖 `inProgress`、`completed`、`failed`、`interrupted`。
- `item/agentMessage/delta` → `turn.outputDelta` 的 `assistant-message`。
- `item/commandExecution/outputDelta` → `turn.outputDelta` 的 `command-output`。
- `item/fileChange/outputDelta` → `turn.outputDelta` 的 `file-change-output`。
- `item/reasoning/textDelta`、`item/reasoning/summaryTextDelta` → `turn.outputDelta` 的 `reasoning`。
- command/file approval server request → `approval.requested` 和 waiting-approval `turn.upserted`。
- `serverRequest/resolved` → 尚在本 mapper pending 集合中的 `approval.resolved`；无法获知外部 decision 时标记为 expired。

## 审批归一化限制

Standard Protocol v0 只有 `approve`/`deny`。command/file 原生 decision 可能包含 `acceptForSession`、`cancel` 或策略 amendment；当前最小回应固定映射为 `approve → accept`、`deny → decline`。原生方法、item ID、可用 decisions 和该映射写入受控 ProviderExtension，便于 UI 展示限制，但不会把完整原生 payload 透传到标准 DTO。

`item/permissions/requestApproval` 可以被识别并投影为 Approval，但 v0 的二元 decision 无法构造 App Server 要求的 granted permission profile，因此 resolve 显式返回 `capability_unsupported`。`item/tool/requestUserInput`、`mcpServer/elicitation/request` 和 dynamic tool call 不是二元审批，当前作为不支持的 server request 返回 ProtocolError，不伪造成功。

## ProviderExtension 策略

extension namespace 固定为 `codex.app-server`。只允许 mapper 明确挑选的标量或短列表进入 extension，例如 native method/status、native cwd、item ID、原生可用 decisions 和 decision mapping。未知 JSON 字段被忽略；不存在把原始 `serde_json::Value`、完整 params、command action、diff 或任意 future field 直接塞进标准 DTO 的 fallback。

Provider capability 只声明 v0 已可调用的方法。extension 同时列出 native method 和当前不支持的 approval kind。独立 Codex Desktop 进程是否把实时通知广播给 Code Pet 管理的 App Server 仍未验证，因此 capability 只承诺 managed conversation 的实时事件。

## 已知限制

- Runtime Gateway Provider 尚未接线，session 目前仅由兼容回复入口共享或供后续 Provider 直接持有。
- 当前没有自动重启、重连和 reconciliation；进程退出会使 pending request 和订阅者收到显式错误。
- thread/list 和 thread/read 响应本身不足以区分 normal/project；mapper 只使用 Gateway/创建请求明确提供的 `workspaceRoot`，不会仅凭 App Server 的默认 cwd 猜成 project chat。
- 未接入完整 item history、diff、usage、model discovery、permission profile discovery、user input 和 MCP elicitation。

## 验证

- `cargo test --manifest-path src-tauri/Cargo.toml codex_app_server --lib -- --test-threads=1`
- `cargo check --manifest-path src-tauri/Cargo.toml`
- `npm run protocol:check`

mock peer 测试不启动真实 Codex，覆盖 initialize 顺序、乱序 response ID、持续通知、normal/project thread start、list/read/start、turn start/steer/interrupt、turn 状态、输出增量、command approval request/response，以及旧 reply/deeplink capability。
