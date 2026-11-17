# 最近首屏查询链路与读取必要性审计

## 范围与结论

2026-11-17，用户要求先说明“首屏 20 条为何逐个补读 1506 个会话”，核对 Codex 原生列表与数据库已有字段。本页记录修复前的链路及历史约束审计；用户随后授权实施，第一阶段见 [有界补读修复](codex-recent-bounded-metadata-repair.md)，后续可选协议与按页补对象见 [有序高优先级分页](recent-ordered-attention-pagination.md)。未知和 legacy 情况仍有保守核实成本，不将本页全部建议视为已完成。

当前实现把“确定全局优先级排序”与“补全全部候选的完整 Conversation 对象”绑定，造成全量原生补读。排序可能需要考察全部候选的轻量排序键，但并不要求对每个候选调用原生 thread/read。上一阶段只聚焦缺失 ID 的目录扫描，提出共享路径查找；本次确认应优先消除不必要的对象补全，再约束例外回退。

## 当前链路

1. Remote `RecentConversationController.pageSize=20`，调用 Gateway `conversation.recent(providerId, limit=20)`；手机没有要求 1506 条。
2. Host `gateway/recent.rs::collect_recent` 先分页耗尽 active 和 callerScope 的 unread。排序规则是 active 优先、unread 次之、其余 14 天内历史最后，各组按最终 updatedAt 降序。
3. Host 新建空 `summaries`，再计算 active/unread 中“缺失摘要”的 IDs。因为这个 map 刚创建，所有候选都被当作缺失。每 100 个 IDs 调用 Provider list，内部请求 limit=100，不受 Gateway 的 20 限制。
4. Codex `directory_list.rs` 的 IDs 查询初始化 `native_done=true`，不会调用 `thread/list`。这条支持 Hook-only、新建未落库会话的按 ID 路径，也承担了整个最近页的高优先级候选读取。
5. Provider 的 DB helper 只返回 `id / archived / updated time / history_mode`，没有取标题、预览、cwd 等现成元数据；然后把 DB 没找到的请求 IDs 也全部加入候选，逐个 `thread/read(includeTurns=false)`，最后才跳过 thread-not-loaded。
6. Host 等所有候选补读完，写入/装饰阅读状态，再全量重验 active/unread。仅在候选不足时按需取日期尾页。
7. `recent_conversations::aggregate` 全量排序，snapshot 保存所有高优先级结果；`page_with_fence` 最后才按 20 切页。失效刷新会重走构建；没有失效时已有 recent snapshot 可命中缓存。

故障机快照报告中的 1506 次 native read、16 批 IDs、228 条成功摘要、0 次 thread/list 与上述链路一致。1278 次 thread-not-loaded 不能证明全部是用户删除，但能证明大量无法解析的旧索引成员被反复尝试。详见 [超时取证](codex-recent-timeout-20261116.md)。

## 原生列表与两类数据库已有的信息

| 来源 | 实际已有/使用的信息 | 当前浪费或限制 |
| --- | --- | --- |
| Codex `thread/list` | `CodexThread` 含 name、preview、cwd、createdAt、updatedAt、status、projectId 等；请求已使用 useStateDbOnly、更新时间降序和分页 | 普通查询会直接映射列表对象；recent 的 IDs 分支跳过了这条路径 |
| Codex 原生 `state_*.sqlite / threads` | 本机核对存在 name/title、preview/first_user_message、cwd、created_at_ms/updated_at_ms、project_id、rollout_path、model 等列 | 现有 `DbEvidence` 故意只读四类证据，之后仍补读原生。不同 schema/旧字段的语义需逐项映射，不能假设所有版本字段相同 |
| Code Pet `app-state.sqlite` | callerScope 基线、activityVersion、readVersion、title、摘要/事件指纹 | 不保存完整列表摘要和原生 updatedAt；其 updated_at 是本地记录写入时间，不能拿来冒充原生更新时间 |
| Provider `ActivityProjection` | Hook 状态、Hook 接收时间、已观察会话的上次 summary | IDs 查询没有先复用已有 summary。现有 Hook 投影在列表分页后执行，会改变 updatedAt，后续优化需保证排序发生在投影之后 |

`read_native_page` 已经会直接复用原生 list 返回的正式对象；DB 与 native 重复的 ID 在 query.pending 命中时也不会再次 read。问题集中在 IDs 首次查询：native 被跳过、pending 为空、DB 不取完整元数据，所以每个 ID 又进入 read。

### 历史补漏约束：不能把原生 list 当作完整目录

用户提醒此前存在 Codex 桌面与 App Server 的多存储问题。回查 `a1f99867`、`09e2fe8b` 及 [会话发现缺口](codex-agent-created-thread-discovery.md)、[桌面加载链路](codex-desktop-conversation-loading-chain.md)，需要区分以下事实：

- 当时 Codex 桌面另有 `.codex/sqlite/codex-dev.db / local_thread_catalog`，保存 display_title、source_updated_at、source_recency_at 等目录投影，通过列表、按 ID 刷新与事件同步。目录里能有当前原生 list 不返回的会话，但记录也可能陈旧；不能把它当作会话正文或永久完整目录。
- 已受控复现的五个遗漏会话都存在于同一 `.codex/state_5.sqlite`：DB 共 41 项，原生 list 仅 36 项。只修改隔离副本中一个遗漏会话的 preview 为非空，list 从 36 变成 37，恢复空值后再次为 36。因此这次漏项不能归因为“这五项只存在另一个数据库”；已证明的是原生列表过滤与 DB 候选集合不同。
- `.codex/thread_history_1.sqlite` 的 turn/item 投影及 rollout 历史又是另一层；legacy/paginated 不代表 state DB 已废弃，也不意味着为展示列表必须读取完整历史。
- 当前 Provider 发现补漏读的是配置 home 中最高数字版本的 `state_*.sqlite`，并未读取 `codex-dev.db` catalog，也未合并所有旧 state 版本。“所有库取并集”不是现有实现事实或本次优化建议。

后续必须保留“原生 list + 同 home 原生 DB 补漏 + 当前实例已知事件/新会话”的发现完整性，以及时间修补和项目归属映射。首屏优化应把候选发现、排序键与展示字段补全分开；不能删掉 DB 补漏，也不能只补原生第一页已经出现的 ID。是否接入桌面 catalog 需要另有具体缺口证据，不因它存在就增加全量扫描。

### 历史问题、后续修正与本轮回归边界

以下为 2026-11-17 对历史文档、提交和现存测试的交叉核对；历史验收结果不代表本轮已重新通过测试。

| 历史证据与当时的问题 | 本轮必须保留的行为 | 对应验证路径 |
| --- | --- | --- |
| [发现缺口](codex-agent-created-thread-discovery.md)：41 个 DB 会话中 native list 只返回 36 个；空 preview 的五个会话按 ID 仍可读 | DB 与 native 是独立候选源；原生空页不代表 DB 没结果，空 preview、`notLoaded` 状态不等于删除 | 原生列表漏项、完全空页、两源重复、DB-only 对象可读；保留原生对象字段与 ephemeral 过滤 |
| 同文“DB ID → App Server 对象可行性验证”：用户明确要求 DB 筛选 ID、正式对象取 App Server | DB 继续提供候选、时间/归档证据；已有正式 list 对象直接复用，当前页确实缺对象时才 metadata read | 对比 native list 与 DB-only 补读输出；不能把“DB 有同名列”当成可以替换正式对象的证明 |
| 同文旧会话 read.updatedAt 回退到 createdAt，17 项时间不一致 | 保留 legacy 时间修补及同秒毫秒精度；不能全局永久取 max，遮蔽合法时间更正 | `directory::tests` 的时间测试；补充旧时间跨 14 天边界、同秒排序、真实时间更正、Hook 时间参与排序 |
| [自动标题](../60-rules/codex-automatic-conversation-titles.md)：Remote 创建线程不必然获得 name，需要独立异步命名 | 保留原生命名与既有 fallback；查询列表不等待标题推理，不以助手正文覆盖名字，自动标题临时线程不进入列表 | 命名变化与推理失败测试；列表查询不触发新模型请求，缓存摘要收到 name 更新后失效 |
| [桌面加载链路](codex-desktop-conversation-loading-chain.md) 与项目补漏实验：DB/native projectId 可空，桌面显式归属另有映射 | 保留 native 项目、legacy 显式分配和工作树归属规则；未映射项目不能误入 standalone，cwd 不代替项目身份 | `union_directory_applies_desktop_membership_before_project_and_standalone_paging`，项目/standalone 过滤在分页前生效 |
| `a1f99867` 曾全量合并并冻结；[按需分页修复](codex-demand-paged-directory.md) 记录启动扫描、手机 236+ 条历史灌入，后由 `09e2fe8b` 改为按需取页 | 两源按相同查询语义和排序边界推进；保留剩余候选、去重及已访问页重放，不恢复全库扫描/周期全量推送 | `union_directory_reads_only_requested_pages_and_preserves_leftovers` 的 1000+1000 候选；补充稀疏筛选、同时间边界、启动无扫描和普通列表事件不扩容 |
| [最近能力与重复刷新](recent-unsupported-and-repeated-refresh.md)：600 条批次被包装层拆散，256 队列仅接纳前缀；每约 5 秒重试，能力和界面反复刷新 | 保留批量原子接纳、真实变更失效；查询成功和 Hook 能力不以历史全扫描为前置条件 | `usage_sink_preserves_batch_admission_and_retry`，持续事件/能力 revision 观察；不能通过忽略正常事件或伪造就绪掩盖错误 |
| [最近会话语义](../10-architecture/recent-conversation-feed.md)：active → unread → 14 天历史，各组按最终更新时间排序 | 14 天前的 active/unread 仍可见；按完整路由身份去重；阅读状态按认证 readerScope 隔离 | Host `complete_global_recent_pages_preserve_scope_and_ordinary_list`；增加大量高优先级候选首屏成本断言、跨 scope markRead 和完整续页 |
| [Hook 约束](../60-rules/codex-hook-activity-and-content-updates.md)：原生摘要不能替代 Hook 生命周期；新会话可能尚未入库 | 本轮不改变 Hook 事实及内容提示的来源，也不由 DB 缺席、Provider 退出或 native idle 推测结束/删除 | `union_directory_resolves_hook_ids_explicitly_without_enumerating_loaded_history`；Hook-only 可读、迟到事件不复活、暂时 I/O 失败不删会话 |
| 手机现存 recent controller/feed 测试：刷新、重连、删除锚点和过期 cursor 都会影响列表稳定性 | 保留 snapshot fence、旧请求丢弃、续页单飞、刷新后深度/可见锚点恢复；错误不能造成自动重试死循环 | `recent_conversation_controller_test.dart` 与 `recent_conversation_feed_test.dart`：旧尾页拒绝、刷新插入、断线重连、锚点删除、连续 cursor 过期 |
| [消息数据源](../60-rules/remote-conversation-message-sources.md)：列表与详情窗口独立，历史页和实时内容需要合并 | 最近页优化不重建消息源；详情保留最新页、手动旧页及 delta/canonical 合并，切会话按完整路由隔离 | 手机消息缓存/详情测试；人工进入历史消息、向上加载后接收活动更新，确认内容和滚动不丢失 |
| [原生请求错误隔离](codex-request-errors-must-not-stop-server.md)：单条错误/超时曾关闭共享 App Server，影响其他会话 | 最近查询预算耗尽只结束对应请求，不终止共享进程、不自动重发写操作；错误与晚到响应按请求隔离 | 同进程并发请求与事件继续成功、PID 保持；真实 EOF/显式 stop 仍正常清理 |

这些记录有先后取代关系：早期“冻结全部候选”和“周期全量核对”不能作为本轮恢复目标；后续按需分页与 Hook 事实来源规则才是约束。旧 Pet/原始 Hook 跨进程实现已由 [Notice 链路](../10-architecture/notice-publication-and-actions.md) 收敛，沿用其保护的业务行为，不恢复已移除的模块。

**修正此前优化建议**：直接用 DB 的 title/preview/cwd 等拼正式 Conversation，会改变当时用户确认的来源边界，不能作为默认修复。先保留 App Server 正式对象，复用现有 list 返回对象和有明确失效条件的已观察对象，把 metadata 补读限制在当前页确需补漏的候选。若证明必须引入 DB 摘要投影，需要单独论证字段语义、版本差异、状态来源和失效机制，而非作为“删掉冗余读取”的隐含变更。

## 哪些读取必要

- **排序必要**：资源 ID、active/unread 分组、最终 updatedAt、稳定的同时间 ID 次序。为了保持旧活动/未读优先，不能只取 Codex 最新 20 条后在本地重排，也不能任取前 20 个未读 ID。
- **展示必要**：本页 20 条的标题、预览、项目/目录、状态、时间和 readerScope 阅读状态。优先复用正式 list 对象和仍有效的原生摘要；DB 保持候选与已验证的证据职责。模型和权限等可选详情不能无条件迫使所有未展示会话补全。
- **状态必要**：实时活动仍来自 Hook，阅读进度仍来自 Code Pet callerScope 权威。Codex DB 元数据不能代替它们。
- **例外补读必要**：刚创建尚未落库、仅 rollout 可见、已加载内存会话，或列表/DB 的所需字段确实缺失。需有界核实，不能把 DB 缺席或暂时 I/O 错误直接当作删除。

## 可以消除或缩小的读取

| 当前行为 | 应收敛的边界 |
| --- | --- |
| 所有高优先级候选都要重新 native read | 批量取得候选/排序证据，复用正式 list 对象与有效 summary；只为当前页缺口补读正式对象 |
| 1278 个不可解析 ID 各走一次磁盘查找 | 限制落空回退工作量，核对候选与有效缓存；保留未落库会话，区分权威缺失与临时不可用。原生内部共享路径查找尚未实现，不能当作已有接口 |
| 全候选完整对象先补完、最后才切 20 | 先用轻量键确定成员及前 20 条，再补展示字段；未显示部分延后到翻页 |
| 整批摘要在 Host 被逐条装饰 | 按 Provider 批量事务，避免重复读取整个阅读状态 |
| 变化后整轮昂贵读重复三遍 | 用事件维护/失效轻量索引，明确整次预算；不能返回伪成功的部分结果 |

### 排序接口的实际缺口

当前 active/unread wire entries 仅有 ID、状态或 readState/version，没有可直接用于 recent 全局排序的 updatedAt。`ConversationAtoms::active` 甚至从带时间的内部 row 生成 entry 时丢掉该时间。这是 Host 目前必须再找排序信息的原因，但不是逐个 native read 的必要理由。

Host 的批量状态装饰和已有正式摘要复用可以先评估。要做到首屏只为当前页补展示对象，需要保留/查询可靠的最终排序键，或由 Provider 返回有序的轻量候选游标；当前协议是否够用尚待实现设计，不能先承诺无需改动。不能只把内部 limit=100 改成 20：当前按 native 时间分页后才覆盖 Hook 时间，且 active/unread 混排会隐藏旧活动项，必须先对齐排序语义。若缺少可靠的轻量排序证据，不能跳过该候选后假称全局前 20 已正确；若前列候选不可解析，也允许有界补读后续候选填页，并不承诺恰好 20 次读取。

## 测试边界与后续验证

附近测试 `recent_time_window_is_fetched_on_demand_and_replayed_without_reads` 覆盖的是 active/unread 为空时的日期尾页；`complete_global_recent_pages_preserve_scope_and_ordinary_list` 验证大量 active 的最终排序和分页，没有约束首屏 metadata 读取次数。Codex `union_directory_resolves_hook_ids_explicitly_without_enumerating_loaded_history` 保护单个 Hook-only ID，并不要求所有 DB 已有字段的 ID 仍原生补读。

后续改动需增加：上千个 active/unread 候选的首屏原生读取次数、已有正式对象时零冗余 read、DB-only 补漏仍获取正式对象、未落库/仅文件可见仍可读、Hook 时间投影前后排序、旧未读资源不漏、独立 readerScope 和完整翻页。性能用例必须包含真实形态的 sessions/archived_sessions 目录以及落空 ID，不只复制 SQLite；同时约束整次查询预算、可取消性及其他 Provider/控制 RPC 的响应。当前只做历史、源码和测试审计，未执行编译测试；验证结论不等同于改造已完成。

## 源码入口

- Host：`crates/codepet-host/src/gateway/recent.rs`、`recent_conversations.rs`、`conversation_state.rs`。
- Provider：`crates/providers/codepet-provider-codex/src/provider/directory_list.rs`、`directory.rs`、`protocol.rs`、`mapper.rs`、`provider/hook_observation.rs`。
- 公共状态：`crates/providers/codepet-provider-data/src/conversation_atoms.rs`、`conversation_state.rs`。
- 手机：`codepet-remote/lib/application/conversations/recent_conversation_controller.dart`、`gateway/gateway_client.dart`。
