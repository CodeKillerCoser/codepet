# Claude Provider 插件运行时

## 当前结论

Claude 默认 Provider 是独立 Rust 二进制 `crates/providers/codepet-provider-claude`。它实现生成的 `Provider` trait，入口只构造 `ClaudeProvider` 并调用公共 `codepet-provider-sdk::serve_stdio`；JSON-RPC reader/writer、普通/控制双通路、typed event、过载、terminal cleanup 与 shutdown drain 不在 Claude crate 重复实现。Host resolver 注入的绝对 Claude executable 是唯一启动路径。Provider crate 不依赖 Host、Gateway、Tauri、Pet SDK、Desktop IPC、Hook collector 或 activity store。

当前官方公开接口中未发现与 Codex App Server 等价、可由 Rust 直接消费的完整 Claude session server。诚实切面是官方 Claude Code CLI 的 `--print` + 双向 `stream-json`：Provider 管理自己的 session ID，一次 turn 启动一个 CLI 子进程，后续 turn 用 `--resume`。它不宣称能枚举、读取、附着或同步其他 Claude Desktop/CLI 会话。

## 配置继承语义

产品要求 CodePet 中的 Claude 与用户在同一 workspace 直接启动 Claude 时使用相同配置。Provider 因此不提供 settings、MCP、Hook、plugin、memory 或 permission sandbox：

- 不传 `--safe-mode`、空 `--setting-sources` 或 inline `--settings`。
- 不传 `--strict-mcp-config`、空 `--mcp-config`、`--restricted` 或 `--tools`。
- 只传 CodePet 当前选择的 `--permission-mode` 和 `--permission-prompt-tool stdio`，不改写 auto-memory 环境变量。
- 继承 Host 进程环境、HOME/`CLAUDE_CONFIG_DIR`、当前 workspace 与 Claude 默认 user/project/local/managed 配置发现。

这意味着本机 MCP、Hook 和 plugin 可以在 Provider-launched CLI 中照常加载或执行。CodePet 不把这描述为安全缺陷，也不承诺配置隔离；用户与组织仍是 Claude 配置的权威。

上一轮 review 曾要求隔离上述配置并强化 read-only，随后产品语义明确修正为“本机设置是什么样就是什么样”。当前实现以该产品决定为准，并关闭无法在继承配置下保证的强 access mode。

## 官方接口审计

- [CLI reference](https://code.claude.com/docs/en/cli-reference) 定义 `--print`、stream-json、`--session-id`、`--resume`、model/effort 与默认配置相关 flags。
- [Settings reference](https://code.claude.com/docs/en/settings) 说明 user/project/local/managed settings 的层级与合并行为。
- [MCP reference](https://code.claude.com/docs/en/mcp) 说明 MCP 的 user、project、local、plugin 与 managed 来源。
- [Hooks reference](https://code.claude.com/docs/en/hooks) 定义本机 Hook 配置和执行行为。
- [Programmatic/headless guide](https://code.claude.com/docs/en/headless) 定义 NDJSON、`system/init`、text delta、最终 result、session resume 与 SIGINT。
- [Agent SDK overview](https://code.claude.com/docs/en/agent-sdk/overview) 列出的 SDK 是 Python/TypeScript；本实现不引入 SDK sidecar。

## 目标与非目标

目标：

- 独立 binary、显式 manifest、生成 Provider SDK、四段 route 和 Host instance lifecycle。
- Provider-managed create/顺序 turn、主 agent 文本增量、终态和 Unix interrupt。
- Claude `can_use_tool` control request、Provider approval event 与同进程 stdin 决策回传。
- Claude CLI 按其默认路径继承本机与项目配置。
- result 只有在进程真实退出和 stdout 有界排空后才形成权威 terminal。
- 正常 EOF、坏 Host frame 或 Provider stdout 写失败都必须通过无输出依赖的最终 reap 回收活动进程树。
- Provider Protocol event 只进入 Host/Gateway remote replay/event。

非目标：

- 不提供 Claude 配置、MCP、Hook、plugin 或 permission sandbox。
- 不实现 Python/TypeScript SDK sidecar、MCP permission server、transcript scan、窗口控制、Claude Desktop IPC 或 Agent View。
- 不伪造全局 Desktop 会话同步、进程重启恢复或 turn steer。
- 不把 Claude Code runtime 本身放进 App，也不绕过用户本机 runtime 配置；发行包只内置 Code Pet 自有 Provider adapter 与 manifest。

## 数据链与配置权威

```text
Claude Code CLI 官方 stream-json
  <-> codepet-provider-claude
  <-> Provider Protocol v1 / stdio JSON-RPC
  <-> PluginProcess / PluginManager
  <-> ProviderGatewayService remote replay/event
```

Provider mapper 不把 Provider event 写入 Pet Protocol/activity store、Desktop IPC/companion 或 Tauri Pet event。Claude 继承的本机 Hook 可以独立调用既有 Hook collector；该事件属于 Claude Hook 数据源，不是 Provider event 的跨层投影。

| 配置 | 权威 | 行为 |
| --- | --- | --- |
| Provider binary/manifest | `codepet-provider.json` | Catalog 解析相对 binary。 |
| Claude executable | `AgentRuntimeService` | Host 注入绝对 `claudeExecutable`；Provider 不探测。 |
| workspace/model/effort | `conversation.create` | 验证后转为 CLI cwd/flag。 |
| native session ID | Provider process | 首 turn `--session-id`，后续 `--resume`。 |
| settings/MCP/Hook/plugin/memory | 本机 Claude | Provider 不传隔离或覆盖参数，使用默认发现。 |
| permissions | CodePet selection + 本机 Claude policy | Provider 传所选 permission mode；`can_use_tool` 经 stdio 映射为 Remote approval。 |

## Provider v1 能力矩阵

| 方法 | 状态 | 真实映射 |
| --- | --- | --- |
| initialize/describe | 支持 | Provider v1、`dev.codepet.claude`、`claude` kind。 |
| instance lifecycle/capabilities | 支持 | 严格 Host executable setting、`--version`、有界停止全部进程组。 |
| conversation.create | Provider-managed | 预留 UUID 与选项；首消息前不伪造上游 session。 |
| conversation.list/get | Provider-managed | 返回本进程管理及受支持的持久会话视图，不宣称全局 Claude Desktop 同步。 |
| turn.start | 支持 | raw user NDJSON、首次 session ID、后续 resume。 |
| turn.interrupt | Unix 支持 | SIGINT 750 ms grace，超时 SIGKILL process group；真实退出后才 terminal。 |
| turn.steer | 不支持 | queued input 不等价于 active-turn steering。 |
| approval.resolve | 支持 | `can_use_tool` 映射 pending approval；approve/deny 通过同一子进程 stdin 的 `control_response` 回写。 |
| provider.shutdown | 支持 | 回收所有 instance；事件发送失败也继续进程清理。 |

Access mode：

| Provider access | 状态 | 语义 |
| --- | --- | --- |
| workspace-write | 支持 | conversation create 的兼容入口；turn 默认选择 `manual` permission mode，不是额外 sandbox。 |
| read-only | 不支持 | 任意本机 allow/managed 配置下无法保证 CodePet 强只读语义。 |
| full-access | 不支持 | 不通过 bypass flag 覆盖本机权限策略。 |

## 进程与输出模型

- 每个 turn 建立独立 process group。后台 reaper 独占 `Child` 并负责 `wait`；control 保存 PID/process-group ID、退出通知和授权回传所需的互斥 stdin writer。
- `can_use_tool` 到达后 turn/conversation 进入 `waiting-approval`；决策成功回写后恢复 `running`。进程退出时未决审批发布 `expired`。
- result/aborted 只记录 pending completion；进程真实退出、stdout 排空后才发布 terminal 并删除 active。此间下一 turn 返回 `turn_already_active`。
- interrupt、instance.stop、destroy 和 protocol shutdown 先给有界 grace，再杀整个 process group并等待 reaper。
- 公共 SDK 把 transport terminal 与 Provider cleanup 分开。stdin EOF 或坏/超大 Host frame 出现时先将 typed event output 标记为不可用，并把此后不可观测的 cleanup event 当作已丢弃，再调用 Claude 的 `provider_shutdown` 回收活动进程；fatal frame 的标准 JSON-RPC error 只在 cleanup 后按 2 秒 drain deadline 尝试写出。即使 stdout 不可写或持续背压，清理与 runtime 返回都保持有界。
- Claude stdout 单物理行硬限制为 4 MiB；超过限制且无换行时立即杀进程组。stderr 单行限制 64 KiB。
- Provider Frame V1 的最终编码 frame 上限是 16 MiB。所有对外 text delta/result 仍按 UTF-8 边界切为最多 64 KiB；终态 metadata 限制为 4 KiB。
- output event 失败时 active 不会先删除；正常 lifecycle 仍尝试尺寸安全的 failed terminal。致命 stdio cleanup 不等待 terminal event 成功，只等待进程退出。

## 涉及模块

- `src/client.rs`：默认 Claude 参数、双向 stdin control response、process group、reaper、退出通知与上游 line limit。
- `src/main.rs`：只构造 `ClaudeProvider` 并进入公共 `serve_stdio` runtime。
- `src/provider.rs`：能力、配置继承元数据、延迟终态、分块和 lifecycle 回收。
- `tests/fixtures/claude_stream.rs`、`tests/provider_vertical.rs`：默认参数、阻塞式授权闭环、无害项目 MCP 可见性、异常 stdio PID、2 MiB 与进程探针。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs`：只负责 resolver setting 注入，本轮无新增逻辑。
- Pet/activity/companion/Tauri event 模块未修改。

## 验证与回归防线

- `provider_inherits_claude_project_configuration_and_rejects_strong_access_modes`：命令行无隔离/permission flags，无害 `.mcp.json` 沿 workspace 默认路径可见，read-only/full-access fail closed。
- `provider_binary_reaps_active_tree_after_response_pipe_breaks`：active ignore-SIGINT turn 下关闭 Provider stdout，Provider 非零退出且 root/child PID 消失。
- `provider_binary_reaps_active_tree_after_invalid_json_under_stdout_backpressure`、`provider_binary_returns_a_standard_error_then_fail_stops_after_an_oversized_host_frame`：分别证明不读取 stdout 时 fatal cleanup 仍有界回收进程树，以及 stdout 可写时超大 Host frame 先返回标准 JSON-RPC error、后续请求不会执行。
- `provider_reaps_result_interrupt_stdout_and_oversize_process_trees`：result-then-sleep、ignore-SIGINT、stdout-close、超长无换行及 stop/destroy/shutdown 回收。
- `provider_chunks_two_mib_result_before_the_one_mib_provider_frame_limit`：2 MiB result 完整分块并抵达 terminal。
- `provider_event_failure_still_publishes_a_small_failed_terminal`：正常 output send failure 不留下 pending。
- `provider_maps_claude_stream_json_and_fails_closed_for_missing_methods`：fixture 发出 `can_use_tool` 并等待 stdin，验证 approval requested、approve control response、approval resolved 和 terminal 闭环。
- 生产依赖断言只有生成 Provider/Core SDK 与纯 Rust基础库；Tauri 隔离测试断言 Provider event 不进入 Desktop/Pet adapter。
- `codepet-host/tests/builtin_provider_integration.rs` 从正式 manifest 与 resolver 等价的绝对 fixture setting 启动 Claude，并与 Codex、OpenCode 一起验证 Ready、Gateway 路由和有序 shutdown。

## 未知项

- 本机 Hook/MCP/plugin 的行为与风险由用户或组织 Claude 配置决定；CodePet 不做审计或隔离。
- Windows 不广告 turn interrupt；stop/destroy/shutdown 使用系统 tree termination，本轮没有 Windows 实机。
- Provider process 重启后不保存 conversation registry，也不扫描 transcript。
- 正式打包与 Catalog 默认发现由 `provider-host-device-and-plugin-runtime.md` 中的统一 staging/resources 流程负责；Claude runtime 仍来自用户本机 resolver。
