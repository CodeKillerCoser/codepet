# Codex Desktop IPC 作为任务事实来源

> 状态：已被 `codex-remote-and-desktop-companion-dual-channel.md` 取代。本页保留阶段一/二为何要求 Desktop IPC fail closed 的历史依据；这些约束继续适用于 Desktop companion，但“不允许独立 App Server”不再适用于隔离的 remote runtime。

## 背景

Code Pet 需要跟随用户正在使用的 Codex Desktop 任务。Code Pet 自行启动的 `codex app-server --listen stdio://` 是独立运行实例，不能代表 Desktop 当前 owner 的实时状态，也可能让控制动作发往错误实例。当前 Desktop 提供私有 Owner/Follower IPC，但协议与 Desktop 版本耦合，且没有从零枚举全部任务的目录。

## 决策

Codex Provider 只连接当前用户运行中的 Codex Desktop 私有 IPC，并以 Owner/Follower snapshot、完整历史和连续 patch 作为任务状态事实来源。

Desktop socket 或握手不可用、协议不兼容、路由无法确认、revision 出现缺口时必须 fail closed。不得启动独立 App Server、读取 transcript、恢复 Hook 或监听文件作为回退。

Desktop 私有 DTO、方法名和路由字段只存在于 Codex Desktop adapter。Runtime Gateway、Standard Protocol 和 UI 只接收映射后的公共语义。阶段二只为同一条已 bootstrap、owner 仍有效的 follower 会话接通 `turn.send`、`turn.interrupt`，以及 command execution/file change 两类可无损映射为二元决定的 `approval.resolve`；其他写能力继续保持 unsupported。

写请求的成功只表示目标 owner 明确接受请求，不表示任务或审批已经进入最终状态。turn 最终状态仍由权威 snapshot/patch 驱动；本地审批决定只有 Owner ack 与权威 request removal 两项证据齐全后才发布，顺序不限。超时、断线或 owner 换届后的非幂等动作不得自动重放。

## 备选方案

- 继续启动独立 App Server：实现较公开，但它是另一个实例，不能可靠跟随 Desktop owner，因此不作为运行时数据源。
- Desktop IPC 失败时回退 App Server：会让同一个 Provider 状态在两个实例间切换，无法保证任务和控制目标一致，因此拒绝。
- transcript、audit 或文件监听：只能事后推断且缺少权威 revision、路由和审批语义，因此拒绝。
- 恢复 Codex Hook：Hook 不是完整任务协议，也不能完成晚加入 bootstrap，因此拒绝。
- 等待公开稳定 API：兼容性风险最低，但当前无法形成 Desktop 任务展示切片；选择私有 IPC，同时明确版本耦合和 fail-closed 成本。

## 取舍理由

- Owner/Follower 通道直接连接当前 Desktop owner，是现有证据中唯一能对齐用户正在运行实例的实时来源。
- 4 字节 little-endian frame、唯一 client identity、定向路由和 revision snapshot/patch 已在当前实现与探针中验证，可以建立可测试的正确性边界。
- 当前打包资源把 follower start、steer、interrupt 分别标为 v2、v1、v4，把 command/file approval decision 标为 v1；方法版本、目标 owner 和 response handler 都属于兼容边界。
- 当前写方法没有 `expectedRevision` wire 字段。adapter 必须在本地冻结 generation、owner、snapshot revision、活动 turn 或 pending request，并在派发前重检；不得虚构私有字段或仅凭旧 UI 状态乐观发送。本地 fence 只覆盖写入前，不能宣称为 wire CAS；start/steer 写出后的 owner 状态竞态，以及 approval `ok` 对跨客户端 no-op 的归因限制，都是当前私有协议无法消除的边界。
- 无任务目录意味着覆盖范围有限，但明确报告限制比通过 transcript 伪造完整目录更可靠。
- 当前联调证明显式已知 id 可以完整附着，但空 known-set 的新 client 不保证收到现有 owner 任务公告；被动 follower 公告只能补充发现，不能改变“无目录”的结论。
- 把私有协议限制在 adapter 内，可以保留 Gateway、IDL、Provider 多实现抽象和 UI 的稳定边界。

## 影响范围

- Codex Provider 的可用性由 Desktop socket 和 initialize/握手决定，不再依赖 Codex CLI 路径。
- 旧 App Server 启动、回复和审批代码不能继续作为生产路径或 fallback。
- revision 缺口、断线重连和晚加入 bootstrap 成为核心测试对象。
- Standard Protocol 方法可以继续服务其他 Provider，但 Codex capability 只能广告 adapter 已实际接通并完成路由、并发前置条件、响应确认和失败测试的方法。
- `conversation.create`、permissions approval、MCP elicitation、user input、plan implementation 等不能由当前 v0 二元 IDL 无损表达的能力继续明确 unsupported，不通过 CLI 或独立 App Server 绕路。
- 发布验证需要绑定当前 Codex Desktop 私有协议版本；不兼容时用户会看到明确 unavailable，而不是静默降级。

## 后续观察

- 跟踪 Codex Desktop 是否提供稳定任务目录或公开 follower API；在此之前不承诺无已知 thread id 的完整枚举。
- 每次 Desktop 升级复核 Owner/Follower v1、当前状态广播 v11、写方法版本、bootstrap 顺序和路由语义。
- 新增审批类别或其他写操作前，必须先证明 Standard Protocol 能无损表达其决定，并验证目标路由、并发前置条件、Owner 确认和失败状态，再单独增加 capability。

本决策曾取代 `codex-app-server-as-primary-reply-path.md`，现由 `codex-remote-and-desktop-companion-dual-channel.md` 取代。
