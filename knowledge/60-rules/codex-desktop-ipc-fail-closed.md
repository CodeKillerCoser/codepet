# Codex Desktop IPC 必须 Fail Closed

## 规则

Codex Desktop socket、握手、路由、协议版本、owner、revision 或 thread source 无法确认正确时，companion 必须明确 unavailable/error，并停止发布不可信状态或写请求。不得把独立 remote App Server、Hook、transcript/audit 或文件监听接成 companion fallback。App Server 可以在隔离 remote runtime 中独立工作。

Desktop 私有 DTO、方法名、路由字段和原始 JSON 只能存在于 Codex Desktop adapter。写 capability 使用肯定列表：只有已完成端到端路由、并发前置条件、Owner 确认和失败测试的动作才能声明；Owner ack 不等于权威任务状态已经变化。

## 适用场景

- 修改 Codex Desktop IPC transport、Owner/Follower adapter、状态 mapper 或重连逻辑。
- 修改 Runtime Gateway Provider 状态、capability 或 ProviderExtension。
- 修改桌宠任务状态、审批按钮、回复入口或 Provider unavailable 提示。
- 升级 Codex Desktop，或调整支持版本范围。

## 反例

- socket 连接失败后静默执行 `codex app-server --listen stdio://`，让 UI 看起来仍在线。
- patch revision 跳跃时继续合并，生成一个从未存在过的任务状态。
- 为了发现晚加入任务而扫描 transcript，并把推断结果标成 Desktop 实时状态。
- 把私有 follower payload 放进 Standard Protocol extension，让前端按原生字段分支。
- 私有协议存在审批请求，就直接广告 `approval.resolve`，但没有验证目标路由和 Desktop 是否确认处理。
- 写请求超时或断线后在新连接自动重放，导致一条回复、停止或审批决定作用两次。
- 收到 `{ok: true}` 就手工删除 pending approval 或把 turn 标成 interrupted，不等待权威 snapshot/patch。
- Provider event 被送入 companion snapshot/event、Desktop action 或 Pet projection；两条链路即使 native thread id 相同也不得互相改写状态。
- 把 permissions、MCP elicitation、user input 或 plan implementation 强行压成 v0 `approve`/`deny`，丢失原生语义。

## 推荐做法

- 连接前检查解析后 socket 的文件类型、有效用户 owner 和 `0600` 权限；失败时提供安全、可诊断原因。
- 每个物理连接使用唯一 client identity，严格过滤定向消息，并单独处理广播。
- snapshot 建立 revision 基线；只应用连续 patch。缺口进入等待 snapshot，重连清空全部连接态。
- mapper 只挑选已验证的公共状态；未知私有事件安全忽略并记录有限诊断。
- 写动作派发前冻结并重检 connection generation、owner、snapshot revision，以及活动 turn 或 pending request。当前 wire 没有 `expectedRevision` 字段，不得虚构字段，也不得仅凭旧 UI 状态发送。
- 只使用当前打包资源验证的 start v2、steer v1、interrupt v4 和 command/file approval decision v1；response 必须匹配 request id、method 和已绑定 owner handler。no-handler、版本不匹配、owner 变化、断线和超时全部失败关闭。
- `turn.send` 只在已 bootstrap 会话中按权威活动 turn 选择 steer 或 start；`turn.interrupt` 必须带当前活动 turn id；`approval.resolve` 必须精确匹配仍 pending 的原生 request/thread/owner。
- command execution 和 file change 可映射为 v0 二元 Approval。其他不能无损表达的 pending request 保持 waiting 并报告 `capability_unsupported`，不得伪装处理。
- Owner 明确接受只结束本次请求；最终 turn 继续由权威 snapshot/patch 驱动，本地审批决定则需同时取得 Owner ack 与权威 request removal，顺序不限。非幂等写请求不自动重试或跨重连重放。
- capability 只包含当前 adapter 已实现并测试的方法；`conversation.create` 等未接通能力继续 unsupported。
- 无任务目录时明确报告覆盖限制，不用推断数据伪造目录。
- companion snapshot/replay/event 只来自 Desktop adapter。Provider route/event 不得写 Desktop scope 或参与 Desktop action；同名 thread 维持两条独立状态。

## 来源

- `../50-decisions/codex-remote-and-desktop-companion-dual-channel.md`
- `../30-domains/agent-control/codex-desktop-companion.md`
- 当前 Codex Desktop 打包资源中的 Owner/Follower 实现、写方法版本与多客户端探针。

## 验证方式

- frame codec 覆盖 little-endian、短帧、非法 JSON 和 256 MiB 上限。
- 多 client fixture 覆盖定向隔离与广播分发。
- revision 测试覆盖顺序、重复、缺口、等待 snapshot 和重连复位。
- fake Router 测试覆盖 start/steer 选择、interrupt 活动 turn 前置条件、approval 精确定向、request/response 隔离、handler/owner/revision 校验、超时和重连不重放。
- mapper/Provider 测试覆盖 command/file Approval DTO、unsupported pending request 诊断、过期/重复保护，以及 ack 不提前发布最终状态。
- Provider 与前端测试共同断言只展示已声明且当前状态允许的 send、interrupt、approve/deny；错误保留可诊断信息。
- 静态检查确认 Desktop adapter 不调用 App Server fallback、PetApp 不引用 remote client/event，Desktop 私有方法名没有越过 adapter 边界。
- source 测试使用生产 bridge wiring，同时观测 remote、companion 与 Pet event，断言 Provider 只进入 remote bus；并覆盖 remote approval/send/interrupt fail closed。
