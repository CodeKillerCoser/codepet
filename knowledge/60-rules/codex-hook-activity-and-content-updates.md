# Codex Hook 是活动事实来源，Remote 被动更新与手动刷新

## 规则

Provider 不维护写冲突会话 ID，也不根据控制权筛选 Hook。Hook 适配为现有活动与内容更新事件；Remote 被动接收时只更新状态及有正文的实时事件，不因无 item 提示或最近列表失效提示发起查询；用户可手动刷新。

活动成员关系由实际收到的 Hook 事实决定：未知状态收到工具或审批 Hook 即可建立 running/waiting 状态，无需先收到 start；已明确结束的会话仍需新的 start，避免迟到工具事件复活。`conversation.active.list` 有现存消费者，因此保留派生投影；原生 loaded threads、resume 成败、Provider 启停和摘要状态都不能成为第二个活动事实来源。

loaded threads 仍可参与主分支既有的会话并集发现，只提供候选 ID 和元数据，不决定活动成员关系。

Hook 按尽力投递处理，不保证必达。不增加历史重放、持久化恢复或独立 App Server 对账；后续有效 Hook 建立未知状态属于处理当前事实，不是恢复漏掉的事件。没有新 Hook 时仍无法即时获知状态。

## 适用场景

- Codex Provider：外部 writer 阻止原生消息订阅，但不阻止独立 Hook 来源；内部观察订阅在实例启动时建立，无需先 resume。
- Provider SDK：Codex 选择显式活动事件模式，防止原生摘要自动转换为活动事件；其他 Provider 保留既有模式。
- Provider/Gateway IDL、Host 路由和共享未读状态：空 item 仍须有可验证的会话身份，稳定 `updateId` 用于内容活动去重。
- Remote 消息缓存与详情 controller：控制权是接收端状态，补拉与分页、实时事件共用同一数据源。

## 反例

登记 resume 冲突后才投递 Hook；有控制权时由 Provider 丢弃 Hook；以 `thread/loaded/list` 或原生 running 摘要建立全局活动成员；Provider 停止就清空外部会话活动；工具事件在 stop 后复活会话；补拉直接替换全部历史。

## 推荐做法

- 共用 Observation 接收器。内部订阅不作为未登记的 Host 订阅转发，Pet 原始事件独立分发。投影失败不抑制 Pet 通知。Hook 来源安装失败时不声明 active-list 能力，其余原生控制能力继续独立工作。
- `SessionStart/UserPromptSubmit` 转为开始，`SessionEnd/Stop/Interrupt` 转为结束，复用 `event.conversationActiveChanged`。工具与审批 Hook 可以建立未知会话状态，也可细化已活动状态；已知 stop 后不自行重新加入活动列表。子代理结束不结束父会话。
- 内容变化复用 `event.conversationItemUpserted` → Gateway `conversation.itemUpserted`：正常通知带 `item`，Hook 提示带 `conversation` 和 `updateId`，不带 `item`。有无控制权都投递；不新增 SessionUpdate 事件，不编造正文或 turn。
- 活动投影按原生会话 ID 缓存 Hook 事实，与执行句柄和 writer 生命周期分离。重复事件去重，过旧生命周期事件丢弃；生命周期与内容分别比较时间，防止较晚工具更新遮蔽迟到的 stop。Provider server 重启不清除外部 Hook 活动。
- 缓存的真实摘要应用已知 Hook 状态，结束时去掉旧 active turn；原生查询同样应用投影，避免覆盖已知事实。尚无任何 Hook 状态证据时可保留原生执行详情，但不能从原生摘要产生活动事件或 active-list 成员。未知会话的 active-list 仅返回资源和状态等字段，不发布猜测的标题摘要。
- Gateway 沿用活动事件使最近会话列表失效；有真实摘要时，摘要更新也送达详情。Host 校验空 item 的路由；item 与 conversation 同时存在时必须一致。共享未读状态按稳定 updateId 去重内容提示。
- 用户手动刷新消息复用 `conversation.get` 分页 API。当前 Codex adapter 使用 `thread/turns/list`，没有 `thread/item/list` 适配。协议 `ConversationItem` 表示 turn 内消息、推理、命令、文件变更、工具、审批等项，可理解为 step，不等于整个 turn。
- 手动消息刷新单飞；期间被动提示不追加读取。按 `(turnId,itemId)` 合并并保留旧页、游标和事件窗口；并发到达的正文或 canonical item 优先。缓存关闭或版本变化后丢弃在途结果，失败保留原内容。
- 无控制权摘要不再携带活动 turn 时，移除旧非终态投影。结束后保留文字、停止流式显示，避免旧 running turn 继续驱动活动 UI。

## 来源

2026-09-09 用户对 Provider/Remote 职责的纠正；实现证据为 `provider/hook_observation.rs`、`provider.rs::conversation_active_list`、SDK `explicit_activity_event_sink`、Remote `ConversationMessageSource`。相关约束见 [通道隔离](codex-provider-channel-isolation.md) 与 [消息数据源](remote-conversation-message-sources.md)。

## 验证方式

- Codex Hook 单元测试覆盖：resume 前更新、有执行槽仍投递、重复事件、跨 server 启停、迟到工具不复活、内容时间不遮蔽生命周期、原生摘要不能建立或结束活动成员。
- SDK 测试覆盖：显式活动模式不从原生摘要生成活动事件，活动通知推进事实版本；原有 Provider 模式不变。
- Remote 缓存测试覆盖：空 item 零额外查询、手动刷新单飞、并发正文优先、运行时切换丢弃在途结果、失败保留历史。最近 controller 覆盖连续 100 次失效零查询、已加载行原位状态更新和手动恢复分页。
- 协议 fixture 和生成新鲜度校验，Host 路由校验，Provider 启停集成测试覆盖跨模块回归。

### 验证记录（2026-09-09，Windows）

- `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --lib`：62 项通过。
- `cargo test --manifest-path sdk/rust/Cargo.toml -p codepet-provider-sdk --lib conversation_atoms::tests --target-dir crates/target`：6 项通过。
- `cargo check --manifest-path crates/Cargo.toml -p codepet-provider-codex -p codepet-host --tests`：通过。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --test provider_vertical start_returns_starting_and_stop_discards_pending_probes -- --exact --test-threads=1`：通过，使用隔离的临时 CODEX_HOME。
- 同级 Remote 仓库 `flutter test --no-pub test/conversations/conversation_message_cache_test.dart`：7 项通过。
- 整合主工作区的并集发现变更后：Provider/Host `cargo check --tests` 通过；Codex `--lib` 64 项、`--test provider_vertical union_directory -- --test-threads=1` 3 项通过；Remote 消息缓存与 Gateway mapper 测试共 11 项通过。
- 此前同一功能的协议检查 20 项、SDK conversation_state 测试 8 项、Remote conversations/gateway/device_session 测试 195 项通过。Host lib 测试 57 项通过，既有日志轮转测试在 Windows 遭遇拒绝访问（OS error 5），未修改该模块。

2026-11-25 用户明确：Provider 重启后应由后续实际 Hook 恢复未知状态；手机被动观察时不主动补拉，保留手动刷新。该要求取代 2026-09-09“工具 Hook 不能建立未知活动”和“空 item 自动补拉”的旧限制。仍不使用 loaded threads 补造事实。SessionStart 表示生命周期开始，不证明正在生成 token。现场证据与本次验证见 [重启后运行状态缺失](../40-runbooks/codex-hook-status-after-provider-restart.md)。
