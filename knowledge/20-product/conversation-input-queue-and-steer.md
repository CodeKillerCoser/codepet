# 会话消息排队与 Steer

## 背景

任务运行时原先禁止继续发送，手机的停止按钮也会替代发送按钮。现在手机和桌面会话输入区共用 Host 权威队列，允许继续输入，每条提供「引导」「重新编辑」「删除」。本次源码实现于 2026-11-08；手机代码位于同机 `/Users/wangxin/Documents/codepet-remote`。

## 目标

- 默认 FIFO；当前 turn 正常完成后，逐条启动下一轮，同会话不并行执行多个 turn。
- 引导指定待发消息进入当前 turn；重新编辑先撤回再回填；删除只移除待发项。
- Host SQLite 保存已受理队列，手机断线、退出页面不影响派发；手机不引入 SQLite。
- 多端操作、重试和异常恢复不重复派发，不哈希完整消息正文。

## 非目标

不做拖动排序、已送达消息撤回、原生客户端自身队列同步或任务抽取改造。聊天历史仍由 Provider/Harness 提供。手机进程退出及桌面窗口关闭后的未提交草稿恢复不在本期保证范围内；暂不清理历史操作账本。

## 现状理解

Gateway 新增 `turn.steer` 及 `conversation.inputQueue.{get,enqueue,withdraw,steer,resume,acknowledgeEdit}`；schema/manifest 是唯一协议来源，Rust、TypeScript、Dart SDK 自动生成。`conversation.inputQueueChanged` 是可重放事件，只携带路由和 revision，客户端重取快照，不重载聊天历史。

Provider 已有 `turn.steer`，继续复用它：Codex 使用原生 `turn/steer` 和 expectedTurnId；其他 Provider 按能力开放，不按名称判断。不支持 Steer 的 Provider 仍可排队。旧兼容桥没有队列合同，桌面新组件直接调用规范 Gateway。

## 实现路径

### 结构化存储

沿用 `app-state.sqlite` 和 `conversations` 的 `queue_paused / queue_revision / next_queue_sequence`。新增队列领域的关联表，不把请求或完整 DTO 序列化成 JSON 列：

| 表 | 存储内容与用途 |
| --- | --- |
| `conversation_inputs` | 主键、会话外键、创建调用方、请求 ID、FIFO 序号、能力 revision、正文、模型/权限/推理配置独立列、状态、时间、目标 turn、错误、撤回类型/调用方及编辑确认标志；每条输入的权威账本 |
| `input_queue_operations` | 会话、调用方、操作请求 ID、类型、项 ID、目标 turn、能力及期望 revision；撤回和引导重试去重 |
| `input_queue_runtime` | 会话及已知活动 turn；避免回执已到、快照尚未更新时连发下一条 |
| `input_queue_terminal_turns` | 会话、turn ID 和终态；处理终态早于回执及重复、迟到终态 |

入队按会话、调用方、clientRequestId 唯一；逐字段比较原请求，不计算正文哈希。内部 Provider 输入 ID 使用 `queue:<项 UUID>`，避免不同客户端同名请求冲突。历史账本当前保留至数据库被显式清理，不设置过期后盲目重发窗口。待发项与未确认编辑恢复项合计最多 100 条、正文共 1 MiB，单条入队正文最多 128 KiB，确保快照编码后仍有传输余量。

### 操作与状态

| 操作 | Host 行为 | 客户端行为 |
| --- | --- | --- |
| 入队 | 事务分配序号并保存；重复请求返回原项 | 受理后清空对应草稿；输入框后来新写的内容保留；响应未知时复用原请求核对 |
| 引导 | 校验同路由、当前活动 turn、能力 revision 和 running 状态，独占认领后调用 Provider | 受理后移出待发列表，显示等待正式消息同步；沿用当前轮的模型与权限 |
| 重新编辑 | 事务撤回 queued/blocked 项，保留仅撤回调用方可见的恢复记录 | 回填正文及配置，聚焦并移动光标；原草稿先备份；回填后 acknowledgeEdit |
| 删除 | 撤回 queued/blocked 项，保留去重账本 | 输入框不变；不删除聊天历史 |
| 继续队列 | 核对 revision、当前活动 turn 和交互权，解除暂停并重试 blocked 项 | 用户明确继续；结果未知的项必须先核对处理 |

状态为 `queued → dispatching / steering → accepted`，明确拒绝变为 `blocked`，不确定是否送达变为 `outcomeUnknown` 并暂停。未派发项可变为 `withdrawn`。发送中不可编辑或删除；结果未知只能在核对会话后移除记录，不能直接编辑重发。

每个会话有独立锁，数据库认领先于外部 RPC；外部调用不持有全局发送锁。直接 turn.send 在存在待发项时拒绝插队。Steer 的活动轮已改变时暂停队列并返回冲突，不转成新一轮。入队事件发布失败不把已提交事务误报成失败，客户端三秒刷新作为补偿。

### 自动推进与恢复

Host 调度器由事件唤醒并定期扫描，派发前检查 Provider、活动 turn、能力、selection 及交互权；不强制接管。completed 允许下一条；failed/interrupted、主动停止、释放交互权和 Provider 不可用会暂停。等待审批/结构化输入时可以排队，但不派发、不开放 Steer。

Host 重启时清除上一进程的 input_queue_runtime 占用，派发前仍查询 Provider 的实际 active turn；accepted 输入不会重发。Host 重启后暂停保存的待发项；遗留 dispatching/steering 改为 outcomeUnknown。RPC future 被取消也记录结果未知。正常发送新消息即为继续处理的显式意图：若无 blocked/outcomeUnknown 项，解除暂停并按 FIFO 派发，无需另点“继续队列”；有失败或结果未知项时明确拒绝新发送并保留草稿，核对移除旧项后再发送。Host 持久化受理不等于 Harness exactly-once，不能凭客户端请求 ID 承诺原生执行去重。

队列项不作为乐观聊天消息插入，也不按 turn ID 或相同正文匹配用户消息。Steer 仅返回 turn 时等待正式 item 事件，支持同轮多次输入而不覆盖原消息。

## 涉及模块

- `protocol/gateway/v1`、三个生成 SDK 和手机导出 SDK：统一 DTO、能力、重试语义与事件结构。
- Host `input_queue.rs / input_queue.sql`、`gateway/input_queue.rs`、`gateway.rs`：事务账本、认领、派发、生命周期协调和事件。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs`：把规范 Gateway 事件转发给主窗口；兼容桥显式忽略新合同。
- `frontend/lib/ConversationComposer.svelte`、`conversationInputDrafts.ts`：队列与草稿操作。`LineageMessages` 和 `TaskLineage` 仅接入会话输入组件，不修改任务抽取或谱系存储逻辑。
- 手机 `application/ports/conversation_input_queue_gateway.dart`、Gateway mapper/client、DeviceSession：可选队列接口、设备/连接隔离及队列 revision 提示。
- 手机 controller、详情页和 `conversation_input_queue_view.dart`：状态与操作、焦点、草稿保护、有限高度滚动区；不直接访问数据库。

## 风险

- 重试或多端竞态重复执行：Host 事务和持久化操作记录防护，存储及集成测试覆盖去重、过期 revision、独占认领和旧终态。
- 中断/崩溃后误续发：dispatch guard、启动恢复和停止 Provider 测试验证暂停；真实 Harness 的强杀时序仍需安装联调。
- 草稿覆盖或发送结果未知：两端交互测试验证旧草稿恢复、输入期间 ack、原请求重试；未提交草稿跨进程持久化仍未提供。
- 小屏挤压输入：Flutter widget 测试覆盖 320×800、1.5 倍字体、300 高软键盘；桌面组件已用浏览器检查窄窗，整机窗口还需发布验收。

## 测试计划与结果

已运行：

- `npm run protocol:check`：23 项通过，包含合法队列快照、事件、Steer 以及缺少 revision 的非法 fixture。
- `cargo test -p codepet-host --manifest-path crates/Cargo.toml --quiet -- --test-threads=1`：158 项通过、4 项依赖外部环境的测试忽略；新增 6 项队列存储和 4 项 Gateway 集成用例。并行运行的旧进程文件描述符断言会受其他测试干扰，使用单线程。
- `npx vitest run frontend`：209 项通过，新增 5 项实际组件行为测试；无路径过滤会误收集 Node 原生测试，不能当作 Vitest 运行。
- 补充全量 `svelte-check` 未通过：44 条错误位于既有测试、QA 与 Node 类型引用，新增队列组件无错误；检查器使用仓库 TypeScript 5.9.3，默认临时安装误取 TypeScript 7 导致工具启动失败。此项不宣称全仓类型检查通过。
- `npm run build`、Tauri `cargo check`（测试环境关闭 macOSPrivateApi 并清空资源清单）通过；这不是安装包验收。
- Remote `flutter analyze --no-pub` 无问题；`flutter test --no-pub` 463 项通过，含 4 项新增队列交互回归。

发布前：安装两端新包，以真实 Codex 验证运行中连续入队、同轮多次 Steer、跨设备编辑/删除、断网重连、Host 强杀恢复及原生终态可靠性。

## 合入 main 的验证

2026-11-08 合并桌面 `main` 的 `b8d22f0`，保留新 Provider release-control 能力映射和仅测试环境启用的 Svelte browser 条件；手机从 `main` 的 `ee7aaea` 独立合入队列提交，未捎带原工作分支的 Live Updates 功能。按合并后的协议生成器重新导出手机 SDK，`--check` 通过。

合并后重跑 `npm run protocol:check`（23 项）、`npx vitest run frontend`（175 项）、`npm run build`、Host 与 Provider 数据层合并回归（201 项通过、4 项依赖外部环境忽略）及 Tauri `cargo check`，全部通过。手机在新工作区分别获取应用及 `packages/codepet_connection_engine` 依赖，`flutter analyze --no-pub` 无问题，`flutter test --no-pub` 469 项通过。构建按现有 main 工作流执行：桌面 all 平台且 publish=false，Android release APK；未改变基础版本。

## 知识沉淀

[桌面状态存储规约](../60-rules/desktop-state-storage.md) 延伸到队列关联表；本文件替换原设计提案，记录当前行为、恢复边界和验收缺口。

## 未知项

尚未连接真实手机和安装后的 Harness 做端到端验收。Provider 永久遗漏终态且未发生状态变更时，已知活动轮不会凭空推断为完成；这类原生事件可靠性需实测。历史操作账本持续增长的清理方案尚未设计，不能擅自删除后重新受理旧请求。
