# Claude Provider 只能使用官方机器接口并与 Pet 链隔离

## 规则

Claude Provider 的上游只能是 Host resolver 注入的 Claude executable 与官方非交互机器接口。当前允许的最小接口是 `claude --print` 的 `stream-json` 输入/输出、Provider 自建 UUID 的 `--session-id`/`--resume`、CLI permission mode 和 Unix SIGINT。不得通过 Hook、transcript 扫描、窗口控制、Claude Desktop/IDE 私有状态或猜测字段补能力。

Provider 不得自己搜索 PATH、应用目录、VS Code 扩展或用户目录。`claudeExecutable` 缺失、非绝对、不可执行或 `--version` 失败时，instance lifecycle 必须 fail closed。

没有稳定机器接口的 Provider Protocol 方法必须从 capabilities 中关闭，并返回 `capability_unsupported`。当前包括 `conversation.list`、`conversation.get`、`turn.steer` 和 `approval.resolve`；Provider process 重启后也不扫描 transcript 恢复 session。审批若只能通过 Agent SDK callback 或 MCP permission tool 成立，不能在禁止 SDK sidecar/MCP 的实现中伪造。

Provider event 只能进入 `ProviderGatewayService` remote replay/event，不得进入 Pet Protocol、`SharedState` activity、Claude Hook collector、companion replay、Desktop IPC、`codex-desktop-companion-event` 或 `pet-event`。

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

## 推荐做法

- 上游 Claude DTO 只覆盖官方文档或真实 fixture 验证过的字段，使用宽松未知 variant；route、session、framing 和非法 JSON 严格失败。
- 使用生成的 Provider SDK server trait、dispatcher、codec、DTO 与四段 route；不要手写第二份 Provider Protocol。
- 每次 Provider-launched turn 用官方 inline setting 禁用 Code Pet 用户/项目 Hook；明确记录管理员托管 Hook 无法由非托管设置覆盖。
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
- [Claude programmatic/headless guide](https://code.claude.com/docs/en/headless)
- [Claude Hooks reference](https://code.claude.com/docs/en/hooks)
