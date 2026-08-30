# Claude Provider 插件运行时

## 当前结论

Claude 默认 Provider 是独立 Rust 二进制 `crates/providers/codepet-provider-claude`。它只通过生成的 `codepet-provider-sdk` 与 Host 交换 Provider Protocol v1 JSON-RPC / stdio JSON-lines，并把 Host resolver 注入的绝对 Claude executable 作为唯一启动路径。Provider crate 不依赖 Host、Gateway、Tauri、Pet SDK、Desktop IPC、Hook collector 或 activity store。

当前官方公开接口中未发现与 Codex App Server 等价、可由 Rust 直接消费的完整 Claude session server。诚实切面是官方 Claude Code CLI 的 `--print` + 双向 `stream-json`：Provider 管理自己的 session ID，一次 turn 启动一个 CLI 子进程，后续 turn 用 `--resume`。它不宣称能枚举、读取、附着或同步其他 Claude Desktop/CLI 会话。

## 配置继承语义

产品要求 CodePet 中的 Claude 与用户在同一 workspace 直接启动 Claude 时使用相同配置。Provider 因此不提供 settings、MCP、Hook、plugin、memory 或 permission sandbox：

- 不传 `--safe-mode`、空 `--setting-sources` 或 inline `--settings`。
- 不传 `--strict-mcp-config`、空 `--mcp-config`、`--restricted` 或 `--tools`。
- 不传 `--permission-mode`，也不改写 auto-memory 环境变量。
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
- Claude CLI 按其默认路径继承本机与项目配置。
- result 只有在进程真实退出和 stdout 有界排空后才形成权威 terminal。
- 正常 EOF、坏 Host frame 或 Provider stdout 写失败都必须通过无输出依赖的最终 reap 回收活动进程树。
- Provider Protocol event 只进入 Host/Gateway remote replay/event。

非目标：

- 不提供 Claude 配置、MCP、Hook、plugin 或 permission sandbox。
- 不实现 Python/TypeScript SDK sidecar、MCP permission server、transcript scan、窗口控制、Claude Desktop IPC 或 Agent View。
- 不伪造全局 list/get、Desktop 同步、进程重启恢复、turn steer 或 approval callback。
- 不在本分支实现二进制/manifest 打包发现；三个 Provider 的 macOS universal/Windows 集成由后续任务统一处理。

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
| permissions | 本机 Claude | Provider 不传 permission mode；managed 与用户配置照常生效。 |

## Provider v1 能力矩阵

| 方法 | 状态 | 真实映射 |
| --- | --- | --- |
| initialize/describe | 支持 | Provider v1、`dev.codepet.claude`、`claude` kind。 |
| instance lifecycle/capabilities | 支持 | 严格 Host executable setting、`--version`、有界停止全部进程组。 |
| conversation.create | Provider-managed | 预留 UUID 与选项；首消息前不伪造上游 session。 |
| conversation.list/get | 不支持 | 不扫描 transcript 或缓存冒充上游 CRUD。 |
| turn.start | 支持 | raw user NDJSON、首次 session ID、后续 resume。 |
| turn.interrupt | Unix 支持 | SIGINT 750 ms grace，超时 SIGKILL process group；真实退出后才 terminal。 |
| turn.steer | 不支持 | queued input 不等价于 active-turn steering。 |
| approval.resolve | 不支持 | 无 SDK callback/MCP permission tool。 |
| provider.shutdown | 支持 | 回收所有 instance；事件发送失败也继续进程清理。 |

Access mode：

| Provider access | 状态 | 语义 |
| --- | --- | --- |
| workspace-write | 支持 | 兼容入口；不传 native permission flag，实际权限继承 Claude 配置，不是 sandbox。 |
| read-only | 不支持 | 任意本机 allow/managed 配置下无法保证 CodePet 强只读语义。 |
| full-access | 不支持 | 不通过 bypass flag 覆盖本机权限策略。 |

## 进程与输出模型

- 每个 turn 建立独立 process group。后台 reaper 独占 `Child` 并负责 `wait`；control 只保存 PID/process-group ID 与退出通知。
- result/aborted 只记录 pending completion；进程真实退出、stdout 排空后才发布 terminal 并删除 active。此间下一 turn 返回 `turn_already_active`。
- interrupt、instance.stop、destroy 和 protocol shutdown 先给有界 grace，再杀整个 process group并等待 reaper。
- Provider binary 把服务循环结果与最终 cleanup 分开。无论正常 EOF、坏/超大 Host frame、response write 或 flush 失败，都会调用不发布 event、不依赖 stdout 的 `reap_active_processes`，然后返回原始服务结果。
- Claude stdout 单物理行硬限制为 4 MiB；超过限制且无换行时立即杀进程组。stderr 单行限制 64 KiB。
- Provider codec frame 上限是 1 MiB。所有对外 text delta/result 按 UTF-8 边界切为最多 64 KiB；终态 metadata 限制为 4 KiB。
- output event 失败时 active 不会先删除；正常 lifecycle 仍尝试尺寸安全的 failed terminal。致命 stdio cleanup 不等待 terminal event 成功，只等待进程退出。

## 涉及模块

- `src/client.rs`：默认 Claude 参数、process group、reaper、退出通知与上游 line limit。
- `src/main.rs`：stdio 服务结果与无输出依赖的最终 reap。
- `src/provider.rs`：能力、配置继承元数据、延迟终态、分块和 lifecycle 回收。
- `tests/fixtures/claude_stream.rs`、`tests/provider_vertical.rs`：默认参数、无害项目 MCP 可见性、异常 stdio PID、2 MiB 与进程探针。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs`：只负责 resolver setting 注入，本轮无新增逻辑。
- Pet/activity/companion/Tauri event 模块未修改。

## 验证与回归防线

- `provider_inherits_claude_project_configuration_and_rejects_strong_access_modes`：命令行无隔离/permission flags，无害 `.mcp.json` 沿 workspace 默认路径可见，read-only/full-access fail closed。
- `provider_binary_reaps_active_tree_after_response_pipe_breaks`：active ignore-SIGINT turn 下关闭 Provider stdout，Provider 非零退出且 root/child PID 消失。
- `provider_binary_reaps_active_tree_after_an_oversized_host_frame`：active turn 下发送超大 Host frame，错误响应后 root/child PID 消失。
- `provider_reaps_result_interrupt_stdout_and_oversize_process_trees`：result-then-sleep、ignore-SIGINT、stdout-close、超长无换行及 stop/destroy/shutdown 回收。
- `provider_chunks_two_mib_result_before_the_one_mib_provider_frame_limit`：2 MiB result 完整分块并抵达 terminal。
- `provider_event_failure_still_publishes_a_small_failed_terminal`：正常 output send failure 不留下 pending。
- 生产依赖断言只有生成 Provider/Core SDK 与纯 Rust基础库；Tauri 隔离测试断言 Provider event 不进入 Desktop/Pet adapter。

## 未知项

- 本机 Hook/MCP/plugin 的行为与风险由用户或组织 Claude 配置决定；CodePet 不做审计或隔离。
- Windows 不广告 turn interrupt；stop/destroy/shutdown 使用系统 tree termination，本轮没有 Windows 实机。
- Provider process 重启后不保存 conversation registry，也不扫描 transcript。
- 三个 Provider 的正式打包、Catalog 默认发现、macOS universal 与 Windows 产物由后续统一集成任务负责。
