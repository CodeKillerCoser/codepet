# Claude Provider 插件运行时

## 当前结论

Claude 默认 Provider 是独立 Rust 二进制 `crates/providers/codepet-provider-claude`。它只通过生成的 `codepet-provider-sdk` 与 Host 交换 Provider Protocol v1 JSON-RPC / stdio JSON-lines，并把 Host resolver 注入的绝对 Claude executable 作为唯一启动路径。Provider crate 不依赖 Host、Gateway、Tauri、Pet SDK、Desktop IPC、Hook collector 或 activity store。

当前官方公开接口中未发现与 Codex App Server 等价、可由 Rust 直接消费的完整 Claude session server。诚实切面是官方 Claude Code CLI 的 `--print` + 双向 `stream-json`：Provider 管理自己的 session ID，一次 turn 启动一个 CLI 子进程，后续 turn 用 `--resume`。它不宣称能枚举、读取、附着或同步其他 Claude Desktop/CLI 会话。

## 安全复核证据与根因

独立 review 对初始提交 `3c86e507` 给出 NO-GO。源码复核确认四个根因：

- 只传 `--settings '{"disableAllHooks":true}'` 不会隔离工作区 `.mcp.json`、用户/项目/local settings 或 plugin MCP；`claude -p` 跳过 trust dialog，认证前即可启动 MCP 命令。
- `read-only -> dontAsk` 仍允许外部 `permissions.allow` 预批准 Write/Edit/Bash，不能单独构成只读边界。
- stdout reader 通过同一个 `Child` mutex 调用 `wait`；result 时又提前移除 active control，导致 stop/shutdown 可能等待死锁或留下已终态但仍运行的进程树。
- Claude 上游允许 16 MiB 物理行，而生成 Provider codec 只允许 1 MiB。大 result 被单帧转发时，terminal event 也可能丢失，Host 永久 pending。

引入历史已确认是 `3c86e507` 的首版 Claude Provider；旧仓库代码不存在该实现。

## 官方接口审计

- [Agent SDK overview](https://code.claude.com/docs/en/agent-sdk/overview) 列出的 SDK 是 Python/TypeScript；Rust 应以 `-p` 启动 CLI 子进程，因此本实现不引入 SDK sidecar。
- [CLI reference](https://code.claude.com/docs/en/cli-reference) 定义 `--print`、stream-json、`--session-id`、`--resume`、`--setting-sources`、`--strict-mcp-config`、`--mcp-config`、`--restricted`、`--tools` 与 permission mode。
- [Settings reference](https://code.claude.com/docs/en/settings) 说明 user/project/local settings 的层级、数组权限规则会跨层合并，以及 managed policy 不能被命令行覆盖。
- [MCP reference](https://code.claude.com/docs/en/mcp) 说明 MCP 可来自 user、project、local、plugin 与 managed 配置；CLI 的 strict 模式只采用显式 `--mcp-config`。
- [Permission modes](https://code.claude.com/docs/en/permission-modes) 说明 `dontAsk` 只拒绝未预批准动作；[permission controls](https://code.claude.com/docs/en/agent-sdk/permissions) 明确 locked-down agent 需要显式工具集合配合 `dontAsk`。
- [Environment variables](https://code.claude.com/docs/en/env-vars) 公开定义 `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`，避免 Provider turn 加载或写入目录外的 auto memory。
- [Hooks reference](https://code.claude.com/docs/en/hooks) 说明 managed hook 不能由普通 user/project/local 设置关闭。Provider 因此不发布“所有 Hook 已禁用”的元数据。
- [Programmatic/headless guide](https://code.claude.com/docs/en/headless) 定义 NDJSON、`system/init`、text delta、最终 result、session resume 与 SIGINT。

本机 Claude Code 2.1.251 帮助和真实未登录 wire 已复核上述参数。本轮 smoke 在恶意 `.mcp.json`、user/project/local SessionStart Hook 和 `permissions.allow=[Bash,Write,Edit]` 下执行首 turn 与 resume：两次 `system/init.mcp_servers` 均为空，只读 tools 仅为 Glob/Grep/Read，MCP 与 Hook marker 均未出现，最终都是预期的未登录 result。没有执行收费模型请求。

## 目标与非目标

目标：

- 独立 binary、显式 manifest、生成 Provider SDK、四段 route 和 Host instance lifecycle。
- Provider-managed create/顺序 turn、主 agent 文本增量、终态、Unix interrupt 和明确权限模式。
- user/project/local/plugin MCP 与普通 Hook 不得进入 Provider-launched CLI；任何非空 `system/init.mcp_servers` fail closed。
- result 只有在进程真实退出和 stdout 有界排空后才形成权威 terminal；所有生命周期动作有界回收进程组。
- Provider event 只进入 Host/Gateway remote replay/event。

非目标：

- 不实现 Python/TypeScript SDK sidecar、MCP permission server、transcript scan、窗口控制、Claude Desktop IPC 或 Agent View。
- 不实现市场、签名、sandbox、自动重启/backoff、复杂 DI 或 Provider 协议兼容层。
- 不伪造全局 list/get、Desktop 同步、进程重启恢复、turn steer 或 approval callback。
- 不在本分支实现二进制/manifest 打包发现；Codex/OpenCode/Claude 的统一 macOS universal/Windows 集成由后续集成任务处理。

## 数据链与配置权威

```text
Claude Code CLI 官方 stream-json
  <-> codepet-provider-claude
  <-> Provider Protocol v1 / stdio JSON-RPC
  <-> PluginProcess / PluginManager
  <-> ProviderGatewayService remote replay/event
```

不存在 Provider 到 Hook/transcript、Pet Protocol/activity store、Desktop IPC/companion 或 Tauri Pet event 的支路。普通 Code Pet Claude Hook 位于 user settings；空 setting sources 与 safe mode 使 Provider-launched turn 不加载它。组织 managed hook 仍可能按 Claude 的 policy 层执行，这是外部管理边界，不能标记为已禁用。

| 配置 | 唯一权威 | 行为 |
| --- | --- | --- |
| Provider binary/manifest | `codepet-provider.json` | Catalog 解析相对 binary。 |
| Claude executable | `AgentRuntimeService` | Host 注入绝对 `claudeExecutable`；Provider 不探测。 |
| workspace/permission/model/effort | `conversation.create` | 验证后转为 CLI flag。 |
| native session ID | Provider process | 首 turn `--session-id`，后续 `--resume`。 |
| filesystem settings | Provider 固定为空 | 每 turn `--setting-sources ""`、`--safe-mode` 与 `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`。 |
| MCP | Provider 固定为空 | `--strict-mcp-config --mcp-config '{"mcpServers":{}}'`，init 非空即失败。 |

## Provider v1 能力矩阵

| 方法 | 状态 | 真实映射 |
| --- | --- | --- |
| initialize/describe | 支持 | Provider v1、`dev.codepet.claude`、`claude` kind。 |
| instance lifecycle/capabilities | 支持 | 严格 Host setting、`--version`、有界停止全部进程组。 |
| conversation.create | Provider-managed | 预留 UUID 与选项；首消息前不伪造上游 session。 |
| conversation.list/get | 不支持 | 不扫描 transcript 或缓存冒充上游 CRUD。 |
| turn.start | 支持 | raw user NDJSON、首次 session ID、后续 resume。 |
| turn.interrupt | Unix 支持 | SIGINT 750 ms grace，超时 SIGKILL process group；真实退出后才 terminal。 |
| turn.steer | 不支持 | queued input 不等价于 active-turn steering。 |
| approval.resolve | 不支持 | 无 SDK callback/MCP permission tool。 |
| provider.shutdown | 支持 | 尝试回收所有 instance；任一回收失败则返回错误，不静默 accepted。 |

权限映射：

| Provider permission | CLI 参数 | 边界 |
| --- | --- | --- |
| read-only | `--restricted` + `dontAsk` + `--tools Read,Glob,Grep` | 不加载 user/project/local allow，移除命令执行工具并限制文件工具；init 出现任何其他 tool 即失败。 |
| workspace-write | `acceptEdits` | 采用 CLI accept-edits 语义；没有 Host 审批回写。 |
| full-access | `bypassPermissions` | 仅调用方明确选择时启用。 |

## 进程与输出模型

- 每个 turn 建立独立 process group。一个后台 reaper 独占 `Child` 并负责 `wait`；control 只保存 PID/process-group ID 与 Condvar 退出通知，绝不在 wait 上持 child mutex。
- stdout reader、child reaper 和完成协调相互独立。result/aborted 只记录 pending completion；进程真实退出、stdout 排空后才发布 terminal 并删除 active。此间下一 turn 必须返回 `turn_already_active`。
- interrupt、instance.stop、destroy 和 shutdown 都保留 control，先给有界 grace，再杀整个 process group并等待 reaper。reaper 在 root 退出后再次清理 group 中的后代。
- Claude stdout 单物理行硬限制为 4 MiB；超过限制且无换行时立即关闭 reader并杀进程组。stderr 单行限制 64 KiB。
- Provider 生成 codec 的 frame 上限是 1 MiB。所有对外 text delta/result 按 UTF-8 边界切为最多 64 KiB；未知 usage 等任意 JSON 不复制进 terminal extension，终态 metadata 限制为 4 KiB。
- 发送 output event 失败时 active 不会先删除；真实退出后改发尺寸安全的 failed terminal。terminal 自身发送失败则保留 exited active 并把错误返回给 lifecycle waiter，不能伪装成功。

## 涉及模块

- `crates/providers/codepet-provider-claude/src/client.rs`：固定 CLI 隔离参数、process group、reaper、退出通知与上游 line limit。
- `src/protocol.rs`：实际 wire 的 tools 与 mcp_servers 字段。
- `src/provider.rs`：init 防线、只读工具验证、延迟终态、分块与生命周期回收。
- `tests/fixtures/claude_stream.rs`、`tests/provider_vertical.rs`：真实参数、2 MiB、发送失败和进程探针。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs`：只负责 resolver setting 注入，本轮无新增逻辑。
- Pet/activity/companion/Tauri event 模块未修改。

## 验证与回归防线

- `provider_read_only_and_mcp_isolation_fail_closed_on_every_resume`：首轮/read-only/resume/auth failure 参数与 init 防线。
- `provider_real_claude_blocks_untrusted_mcp_hooks_and_keeps_read_only_restricted`：真实未登录 CLI 下恶意 MCP/Hook/permissions marker 均不执行。
- `provider_reaps_result_interrupt_stdout_and_oversize_process_trees`：result-then-sleep、ignore-SIGINT、stdout-close、超长无换行，以及 stop/destroy/shutdown 的 root/child PID 消失。
- `provider_chunks_two_mib_result_before_the_one_mib_provider_frame_limit`：2 MiB result 完整分块并抵达 terminal。
- `provider_event_failure_still_publishes_a_small_failed_terminal`：output send failure 不留下 pending。
- 生产依赖 metadata 继续断言只有生成 Provider/Core SDK 与纯 Rust serde/tokio/uuid/libc；Tauri 隔离测试继续断言 Provider event 不进入 Desktop/Pet adapter。

验证命令：

```sh
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-claude --all-targets
CODEPET_CLAUDE_EXECUTABLE=/absolute/claude cargo test --manifest-path crates/Cargo.toml -p codepet-provider-claude --test provider_vertical provider_real_claude_blocks_untrusted_mcp_hooks_and_keeps_read_only_restricted -- --ignored --exact
cargo test --manifest-path src-tauri/Cargo.toml --test runtime_gateway_core_tests real_provider_events_only_emit_remote_tauri_channel_and_never_call_desktop_adapter
cargo tree --manifest-path crates/Cargo.toml -p codepet-provider-claude --edges normal
git diff --check
```

## 未知项

- Claude managed policy hook 仍可能运行；Provider 不声称关闭它，也不发布 `hooksDisabled`。组织若要求 managed hook 也不执行，必须在管理员 policy 层处理。
- Windows 不广告 turn interrupt；stop/destroy/shutdown 使用系统 tree termination，但本轮没有 Windows 实机。
- Provider process 重启后不保存 conversation registry，也不扫描 transcript。
- 三个 Provider 的正式打包、Catalog 默认发现、macOS universal 与 Windows 产物由后续统一集成任务负责。
