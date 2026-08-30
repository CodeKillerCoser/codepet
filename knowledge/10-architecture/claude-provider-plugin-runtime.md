# Claude Provider 插件运行时

## 当前结论

Claude 默认 Provider 已实现为独立 Rust 二进制 `crates/providers/codepet-provider-claude`。它只通过生成的 `codepet-provider-sdk` 与 Host 交换 Provider Protocol v1 JSON-RPC / stdio JSON-lines，并把 Host resolver 注入的 Claude executable 作为唯一启动路径。Provider crate 不依赖 Host、Gateway、Tauri、Pet SDK、Desktop IPC、Hook collector 或 activity store。

当前官方公开接口中未发现与 Codex App Server 等价、可由 Rust 直接消费的完整 Claude session server。当前诚实切面是官方 Claude Code CLI 的 `--print` + 双向 `stream-json`：Provider 管理自己创建的 session ID，一次 turn 启动一个 CLI 子进程，后续 turn 用 `--resume` 恢复。它不宣称能枚举、读取、附着或同步 Claude Desktop/CLI 的其他会话。

## 审计证据

### 仓库事实

- `protocol/provider/v1` 与 `sdk/rust/codepet-provider-sdk` 已生成 Provider DTO、四段 `RoutedResourceId`、server trait、dispatcher 和有界 `JsonLineCodec`；Claude Provider 直接复用它们，没有复制协议类型。
- `crates/codepet-host` 已按显式 manifest 管理独立 Provider 进程、instance lifecycle 与 Gateway event/replay；Claude 作为普通 `dev.codepet.claude` manifest 接入，没有新增注册市场或生命周期框架。
- `src-tauri/src/agent/runtime.rs` 已有 Claude descriptor，负责候选发现、`--version` 验证和绝对路径 canonicalize。`tauri_bridge.rs` 只把这个 resolver 结果写入 `claudeExecutable`；Provider 内没有 PATH、应用目录或扩展目录探测。
- 旧 Claude Hook、transcript 和桌宠 activity 代码仍属于 Pet/观察链路，不是 Provider 输入。Provider 每次调用 CLI 时使用官方单次运行设置关闭用户/项目 Hook，且不读取 transcript。

### 官方接口

- [Agent SDK overview](https://code.claude.com/docs/en/agent-sdk/overview) 明确 SDK 只有 Python/TypeScript；其他语言应以 `-p` 启动 CLI 子进程。因此没有引入 Node/Python SDK sidecar。
- [CLI reference](https://code.claude.com/docs/en/cli-reference) 定义 `--print`、`--input-format stream-json`、`--output-format stream-json`、`--include-partial-messages`、`--session-id`、`--resume`、`--permission-mode`、model 与 effort 参数。
- [Programmatic/headless guide](https://code.claude.com/docs/en/headless) 定义 NDJSON streaming、`system/init`、`stream_event` text delta、最终 `result`、按 session ID 恢复，以及 SIGINT 正常结束当前 turn 的语义。
- [Hooks reference](https://code.claude.com/docs/en/hooks) 明确 `--settings '{"disableAllHooks":true}'` 可为单次运行覆盖用户、项目和本地 Hook；管理员托管 Hook 仍只能由托管设置关闭，这一限制不能被 Provider 伪装消除。
- [Approval/user input guide](https://code.claude.com/docs/en/agent-sdk/user-input) 把可交互审批定义为 SDK `canUseTool` 回调；CLI 的 `--permission-prompt-tool` 又要求 MCP tool。当前边界禁止 SDK sidecar/MCP 审批桥，因此 `approval.resolve` 必须关闭。

### 本机实测

2026-08-30 在 VS Code 扩展内的官方 Claude Code 2.1.251 上独立验证：

- `--version` 返回 `2.1.251 (Claude Code)`，`auth status --json` 返回未登录；PATH resolver 当前不会自动选择扩展私有 binary。
- 使用临时、无持久化 session 运行真实 `stream-json` 后，依次观察到 `command_lifecycle`、`system/init`、`system/status`、带 `authentication_failed` 的 `assistant` 和最终 `result`。
- 真实失败结果的 `subtype` 仍是 `success`，但 `is_error=true`、`terminal_reason=api_error` 且进程退出 1；映射必须以 `is_error` 为准，不能只猜 subtype。
- 脱敏后的实际输出保存在 `tests/fixtures/claude-2.1.251-no-auth.ndjson`，路径、UUID 和非必要本机字段已替换，事件形状保持不变。

## 目标与非目标

目标：

- 提供可安装的 `codepet-provider-claude`、一个显式 manifest 和完整 Provider instance lifecycle。
- 支持 Provider 自己创建的会话、顺序 turn、主 agent 文本增量、终态、Unix SIGINT 和安全权限模式映射。
- 未经验证的输出类型向前兼容忽略；route/session/JSON/framing 错误 fail closed。
- Provider event 只进入 Host/Gateway remote replay/event。

非目标：

- 不实现 SDK sidecar、MCP permission server、Hook、transcript scan、窗口控制、Claude Desktop IPC 或 Agent View 适配。
- 不实现插件市场、签名、sandbox、自动重启/backoff、复杂 DI 或 Provider 协议兼容层。
- 不伪造全局 conversation list/get、Desktop 同步、进程重启恢复、turn steer 或 approval callback。
- 不把 Provider event 投影到 Pet Protocol、`SharedState` activity、companion replay 或 Tauri Pet event。

## 数据链与配置权威

唯一业务数据链：

```text
Claude Code CLI 官方 stream-json
  <-> codepet-provider-claude
  <-> Provider Protocol v1 / stdio JSON-RPC
  <-> PluginProcess / PluginManager
  <-> ProviderGatewayService remote replay/event
```

明确不存在的支路：

```text
Provider -X-> Claude Hook / transcript
Provider -X-> Pet Protocol / activity store
Provider -X-> CodexDesktopCompanionState / Desktop IPC
Provider -X-> codex-desktop-companion-event / pet-event
```

配置权威分工：

| 配置 | 唯一权威 | 行为 |
| --- | --- | --- |
| Provider binary 与 manifest | `codepet-provider.json` | Catalog 解析相对 binary 路径。 |
| Claude executable | `AgentRuntimeService` Claude resolver | Host 注入绝对 `claudeExecutable`；无结果时删除该 setting，instance create/start 明确失败。 |
| workspace、permission、model、effort | `conversation.create` 请求 | Provider 验证后转成当前 CLI 官方 flag；不写第二份全局配置。 |
| native session ID | Provider process | create 时生成 UUID；首 turn 使用 `--session-id`，后续 turn 使用 `--resume`。 |

应用启动、runtime refresh、set 和 clear 都通过同一 Host target 更新 `dev.codepet.claude` 的 `claude` instance setting，并只重启该 Provider。Claude Provider 自身从不搜索 PATH、VS Code 扩展或用户目录。

## Provider v1 能力矩阵

| 方法 | 状态 | 真实映射 |
| --- | --- | --- |
| `provider.initialize` / `provider.describe` | 支持 | 协商 v1，返回 `dev.codepet.claude` 与 `claude` kind。 |
| `instance.create/start/stop/destroy/capabilities` | 支持 | 严格解码 Host setting，`--version` 验证，管理并终止活动子进程。 |
| `conversation.create` | 支持但为 Provider-managed | 预留 UUID 和选项；第一条消息到达前不伪造上游已有完整 session。 |
| `conversation.list` | 不支持 | CLI 没有覆盖 Provider print sessions 的稳定全局 CRUD/list API。 |
| `conversation.get` | 不支持 | 不扫描 transcript，也不把本进程缓存伪装为上游权威 get。 |
| `turn.start` | 支持 | 一条 raw user NDJSON；首 turn `--session-id`，后续 `--resume`；映射主 agent text delta 与 result。 |
| `turn.interrupt` | Unix 支持，其他平台关闭 | 对当前 CLI 进程发送官方 SIGINT；Windows capability 不广告。 |
| `turn.steer` | 不支持 | 一次一进程/一 turn；不把排队下一条消息伪装为 active-turn steering。 |
| `approval.resolve` | 不支持 | 没有 SDK callback/MCP permission tool，返回 `capability_unsupported`。 |
| `provider.shutdown` | 支持 | 停止全部实例和子进程后退出 stdio loop。 |

权限映射：

| Provider permission | Claude `--permission-mode` | 边界 |
| --- | --- | --- |
| `read-only` | `dontAsk` | 未预先允许的写入/命令由 CLI 拒绝，不向 Host 伪造审批。 |
| `workspace-write` | `acceptEdits` | 只采用 CLI 自己的 accept-edits 规则；其他需要询问的动作仍被非交互模式拒绝。 |
| `full-access` | `bypassPermissions` | 仅在调用方明确选择 full access 时启用。 |

model 广告使用官方 CLI 的稳定别名 `sonnet`、`opus`、`haiku`、`fable`；也允许 CLI 自己验证显式完整 model 名。effort 广告为本机帮助与官方文档共同确认的 `low`、`medium`、`high`、`xhigh`、`max`。

## 运行时映射

- 输入只序列化已实测的 raw user message：`type=user`、UUID、`message.role=user`、文本 content、`parent_tool_use_id=null`。
- `system/init` 必须带匹配的 `session_id`；它更新 materialized、cwd 和 model。未知 system subtype 被忽略。
- 只发布 `parent_tool_use_id=null` 的 `content_block_delta/text_delta`，避免把 subagent/tool 内部流伪装成主回答。
- `result.is_error`、非 success subtype 或明确取消 reason 决定 terminal status；没有 delta 时才用 `result` 文本补一个最终 delta。
- 输出物理行限制为 16 MiB，stderr 单行诊断限制为 64 KiB；Host framing 继续使用生成 SDK 的 1 MiB `JsonLineCodec` 和标准 JSON-RPC 错误。
- 每个 conversation 同时最多一个活动 turn。route 与 output session 任一不匹配都停止对应 CLI 进程，不跨会话接收数据。

## 影响模块

- `crates/providers/codepet-provider-claude/`：binary、上游 Claude wire parser、process adapter、Provider server、manifest 与 fixture/tests。
- `crates/Cargo.toml`、`crates/Cargo.lock`：Provider workspace member。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs`：把已有 Claude resolver 结果注入 manifest instance，并为 Claude/Codex 分别串行化 runtime refresh。
- `src-tauri/src/lib.rs`：refresh/set/clear 时把 Claude runtime 交给同一 Host refresh 入口。
- 未修改 `src-tauri/src/activity`、`pet`、`agent/claude_transcript`、Desktop companion 或前端事件消费。

## 风险、验证与未知项

- 上游 wire 漂移：实际 2.1.251 fixture 覆盖未知 command lifecycle、init/status、auth error 和 counterintuitive result；未知 top-level/event delta 忽略，缺失/错误 session 或非法 JSON fail closed。
- 假能力：capability 负例直接调用 list/get/steer/approval 并断言标准 `capability_unsupported`；Windows 构建不广告 interrupt。
- session 串线：垂直 fixture 验证首 turn `--session-id`、第二 turn `--resume`、四段 route 与失败 result 映射。
- Hook/Pet 污染：CLI 参数 fixture 要求 `disableAllHooks=true` 且禁止 hook output flag；Provider dependency metadata 必须不含 Host/Gateway/Pet/Tauri。生产 bridge 的既有隔离测试继续断言 Provider event 只进入 remote channel。
- 本机未认证：只完成 `--version`、帮助、auth status 和无持久化真实 wire smoke，没有执行收费的成功模型请求；成功流由严格检查启动参数的真实子进程 fixture 覆盖。
- 管理员托管 Hook 无法被非托管 `disableAllHooks` 覆盖，这是 Claude 官方限制；Code Pet 自己安装的用户 Hook 会被本次 inline setting 关闭。若组织强制 Hook，Provider 不能声称绝对隔离，应由管理员在托管设置中关闭。
- Provider process 重启后不保存 conversation registry；即使 Claude transcript 仍在磁盘，也不扫描或恢复。外部 CLI/Desktop 对同一 session 的改变不会同步回 Provider。

验证命令：

```sh
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-claude --all-targets
cargo test --manifest-path src-tauri/Cargo.toml claude_runtime_executable_is_injected_into_the_manifest_instance --lib
cargo test --manifest-path src-tauri/Cargo.toml --test runtime_gateway_core_tests real_provider_events_only_emit_remote_tauri_channel_and_never_call_desktop_adapter
cargo tree --manifest-path crates/Cargo.toml -p codepet-provider-claude
git diff --check
```
