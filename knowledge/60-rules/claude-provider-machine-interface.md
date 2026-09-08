# Claude Provider 使用官方机器接口、继承本机配置并保持协议链分层

## 规则

Claude Provider 的上游只能是 Host resolver 注入的 Claude executable 与官方非交互机器接口。当前允许的最小接口是 `claude --version`、`claude auth status --json`、`claude --print` 的双向 `stream-json`、`--permission-prompt-tool stdio` 的 `control_request`/`control_response`、Provider 自建 UUID 的 `--session-id`/`--resume` 和 Unix 信号。握手版本与认证状态来自前两个命令；用量统一通过 `usage.query` 提供，runtime 不再携带独立摘要。`usage.query` 允许由结束 Hook 注册来源并增量只读对应 transcript 的计量字段，按消息 ID 去重；不得全目录扫描或用于恢复会话控制，详见 [Provider 用量查询与业务存储](../10-architecture/provider-usage-query-and-storage.md)。不得通过窗口控制、Claude Desktop/IDE 私有状态或猜测字段补能力。

Claude Provider 的 `runtime.getInstalled` 负责搜索当前 PATH 与登录 Shell，并以 `--version` 验证结果；不得扫描 Claude Desktop、IDE 私有目录或 transcript。选择结果缺失、非绝对、不可执行或版本验证失败时，instance lifecycle 必须 fail closed。

没有稳定机器接口的 Provider Protocol 方法必须从 capabilities 中关闭，并返回 `capability_unsupported`。当前包括 `turn.steer`；Provider process 重启后也不扫描 transcript 恢复 session。Claude 的 `can_use_tool` stdio control request 必须映射为 pending Provider approval，App 的 approve/deny 必须回写同一 CLI 进程的 stdin；不得只展示一个无法回传的伪审批。进程退出时仍 pending 的审批必须发布 expired resolution。

Provider Protocol event 只能进入 `ProviderGatewayService` remote replay/event，不得由 Provider mapper 写入 Pet Protocol、`SharedState` activity、companion replay、Desktop IPC、`codex-desktop-companion-event` 或 `pet-event`。Claude 继承的本机 Hook 仍可按用户配置独立调用既有 collector；这是 Claude 配置行为，不是 Provider event 路由。

每个 turn 都必须使用 Claude 的默认配置发现：不得传 `--safe-mode`、空 `--setting-sources`、inline `--settings`、strict/empty MCP、`--restricted` 或工具白名单，也不得改写 auto-memory 环境变量。user/project/local/managed settings、MCP、Hook、plugin 与 memory 以本机 Claude 实际配置为准；CodePet 选择的 access mode 可映射为 Claude permission mode，默认 `manual`，并以 `stdio` 承接授权交互。CodePet 不提供额外 sandbox 隔离边界。

继承任意本机配置时无法保证强 `read-only`，因此 capabilities 不得广告 `read-only`，conversation.create 必须拒绝它。当前只接受 `workspace-write` 作为“继承 Claude 默认权限”的 Provider Protocol 兼容入口，不将它描述为 sandbox；`full-access` 同样不广告，避免通过 bypass flag 覆盖本机策略。

每个 turn 的后台 reaper 必须独占 `Child` 和 `wait`。运行时 control 保存 PID/process-group ID、退出通知和互斥保护的 stdin writer；result、aborted 或 interrupt 请求都不能在进程真实退出前删除 active。interrupt、stop、destroy 和 shutdown 必须有界等待，超时杀整个进程组，并在 reaper 确认退出后才发布权威 terminal。

Provider binary 的服务循环结果与最终进程清理必须分开。正常 EOF、坏或超大 Host frame、response write/flush 失败都必须先进入不发布状态或 terminal event 的有界 reap，再返回原始服务结果；不能把清理只放在循环的正常落点，也不能依赖 stdout 仍可写。codec fatal error 在 reap 前不得写错误响应；当前实现直接 fail-stop，不回写该响应。

Claude stdout 单物理行当前硬上限是 4 MiB，超过上限且未换行时应立即终止进程组。Provider→Host 使用 SDK Runtime 管理的 Frame V1，最终编码 frame 上限是 16 MiB；所有对外文本必须按 UTF-8 边界切成不超过 64 KiB 的事件。output 发送失败时不得先删除 active，必须在进程退出后尝试发送尺寸安全的 failed terminal。

## 适用场景

- 修改 `crates/providers/codepet-provider-claude` 的 CLI 参数、wire DTO、mapper、capability 或 lifecycle。
- 修改 Claude runtime resolver、Host instance setting 注入或 Provider refresh。
- 试图增加会话发现、恢复、steer、审批、桌面同步或 Pet 展示。

## 反例

- Provider 内调用 `which claude`、遍历扩展目录或在 Host path 失败后静默 fallback。
- 读取 `~/.claude/projects` transcript 来实现 list/get 或恢复进程内 registry。
- 把 Hook payload、窗口标题、Agent View UI 或未文档字段解释成 Provider session 状态。
- 收到 `can_use_tool` 后关闭 stdin，导致 App 能看到授权但决策无法回写 Claude。
- 为复用旧桌宠代码，把 remote Provider event 写进 activity store，再依赖前端过滤。
- 为追求隔离而传 safe/restricted/strict-empty MCP，使 CodePet 中的 Claude 与用户直接运行 Claude 表现不同。
- result frame 到达就移除 active，而 Claude 根进程或其子进程仍在运行。
- 把巨型 result 塞进单个 Provider event，并在 Provider Frame V1 超限后静默丢 terminal；Runtime 必须显式返回稳定错误，业务实现不得绕过 SDK transport。
- 在 Provider stdio loop 内用 `?` 直接返回 write/flush 错误，使循环末尾的 protocol shutdown 永远不执行。
- 收到非法或超大 Host frame 后先同步 flush 错误响应；Host 不读取 stdout 时会在进入外层 reap 前永久阻塞。

## 推荐做法

- 上游 Claude DTO 只覆盖官方文档或真实 fixture 验证过的字段，使用宽松未知 variant；route、session、framing 和非法 JSON 严格失败。
- 使用生成的 Provider SDK server trait、dispatcher、codec、四段请求 route 与共享 Agent DTO；不要手写第二份 Provider Protocol 或 Provider→Gateway 业务对象。
- 每次 Provider-launched turn 只添加机器接口、session、model/effort、所选 permission mode 和 stdio permission prompt 参数，不覆盖 Claude 的 settings/MCP/plugin 配置发现。
- fixture 必须让 Claude 子进程发出真实形状的 `can_use_tool` control request、阻塞读取第二条 stdin 消息，并分别验证 allow 的 `updatedInput` 与 deny 结果。
- 用无害 `.mcp.json` fixture 证明项目配置沿默认 cwd 可见，同时断言命令行不包含隔离参数；不要为测试执行危险 MCP 或 Hook 命令。
- 用 result-then-sleep、ignore-SIGINT、stdout-close、超长无换行与 stop/destroy/shutdown 探针确认 root/child PID 都消失。
- 在 active ignore-SIGINT turn 下分别关闭 Provider stdout，以及填满 stdout 后发送非法 JSON、超大 Host frame；确认 Provider 可非零退出但 root/child PID 必须有界消失。
- 新能力先给出官方接口证据、真实 wire fixture 和 fail-closed 负例，再加入 capability。

## 验证方式

- `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-claude --all-targets`
- `cargo tree --manifest-path crates/Cargo.toml -p codepet-provider-claude`，确认无 Host、Gateway、Pet、Tauri 或 Desktop 依赖。
- 运行 Tauri 的 `real_provider_events_only_emit_remote_tauri_channel_and_never_call_desktop_adapter` 隔离测试。
- 审查生产源码不引用 `claude_transcript`、Hook、activity、Pet、companion 或 Desktop IPC。

## 来源

- `../10-architecture/claude-provider-plugin-runtime.md`
- `protocol-layer-and-channel-boundaries.md`
- [Claude CLI reference](https://code.claude.com/docs/en/cli-reference)
- [Claude settings reference](https://code.claude.com/docs/en/settings)
- [Claude MCP reference](https://code.claude.com/docs/en/mcp)
- [Claude permission modes](https://code.claude.com/docs/en/permission-modes)
- [Claude environment variables](https://code.claude.com/docs/en/env-vars)
- [Claude programmatic/headless guide](https://code.claude.com/docs/en/headless)
- [Claude Hooks reference](https://code.claude.com/docs/en/hooks)
