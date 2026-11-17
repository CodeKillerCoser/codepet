# 最近高优先级会话的有序候选分页

## 背景与证据

第一阶段 [有界补读修复](codex-recent-bounded-metadata-repair.md) 缩短了串行和重试等待，但仍先补齐全部活动/未读正式对象。本轮目标是对可靠排序键先确定页成员，再获取当前页对象；历史 [补漏、项目和体验约束](recent-first-page-query-audit.md) 持续有效。

上游 Codex 0.166.0 的 [read_thread](https://github.com/openai/codex/blob/rust-v0.166.0/codex-rs/thread-store/src/local/read_thread.rs) 显示 paginated metadata 使用 SQLite 时间，legacy 可以使用 rollout 视图。不能把两类时间统一视为可靠 DB 排序键。另在本机 0.167.0-alpha.4.1 上用原生 SQLite 隔离副本检查 20 个 paginated metadata：补齐同秒毫秒精度后，20/20 与 DB 时间一致。探针未 start/resume/prompt，只导出计数，原生 DB 不写入；样本不能证明任意未来运行时或损坏记录均满足该关系。

## 协议与代码生成

唯一源为 `protocol/v2/provider/conversation/messages.schema.json`，新增可选 `ConversationListQueryCapabilities.orderedIds` 和 IDs 查询的 `orderByUpdatedAt`。未声明能力不使用新语义；原有模式仍最多 100 个 IDs、按 ID 升序。有序模式最多 10000 个 IDs，按最终 updatedAt 降序、ID 升序，过滤后分页；对象缺失与 I/O 失败仍严格区分。

新增可选能力不改变业务 version/minVersion；不是把未知语义发给旧插件。Rust/TS SDK、聚合 Schema 及公开 SDK 从该 IDL 生成。Gateway 方法和返回 DTO 无变化，手机与桌面继续消费原有最近分页协议。公共 ConversationAtoms 实现完整集合排序后分页，Claude/OpenCode 声明能力；Codex 使用下面的按需对象补全路径。

## 实现及边界

- Host `gateway/recent.rs`：完整收集轻量活动/未读成员，保存活动、未读、日期三组进度，页预算合计默认 20。各组每页核对成员、limit、单调排序及游标循环；活动覆盖状态，未读保持 readerScope。超过单组上限或未声明能力时走原有完整聚合，不截断历史。
- Host `recent_conversations.rs`：快照保留高优先级成员；续页中的旧活动/未读不受 14 天到期规则误删。已访问页不重复读来源，来源 cursor 失效转成 `recent_cursor_expired`。构建内因自身基线重试可复用已核实首屏和来源进度，来源变化仍清空复用结果。
- Codex `provider/ordered_ids.rs`：已核对的 0.166/0.167 运行时、显式 paginated DB 记录且时间有效时，只读取候选排序证据；Hook 时间在排序前参与。legacy、未知运行时/schema、未落库 ID 原生核实后排序；完整对象仍通过共享 8 路 metadata reader 获取，不用 DB 拼标题/正文/状态。保留精确 thread-not-loaded 区分、时间修补、项目映射和 ephemeral 过滤。
- 每个来源 view 绑定 generation、整个查询、readerScope、TTL；最多保留 64 个 view。只成功后提交对象和游标进度。首屏正式时间与轻量键不一致则完整原生核实；已发行页后发现不一致则明确使游标失效。事实 epoch 变化不会继续扩展旧 view，已访问页保留重放语义。
- 诊断增加 `provider.conversation.orderedIds` 的候选数、当前页逻辑 metadata 补读候选数、返回数、offset 和耗时；Host `gateway.conversation.recent` 记录总耗时、返回数和错误码。不记录候选 ID 清单、readerScope、标题或正文。metadataCandidates 是逻辑补读候选数，重叠请求仍可能共享实际原生 RPC，不能将它解释为精确物理请求计数。

## 风险与验证

现代路径测试：1000 个 paginated 候选，首屏仅 20 次 metadata read，下一页累计 40 次，重放不增；毫秒精度、排序与原生字段保持。未访问页时间变化导致 cursor 失效，新首屏能看到新顺序；legacy 的合法原生时间更正不被 DB max 覆盖，DB 缺席的事件会话仍补出，精确落空可省略。

Host 测试：1000 个旧活动首屏仅请求 20 个对象，完整翻页后仍包含第 1001 个旧未读和近期历史；readerScope cursor 隔离、页重放不读源、过期恢复和原有未声明能力路径均保留。公共原子测试检查大集合有序分页，以及默认模式继续拒绝超出 100 个 IDs。

运行时/存储语义未知时保守核实，因此 **大量 legacy 或不可解析旧 ID 仍可能需要很多原生读取**；本轮没有建立永久负缓存或额外扫描历史目录来掩盖这一成本。持续修改会使 cursor 失效，不能承诺任意负载下固定读取 20 个对象。故障机部署、真实网络端到端和长期负载仍需后续验收，不能用合成样本替代。

## 验证结果（2026-11-17）

- `cargo check --manifest-path crates/Cargo.toml -p codepet-host -p codepet-provider-codex -p codepet-provider-claude -p codepet-provider-opencode --tests` 通过；既存 dead-code/unused-import warnings 保留。
- Host `cargo test --manifest-path crates/Cargo.toml -p codepet-host --lib --test manager_gateway -- --test-threads=1`：109 单元、57 集成通过；随后新增的“不向未声明能力的 Provider 发送有序请求”用例与 recent 全部 9 项定向复验通过（共覆盖 58 个 manager_gateway 用例）。
- Codex `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --lib --test provider_vertical -- --test-threads=1`：85 单元、70 纵向通过，两个既存环境依赖用例忽略；游标重放边界调整后再次运行 `ordered_ids` 两项通过。
- Claude/OpenCode/provider-data `--lib -- --test-threads=1`：分别 7/26/20 项通过。首次 OpenCode 本机 HTTP fixture 受代理环境干扰，清除测试子进程代理、设置 loopback NO_PROXY 后全部通过；结束了首次挂起的测试子进程，没有修改系统代理。
- `npm run protocol:check`：41 项通过，生成物新鲜；`npm run providers:test`：6 项通过，公开 SDK 和插件 staging 边界保持。
- 手机 `flutter test --no-pub --concurrency=1 test/application/recent_conversation_controller_test.dart test/home/recent_conversation_feed_test.dart test/conversations/conversation_message_cache_test.dart`：30 项通过，使用 loopback NO_PROXY，手机工作区没有源码或生成物改动。
- `git diff --check`、相关 knowledge 相对链接校验通过。原生时间对照产物位于 `/Users/wangxin/Documents/CodePet-builds/recent-root-cause-20261117/paginated-rank-probe.py` 与 `paginated-rank-probe.json`。

本轮没有安装或重启用户 App，未在原故障机部署新包；上述数量与时间键证据不能代替真实网络上的端到端验收。
