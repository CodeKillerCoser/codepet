# 会话消息排队与 Steer 设计提案

## 背景

用户希望在任务运行期间继续输入消息，在手机展示待发送队列，每条支持「引导」「重新编辑」「删除」。本次完成源码调查和设计，尚未实现功能。手机证据来自同机 `/Users/wangxin/Documents/codepet-remote` 工作区。

## 目标

- 同一会话仍只执行一个 turn，但允许继续提交待发送消息；建议默认 FIFO，当前轮正常结束后逐条启动下一轮。
- 引导：将指定队列项送入点击时的活动 turn，不中断后另开任务。
- 重新编辑：从队列撤回后将内容恢复到输入框；再次提交作为新队列项加入尾部。
- 删除：撤销待发送项，不修改输入框，不删除已经送达的聊天历史。
- 手机与桌面主窗口共享队列，手机退出页面、切后台或断线不影响 Host 已受理的队列。

## 非目标

本期不做队列拖动排序、批量合并、已发送消息撤回、原生 Codex 客户端自身队列同步；不恢复已退出生产链路的 Desktop companion，不将 Remote 控制混入 Pet Gateway。具体 UI 和自动执行默认值属于本提案建议。

## 现状理解

| 入口 / 层 | 源码证据 | 当前行为 |
| --- | --- | --- |
| 手机详情 | Remote `lib/application/conversations/conversation_detail_controller.dart` 的 `conversationBlocksSend`、`canSend`、`composerEnabled` | activeTurn、running、waitingApproval、waitingUserInput、archived 阻止发送；同时禁用编辑；还检查连接、交互权、能力、发送中和结果未知 |
| 手机工具栏 | Remote `lib/features/conversations/conversation_detail_screen.dart` | 可停止时显示停止按钮，通过 `else` 替代发送按钮；没有业务待发队列 |
| 桌面主窗口 | `frontend/lib/TaskLineage.svelte::send`、`taskLineage.ts::send` | 前端拒绝 running / waiting-approval；随后 acquireInteraction，再调用 Gateway `turn.send` |
| Host Gateway | `crates/codepet-host/src/gateway.rs::perform_turn_send`、`map_capabilities` | 有 activeTurn 即返回 `turn_already_active`；只调用 Provider `turn.start`；明确丢弃 `TurnSteer` capability |
| Provider | `protocol/provider/v1/schema.json`、Codex `provider.rs::turn_steer` | 已有 `turn.steer`，经 manager 转给 Codex `turn/steer`，携带 `expectedTurnId`；Gateway schema 尚无对应方法 |
| 旧桌面桥 | `src-tauri/src/runtime_gateway/provider_host_compat.rs` | 拒绝 `steerTurnId`，广告 `can_steer: false` |

Codex 和 OpenCode Provider 已广告 `turn.steer`；Claude 明确拒绝，说明其 CLI 排队输入不等价于 Steer。UI 应按能力开放，不能按名称猜测。源码保留的 `codex_desktop_ipc` 具备另一套 steer，但当前 `PetApp.svelte` 使用 Pet Gateway；[通道隔离规约](../60-rules/codex-provider-channel-isolation.md) 已明确 companion 退出生产链路。

现有发送去重是 caller 隔离、容量 1,024 的进程内缓存，且锁持有期间等待 Provider；不能将它直接当作可靠队列。Remote `models.dart::accept` 和 user item 合并通过 turn 关联一个 `clientRequestId`，还会把同 turn 已有任意正式用户消息视为 pending 已收敛。Steer 后一轮多条输入会打破这个假设。

## 实现路径

### 已确认的存储方向（2026-11-08 更新）

用户已授权并实施存储阶段：设置继续使用 JSON，会话和 Provider 记录使用 SQLite；现有 JSON 文档/数组已拆为字段与关联表。conversations 的 14 列已建立，queue 控制字段预留。共享状态使用实际数据根 app-state.sqlite，插件用量使用 providers/<pluginId>/usage.sqlite。

用户明确不迁移、不兼容旧数据，后续重新连接。手机暂不引入 SQLite；任务抽取与谱系保持现有实现。本文的 Gateway Steer、输入队列、操作回执、派发器和客户端 UI 仍为设计，尚未实施。实际存储结构和验证见 [桌面业务状态的 SQLite 存储](../10-architecture/desktop-storage-objects-proposal.md)。

2026-11-08 补充约束：保留会话摘要/详情指纹，将事件字段命名为 last_activity_event_key；不对大段正文、工具输出、完整历史或完整排队请求计算哈希。会话摘要采用有界输入，具体规则见存储提案；队列重复请求仍须核对内容一致性，不能以正文截断哈希代替精确冲突判断，操作表的核对字段留待单独确定。

Host 负责权威队列的持久化、派发和操作回执。队列项、选项、回执都需要结构化字段，不能把整个请求或响应作为 JSON 文档保存。手机继续保存会话投影与页面草稿，通过 Host 快照和操作回执恢复队列状态；不新增手机数据库，不承诺本地草稿跨进程退出恢复。

撤回编辑的正文恢复优先由 Host 保留按调用方隔离的撤回记录，直到客户端明确确认恢复或用户显式丢弃；已恢复后尚未再次提交的本地编辑，仍受手机当前草稿生命周期限制。具体恢复协议待对象讨论后确定。手机端不存在 SQLite 与 Host 的跨库事务；操作结果未知时仍先查询，不能自动重发。

### 交互与队列归属

建议 Host 持久化每个完整会话路由的队列，手机和桌面订阅同一快照与事件。客户端页面仅持有投影与草稿；不要把队列放进详情 controller 或会因断线、LRU 淘汰而清空的消息缓存。

空闲且队列为空时，提交项立即尝试派发；运行中提交显示「加入队列」，保留独立「停止」按钮。输入框上方展示「待发送 N 条」，每项显示顺序、两行摘要、状态和三个具名操作；多项可折叠或限高滚动，键盘出现时输入框及操作仍可见。尚未获得 Host 受理回执时保留草稿和提交中状态；离线可编辑本地草稿，不能显示为已入 Host 队列。

| 操作 | Host 原子语义 | 手机反馈 |
| --- | --- | --- |
| 引导 | 将 queued 项独占认领为 steering；绑定请求中的活动 turn；调用 Provider steer | 受理成功后移出待发列表；正式消息由事件收敛；失败保留内容和原因 |
| 重新编辑 | 原子撤回 queued 项，返回原内容与配置及撤回回执 | 回填并聚焦、光标移至末尾；已有草稿先保存为可恢复草稿，不静默覆盖 |
| 删除 | 原子取消 queued 项，保留操作回执 | 从队列移除，不影响输入框 |

Host 撤回时一并保存恢复正文及操作 ID，手机在确认撤回成功后回填。若响应丢失或页面退出，以 Host 操作回执核对并恢复原始正文；手机本地后续编辑暂不保证跨进程恢复。发送中的项不可再编辑或删除；并发输给派发时明确显示「该消息已开始发送」。已送入模型的内容不能承诺撤回。

### 协议与状态

保持现有 `turn.send` 的“空闲会话新开一轮”合同，新增 Gateway `turn.steer`，复用 Provider 的既有实现。Steer 请求建议采用统一的 `conversation + turn + clientRequestId + capabilityRevision + input`；校验同路由、turn 归属、当前活动状态及 capability。Steer 沿用当前轮配置，不携带待发项的新 model / accessMode / effort；UI 提示这些选项仅在新一轮生效。

另新增 Host 队列 API，建议命名 `conversation.inputQueue.{get,enqueue,withdraw,steer,resume}`，以及携带单调 revision 的 changed 事件。它们是提案名称，最终需加入 schema/manifest 后生成 SDK。`withdraw` 区分 edit/delete 并返回稳定回执；steer 操作由 Host 独占队列项后调用内部统一 steer 服务，客户端不能先 delete 再 steer。

每项保存 queueItemId、完整路由、Host 分配的顺序、创建者、正文、提交时 selection、state、revision、派发操作 ID、目标 turn 和错误。快照带队列运行状态与事件 fence；事件乱序、重复或 replay 过期时按 revision 合并或重取队列快照，不重置聊天历史。入队/撤回/引导命令通过稳定 caller scope + 操作 ID 去重并校验 payload 冲突，显式定义回执保留窗口。

状态建议为 `queued → dispatching / steering → accepted`；未派发项可变为 `withdrawn`。明确业务拒绝进入 `blocked` 并保留内容，允许撤回；发送结果未知进入 `outcomeUnknown`，冻结该项及自动续发。队列另有 running / paused 状态。accepted 表示 Harness 已受理，不代表该任务已完成。

同会话短锁或 actor 负责认领、撤回和状态转换；数据库事务先写派发意图，再发非幂等 RPC，外部 RPC 期间不持有全局队列锁。普通直接发送、自动出队、Steer 与停止都通过同一会话协调入口；队列有待发项时旧 `turn.send` 返回明确冲突，不能插队。各会话独立推进。外部原生客户端仍可能改变状态，最终由 Provider 的 start 活跃检查和 steer 目标检查兜底。

### 自动推进与恢复

- 正常 completed 事件触发重新核对活动轮、审批、输入等待、Provider 可用性及交互权，然后仅派发队首一条；派发回执先登记活动轮，不能因 snapshot 滞后连续启动多轮。监听事件早于响应、重复终态和迟到旧轮事件都不得重复推进。
- failed / interrupted 默认暂停队列；用户点停止时先暂停自动续发，再中断当前轮，避免刚停止又执行下一条。显式释放交互权也暂停队列，保留待发内容。
- 等待审批或结构化输入时可排队，但不自动开下一轮；首版禁用引导，不用 Steer 代替审批或回答结构化请求。不支持 Steer 的 Provider 仍可按串行 start 支持队列。
- 引导时目标轮已结束或变化，返回冲突并暂停该项等待用户处理；不悄悄改为新轮发送。引导可选择任意待发项，成功后剩余项保持原顺序。
- 手机断线不暂停已受理队列。Host 重启后先核对 Provider 与交互权，未派发项仍保存；重启前 dispatching / steering 项先视为结果未知，不能盲目重发。交互权被其他客户端持有时标记 blocked，不能强制接管。
- 能力 revision 或模型失效时重新校验保存的 selection，无法确认时暂停并显示原因，不静默换模型或提高权限。release / stop / archived / Provider 卸载均需明确暂停或终结队列；不要因排队或手机心跳永久固定共享 Server 生命周期。

Host 的持久化受理不能保证 Harness exactly-once。Codex client 可能收到 steer ack 后因读取权威 turn 失败返回 `SentOutcomeUnknown`；即使有 clientRequestId 也不能自动重发。无足够关联证据时保持待核对状态。

### 聊天消息收敛

队列项属于“待发送”，不要入队即伪造成已送达聊天消息。普通发送和 Steer 分别有稳定的每次输入 ID；同 turn 允许多个用户 item。优先用 provider 返回的用户 item ID / 明确输入关联收敛，不能用 turnId 或相同文本猜测。现有 Provider steer 回执仅返回 turn，是否能获得足够原生关联还需验证；关联不足时显示独立「引导已受理，等待消息同步」状态，等待正式事件，不借用 `accept(TurnSendReceipt)` 隐藏占位消息。

## 涉及模块

- `protocol/gateway/v1/{schema,manifest}.json`、`tools/protocol-codegen`、生成 Rust/TS/Dart SDK：增加 steer、队列 DTO、能力和事件；手机 SDK 必须从同一 schema 导出，不能手改生成文件。
- `crates/codepet-host/src/gateway.rs`、新增队列存储/调度模块、`providers/manager.rs`：统一受理、持久化、单会话仲裁、结果未知恢复和事件发布；SQLite 路径沿用项目 path manager 边界。
- Codex Provider `provider.rs`、`client.rs`、`mapper.rs`：复用 steer 和代际生命周期，核实用户 item 关联；其他 Provider 按广告能力接入。
- 桌面会话 UI：后续确定独立的队列接入位置。`TaskLineage` 等文件仅作为现状调查证据，本轮不修改任务抽取或谱系页面。
- Remote `application/ports/gateway_client.dart`、`gateway/gateway_client.dart`、`application/sessions/device_session.dart`：承载新接口、连接代际隔离和设备级队列投影。
- 新增 Host `InputQueueStore` 及 schema 升级：保存权威队列和可核对的撤回/派发记录；Remote 只新增队列查询与操作接口，不新增 SQLite。
- Remote `application/conversations/conversation_detail_controller.dart`、`features/conversations/conversation_detail_screen.dart`、`core/domain/models.dart`：拆分可编辑/可入队/可引导条件，队列操作与焦点恢复，修正一轮多输入收敛。

建议分三步交付：先开放并验证 Gateway steer；再完成 Host 队列及竞态测试；最后接手机和桌面 UI。只开放 Steer 还不构成用户要求的完整队列功能。

## 风险

- 多端撤回、引导与自动出队竞争：通过 revision、持久化认领与操作回执；测试两个客户端同时操作同一项，仅一条路径可派发。
- 断线或重启导致重复执行：测试 RPC 发出前后、ack 前后、落盘前后故障；结果未知必须停住，不能承诺 exactly-once。
- 多次 Steer 覆盖消息、乱序终态推进多项：测试一轮三条用户输入、相同正文、事件先于响应、响应先于事件及旧代事件。
- 队列随页面销毁、缓存淘汰丢失或编辑覆盖草稿：测试返回页面、LRU 淘汰、离线重连、编辑时已有草稿及撤回响应丢失。
- 队列继续执行违背停止/释放意图：测试 stop、release、审批和用户输入等待都阻止自动推进；其他会话仍能发送。
- 手机上队列挤压输入或动作不可见：人工与 widget 检查 320 / 390 宽度、长文本、大字体、软键盘、三项操作可点击和编辑后焦点；桌面检查窄窗与键盘导航。
- SQLite 升级、落盘失败或本地恢复记录损坏：使用临时数据库验证版本升级回滚、重复初始化、磁盘写入失败时零外部派发及数据保留；手机进程重启后核对 Host 队列及未确认撤回记录，不将 Activity 重建测试等同于持久草稿恢复。
- 队列快照被误作权威状态，或草稿被当缓存清理：测试离线快照禁用写操作、重新配对隔离、清理缓存保留草稿、入队响应不清除较新的草稿 revision；架构测试约束 UI 不直接导入数据库。

## 测试计划

实现阶段新增 Gateway capability / 错路由 / stale turn / 去重测试，Host 队列持久化与并发测试，Codex fixture 同 turn 多输入测试，Remote controller / widget / 消息收敛测试。同步 Dart SDK 后运行 Flutter analyze 与会话/Gateway 测试；桌面运行相关 Vitest，协议运行 `npm run protocol:check`，Host 与 Provider 执行定向 Cargo 测试。完成后真机验证手机锁屏、断网和恢复。

本次调查已执行：

- `npm run protocol:check`：生成文件一致，22 项通过。
- Remote：`/Users/wangxin/develop/flutter/bin/flutter test --no-pub test/conversations/conversation_detail_screen_test.dart --plain-name 'disables empty, unavailable, active-turn and offline sends while retaining draft'`：1 项通过，证实现有发送限制。
- 存储补充调查：在 Remote 执行 `/Users/wangxin/develop/flutter/bin/flutter test --no-pub test/devices/device_registry_test.dart test/conversations/conversation_message_cache_test.dart`：17 项通过，覆盖现有设备元数据/凭据隔离、串行提交与回滚，以及消息缓存淘汰和失效；这不是 SQLite 新方案的验证。
- 未执行真实 Codex Steer、Host 新队列测试或 UI 验收；以上是调查阶段的验证，不代表 Steer/队列功能验证完成；后续存储阶段的代码与测试见存储文档。

## 知识沉淀

实现落地时更新 [Gateway 发送受理规约](../60-rules/gateway-turn-send-admission.md)，保留现有 best-effort 限制并追加队列与 Steer 的边界；复用 [手机消息数据源规约](../60-rules/remote-conversation-message-sources.md) 与 [SDK 同步规约](../60-rules/remote-sdk-synchronization.md)。队列操作不得触发历史全量重拉。若真实断线恢复暴露固定排查路径，再补 runbook。

## 未知项

- 用户尚未明确 FIFO 自动续发、失败/停止后暂停、Host 重启后继续策略；本文给出推荐默认值。
- 本次未核实安装版 Codex 的 steer 消费时机和每次输入关联字段，不能承诺请求受理即刻改变正在执行的工具。
- 未验证手机安装包是否对应所读 Remote 源码；已有 companion 源码不代表当前生产入口可用。
- 存储已按全新格式实现并覆盖并发、重启、事务失败、配对与查询快照；待发消息和派发结果恢复尚未实现。手机 SQLite 继续暂缓。
