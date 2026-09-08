# Codex Hook 是活动事实来源，Remote 决定内容补拉

## 规则

Provider 不维护写冲突会话 ID，也不根据控制权筛选 Hook。Hook 适配为现有活动与内容更新事件；Remote 仅在 `item == null && !interactionAcquired` 时补拉最新内容。

活动成员关系只由 Hook start/stop 决定。`conversation.active.list` 有现存消费者，因此保留派生投影；原生 loaded threads、resume 成败、Provider 启停和摘要状态都不能成为第二个活动事实来源。

Hook 按尽力投递处理，不保证必达。允许漏事件导致活动投影暂时不准确，不为此增加重试、补偿、恢复或对账机制。

## 适用场景

- Codex Provider：外部 writer 阻止原生消息订阅，但不阻止独立 Hook 来源；内部观察订阅在实例启动时建立，无需先 resume。
- Provider SDK：Codex 选择显式活动事件模式，防止原生摘要自动转换为活动事件；其他 Provider 保留既有模式。
- Provider/Gateway IDL、Host 路由和共享未读状态：空 item 仍须有可验证的会话身份，稳定 `updateId` 用于内容活动去重。
- Remote 消息缓存与详情 controller：控制权是接收端状态，补拉与分页、实时事件共用同一数据源。

## 反例

登记 resume 冲突后才投递 Hook；有控制权时由 Provider 丢弃 Hook；以 `thread/loaded/list` 或原生 running 摘要建立全局活动成员；Provider 停止就清空外部会话活动；工具事件在 stop 后复活会话；补拉直接替换全部历史。

## 推荐做法

- 共用 Observation 接收器。内部订阅不作为未登记的 Host 订阅转发，Pet 原始事件独立分发。投影失败不抑制 Pet 通知。Hook 来源安装失败时不声明 active-list 能力，其余原生控制能力继续独立工作。
- `SessionStart/UserPromptSubmit` 转为开始，`SessionEnd/Stop/Interrupt` 转为结束，复用 `event.conversationActiveChanged`。工具与审批 Hook 只能细化已活动会话的状态，不能自行加入活动列表。子代理结束不结束父会话。
- 内容变化复用 `event.conversationItemUpserted` → Gateway `conversation.itemUpserted`：正常通知带 `item`，Hook 提示带 `conversation` 和 `updateId`，不带 `item`。有无控制权都投递；不新增 SessionUpdate 事件，不编造正文或 turn。
- 活动投影按原生会话 ID 缓存 Hook 事实，与执行句柄和 writer 生命周期分离。重复事件去重，过旧生命周期事件丢弃；生命周期与内容分别比较时间，防止较晚工具更新遮蔽迟到的 stop。Provider server 重启不清除外部 Hook 活动。
- 缓存的真实摘要应用已知 Hook 状态，结束时去掉旧 active turn；原生查询同样应用投影，避免覆盖已知事实。尚无生命周期证据时可保留原生执行详情，但不能据此产生活动事件或 active-list 成员。未知会话的 active-list 仅返回资源和状态等字段，不发布猜测的标题摘要。
- Gateway 沿用活动事件使最近会话列表失效；有真实摘要时，摘要更新也送达详情。Host 校验空 item 的路由；item 与 conversation 同时存在时必须一致。共享未读状态按稳定 updateId 去重内容提示。
- Remote 补拉复用 `conversation.get` 分页 API。当前 Codex adapter 使用 `thread/turns/list`，没有 `thread/item/list` 适配。协议 `ConversationItem` 表示 turn 内消息、推理、命令、文件变更、工具、审批等项，可理解为 step，不等于整个 turn。
- 补拉单飞，期间多次提示最多追加一次后续读取。按 `(turnId,itemId)` 合并并保留旧页、游标和事件窗口；并发到达的正文或 canonical item 优先。控制权恢复、缓存关闭或版本变化后丢弃在途结果，失败保留原内容。
- 无控制权摘要不再携带活动 turn 时，移除旧非终态投影。结束后保留文字、停止流式显示，避免旧 running turn 继续驱动活动 UI。

## 来源

2026-09-09 用户对 Provider/Remote 职责的纠正；实现证据为 `provider/hook_observation.rs`、`provider.rs::conversation_active_list`、SDK `explicit_activity_event_sink`、Remote `ConversationMessageSource`。相关约束见 [通道隔离](codex-provider-channel-isolation.md) 与 [消息数据源](remote-conversation-message-sources.md)。

## 验证方式

- Codex Hook 单元测试覆盖：resume 前更新、有执行槽仍投递、重复事件、跨 server 启停、迟到工具不复活、内容时间不遮蔽生命周期、原生摘要不能建立或结束活动成员。
- SDK 测试覆盖：显式活动模式不从原生摘要生成活动事件，活动通知推进事实版本；原有 Provider 模式不变。
- Remote 缓存测试覆盖：空 item 与无控制权双条件、单飞和后续读取、并发更新、控制权恢复、失败保留历史。
- 协议 fixture 和生成新鲜度校验，Host 路由校验，Provider 启停集成测试覆盖跨模块回归。

### 验证记录（2026-09-09，Windows）

- `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --lib`：62 项通过。
- `cargo test --manifest-path sdk/rust/Cargo.toml -p codepet-provider-sdk --lib conversation_atoms::tests --target-dir crates/target`：6 项通过。
- `cargo check --manifest-path crates/Cargo.toml -p codepet-provider-codex -p codepet-host --tests`：通过。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --test provider_vertical start_returns_starting_and_stop_discards_pending_probes -- --exact --test-threads=1`：通过，使用隔离的临时 CODEX_HOME。
- 同级 Remote 仓库 `flutter test --no-pub test/conversations/conversation_message_cache_test.dart`：7 项通过。
- 此前同一功能的协议检查 20 项、SDK conversation_state 测试 8 项、Remote conversations/gateway/device_session 测试 195 项通过。Host lib 测试 57 项通过，既有日志轮转测试在 Windows 遭遇拒绝访问（OS error 5），未修改该模块。

按用户确认的范围，订阅之前或中断期间漏掉的 Hook 不做恢复或补偿，活动只按实际收到的 start/stop 更新，也不使用 loaded threads 补造事实。真实 Codex 双进程抢锁与 Hook 投递仍待实机验证。SessionStart 表示生命周期开始，不证明正在生成 token。
