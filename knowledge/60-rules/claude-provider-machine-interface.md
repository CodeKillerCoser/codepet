# Claude Provider 只能使用官方机器接口并与 Pet 链隔离

## 规则

Claude Provider 的上游只能是 Host resolver 注入的 Claude executable 与官方非交互机器接口。当前允许的最小接口是 `claude --print` 的 `stream-json` 输入/输出、Provider 自建 UUID 的 `--session-id`/`--resume`、CLI permission mode 和 Unix 信号。不得通过 Hook、transcript 扫描、窗口控制、Claude Desktop/IDE 私有状态或猜测字段补能力。

Provider 不得自己搜索 PATH、应用目录、VS Code 扩展或用户目录。`claudeExecutable` 缺失、非绝对、不可执行或 `--version` 失败时，instance lifecycle 必须 fail closed。

没有稳定机器接口的 Provider Protocol 方法必须从 capabilities 中关闭，并返回 `capability_unsupported`。当前包括 `conversation.list`、`conversation.get`、`turn.steer` 和 `approval.resolve`；Provider process 重启后也不扫描 transcript 恢复 session。审批若只能通过 Agent SDK callback 或 MCP permission tool 成立，不能在禁止 SDK sidecar/MCP 的实现中伪造。

Provider event 只能进入 `ProviderGatewayService` remote replay/event，不得进入 Pet Protocol、`SharedState` activity、Claude Hook collector、companion replay、Desktop IPC、`codex-desktop-companion-event` 或 `pet-event`。

每个 turn 都必须固定使用 `--safe-mode --setting-sources "" --strict-mcp-config --mcp-config '{"mcpServers":{}}'` 和 `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`。`system/init.mcp_servers` 非空必须立即 fail closed；这些参数不能因首轮、resume、权限等级或认证失败而变化。Provider 不得声称 managed policy Hook 已关闭，也不得发布笼统的 `hooksDisabled` 元数据。

`read-only` 必须同时使用 `--restricted`、`dontAsk` 和精确的 `--tools Read,Glob,Grep`。`system/init.tools` 出现其他工具必须 fail closed。不能只靠 `dontAsk`，因为外部 settings 的 `permissions.allow` 会预批准动作；如果未来 CLI 不能提供并回报这一工具边界，应关闭 read-only capability。

每个 turn 的后台 reaper 必须独占 `Child` 和 `wait`。运行时 control 只能保存 PID/process-group ID 与退出通知；result、aborted 或 interrupt 请求都不能在进程真实退出前删除 active。interrupt、stop、destroy 和 shutdown 必须有界等待，超时杀整个进程组，并在 reaper 确认退出后才发布权威 terminal。

Claude stdout 单物理行当前硬上限是 4 MiB，超过上限且未换行时应立即终止进程组。Provider codec frame 上限是 1 MiB；所有对外文本必须按 UTF-8 边界切成不超过 64 KiB 的事件。output 发送失败时不得先删除 active，必须在进程退出后尝试发送尺寸安全的 failed terminal。

## 适用场景

- 修改 `crates/providers/codepet-provider-claude` 的 CLI 参数、wire DTO、mapper、capability 或 lifecycle。
- 修改 Claude runtime resolver、Host instance setting 注入或 Provider refresh。
- 试图增加会话发现、恢复、steer、审批、桌面同步或 Pet 展示。

## 反例

- Provider 内调用 `which claude`、遍历扩展目录或在 Host path 失败后静默 fallback。
- 读取 `~/.claude/projects` transcript 来实现 list/get 或恢复进程内 registry。
- 把 Hook payload、窗口标题、Agent View UI 或未文档字段解释成 Provider session 状态。
- 因为 CLI 返回了 permission denial，就发布一个并不存在回写通道的可批准 Approval。
- 为复用旧桌宠代码，把 remote Provider event 写进 activity store，再依赖前端过滤。
- 只传 `--settings '{"disableAllHooks":true}'`，却继续加载 `.mcp.json`、plugin MCP 或外部权限 allow。
- result frame 到达就移除 active，而 Claude 根进程或其子进程仍在运行。
- 把 2 MiB result 塞进单个 Provider event，超过生成 codec 的 1 MiB frame 上限后静默丢 terminal。

## 推荐做法

- 上游 Claude DTO 只覆盖官方文档或真实 fixture 验证过的字段，使用宽松未知 variant；route、session、framing 和非法 JSON 严格失败。
- 使用生成的 Provider SDK server trait、dispatcher、codec、DTO 与四段 route；不要手写第二份 Provider Protocol。
- 每次 Provider-launched turn 使用 safe mode、空 filesystem setting sources 和 strict-empty MCP；明确记录管理员托管 Hook 无法由普通配置覆盖。
- 用真实无登录 CLI 在恶意 `.mcp.json`、user/project/local Hook 与 `permissions.allow` 下同时验证首轮和 resume；认证失败也必须先经过空 MCP、只读工具 init 防线。
- 用 result-then-sleep、ignore-SIGINT、stdout-close、超长无换行与 stop/destroy/shutdown 探针确认 root/child PID 都消失。
- 新能力先给出官方接口证据、真实 wire fixture 和 fail-closed 负例，再加入 capability。

## 验证方式

- `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-claude --all-targets`
- `CODEPET_CLAUDE_EXECUTABLE=/absolute/claude cargo test --manifest-path crates/Cargo.toml -p codepet-provider-claude --test provider_vertical provider_real_claude_blocks_untrusted_mcp_hooks_and_keeps_read_only_restricted -- --ignored --exact`
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
