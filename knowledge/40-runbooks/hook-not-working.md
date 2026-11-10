# Hook 不生效

## 当前入口

v0 的 Codex、Claude、OpenCode 活动走 Provider observation → Host subscription → PetGateway。旧 collector 的 `/hook`、47621 端口和 `code-pet-hook.mjs` 不能证明这条链路正常。源码入口是 `crates/providers/codepet-observation`、各 Provider 的 observation Definition、`crates/codepet-host/src/pet_gateway`。

## 2026-09-07 Windows Codex 故障证据

- 用户在 Codex Windows 桌面应用信任 hooks 后，旧任务和新建任务仍无桌宠事件。
- 原生 Codex `app-server` 的 `hooks/list` 返回 10 个 CodePet handlers 均 enabled / trusted，warnings / errors 为空。
- `.codex/codepet-observation/endpoint.json` 的监听端口可达。原命令经 CMD 向隔离 HTTP 接收器投递成功，但经 PowerShell 执行出现 ParserError，转发脚本未执行。
- Codex `rust-v0.153.4` 的 `core/src/session/mod.rs::build_hooks_config` 从当前环境 shell 派生命令；不能只根据 hooks command runner 的默认 CMD 回退推断桌面应用使用 CMD。
- 写入修复后的 Windows 命令，用户重新信任并发送新任务后，明确确认桌宠收到事件。

## 根因及修复

原命令把 Node 绝对路径和参数用双引号包裹，适用于 CMD；PowerShell 对带引号可执行路径需要调用运算符 `&`。Codex Definition 显式启用 Windows command override，共享安装器生成 `commandWindows`：通过 PowerShell EncodedCommand 调用 Node，参数先按 PowerShell 单引号规则转义，再用 UTF-16LE / Base64 编码。外层调用可由 CMD 或 PowerShell 执行，避免双层引号破坏空格、中文或单引号路径。保留 command 字段和托管项匹配方式，重复安装不改变配置，不删除用户 handlers。

Windows Codex 非 Interrupt hook 给 shell 启动留 10 秒上限；Interrupt 遵守 Codex 的 3 秒限制。Node 转发仍有原有 1 秒网络 / 1.5 秒进程预算，不输出权限决定。Claude 不启用 Codex 的 commandWindows 字段；macOS/Linux 保持现有命令。

引入历史未确认。更改命令或 timeout 会改变 Codex 信任 hash，必须由用户重新信任；不得代写 trusted_hash。

## 排查顺序

1. 查 CodePet 日志和来源状态，区分 Provider 启动失败、订阅失败和等待首次活动。
2. 按各 Provider Definition 找实际配置目录，核对专用环境变量是否一致。检查托管 hook、转发脚本和 endpoint 文件；不要输出 endpoint token。
3. 读取原应用实际 hook 列表，核对 enabled、trusted、warnings 和 errors。安装成功不能替代执行证据。
4. 使用实际 shell 及安装后的完整命令向临时接收器投递，测试 stdin、HTTP 和退出行为。直接 `node script` 或只用 CMD 不覆盖 Codex Windows 桌面入口。
5. 在真实原应用提交任务验证。必要时临时记录执行阶段和 HTTP 状态，禁止记录提示词、工具输入或 token，结束后恢复脚本。
6. 若 HTTP 已成功，继续检查 Host 上游订阅 ID、generation、PetGateway 来源状态和任务投影，不重复要求用户信任。

## 2026-11-10 macOS：运行中显示“尚未收到回复内容”

现象：用户确认任务仍在执行，hooks 正常工作，但顶部桌宠没有显示过程回复。

证据：本机 `logs/events.jsonl` 的 Codex 事件含 `PreToolUse/PostToolUse`；抽查最新事件只有工具输入、输出、会话与 turn 等字段，没有 `last_assistant_message`。检查时当前日志中两类事件分别出现 116、104 次（日志会继续增长，计数不代表唯一工具调用）。[官方 Hooks 定义](https://learn.chatgpt.com/docs/hooks) 的工具事件不提供此回复字段，`Stop` 才提供可空的 `last_assistant_message`。

修复前链路：`codepet-observation/src/forward.mjs` 原样转发；Host `pet_gateway/projection.rs` 在 UserPromptSubmit 清空 summary，随后只从 `last_assistant_message` 更新正文；`pet_gateway/mod.rs` 只消费活动订阅，没有正文补拉；`NotchPet.svelte` 对空 summary 显示占位提示。Provider 的空 item 内容更新提示虽供 Remote 补拉，但桌宠没有消费这条正文链路。

根因：桌宠将活动摘要收窄为助手回复，移除了执行中可用的内容回退，导致正常工具事件无法更新摘要。该现象不能用于判断 hooks 未生效。

用户补充“之前正常”后追查 Git：`ea9ee1e` 明确将 summary 从 `last_assistant_message → prompt → error → tool_input.description/command/file_path` 的回退链改成只读取 `last_assistant_message`，并在 UserPromptSubmit 忽略回复字段。`421d05c` 的顶部视图使用“尚未收到回复内容”作为空正文提示。由此证实本项目改变过运行中展示行为；目前没有证据证明 Codex 官方删改了工具 hook 的回复字段，也尚未确定用户此前使用的具体安装版本。旧版有文字不等于旧版拿到了中途助手正文。

修复：按用户要求恢复活动摘要回退，依次读取助手回复、prompt、error、OpenCode error.data.message、工具 description/command/file_path。新 UserPromptSubmit 忽略可能携带的上轮助手回复，仍先清空旧摘要；空事件保留本轮已有摘要。仅更改共享 Host 投影，前端直接消费恢复后的 summary，无需修改 hooks 或新增正文补拉。

验证：`cargo test --manifest-path crates/Cargo.toml -p codepet-host --lib pet_gateway::projection::tests -- --nocapture`，7 项通过；`git diff --check` 通过。覆盖工具描述/命令/路径回退、空白字段、回复优先、新轮清空、迟到旧回复，以及 OpenCode 错误摘要和终态保留。回归约束见 [桌宠活动摘要](../60-rules/pet-activity-summary.md)。

未知项：尚未构建替换已安装应用或进行修复后实机验收；用户此前使用的安装版本未确认。若任务结束后仍为空，另查 Stop 是否到达、字段是否为空，以及时间和 turn 过滤。

## 回归防线与验证

`cargo test --manifest-path crates/Cargo.toml -p codepet-observation --lib` 覆盖重复安装、用户 handlers 保留、多订阅、多个 Hook 进程、CMD / PowerShell 命令执行、噪声过滤和离线退出。2026-09-07 三项通过；本机 CMD / PowerShell 独立投递均返回 204；用户确认真实桌面任务恢复。

跨平台风险通过非 Windows 条件编译和原命令保持控制；macOS/Linux 本轮未实机运行。Codex 使用其他自定义 shell 的情况仍需对应环境验证。


## 2026-09-07 Claude 独立实测与注入规范

Claude 官方 [Hooks reference](https://code.claude.com/docs/en/hooks) 支持 command + args 的 exec form：直接启动可执行文件，参数为数组；Windows 应使用真实 exe，不把 cmd/bat 当直接可执行文件。无 args 时才进入 shell 解析，不能沿用 Codex 的 PowerShell 命令文本。

共享 observation 对 Claude 生成 node 可执行文件加 [forward.mjs, endpoint.json] 参数；Codex Windows 保留经双 shell 验证的 EncodedCommand 形式。更新/清理时同时检查 command 和 args 中的 CodePet 标记，保留其他用户 handler。新增 Claude exec bridge 测试直接执行生成的 argv，验证多会话、过滤与离线 fail-open。

独立脚本 scripts/test_claude_observation.py 创建临时 Python HTTP 接收器和临时 settings.json，通过真实 claude -p --settings 执行固定回复。Claude 2.1.209 已收到 SessionStart、UserPromptSubmit、Stop，Stop 的 last_assistant_message 为 OK，退出码 0。使用 SDK process_runner 包裹 CLI 后临时工作目录可正常清理；此前不托管子树的试验虽收到事件，但目录仍被后代占用，不能视为完整通过。

```powershell
cargo build --manifest-path sdk/rust/Cargo.toml -p codepet-provider-sdk --example process_runner
python scripts/test_claude_observation.py --runner sdk/rust/target/debug/examples/process_runner.exe --executable <Claude 原生 exe 的绝对路径>
```

测试不修改正式 Claude 配置。当前 Codex/Claude/OpenCode 的旧 CodePet hook 已清理，桌宠三个 source 偏好保持关闭，清理前数据存于本机临时目录 codepet-injection-backup-5i0_i_eu。不要把该备份恢复当作测试步骤。Codex 桌面实际事件由用户在修正注入并重新信任后确认收到；这与 Claude 独立测试是两份不同证据。
