# App Server 漏项的按页补查可行性

## 问题与约束

2026-09-09 用户重新明确：App Server 查询指定范围，Codex DB 只补漏；运行中由 Hook 维护，未读由自有存储维护。不得为了第一页预先扫描所有历史，也不得后台扫描后全量推送。此前双源目录方案中的全量枚举和全量冻结快照是当前实现事实，不再作为后续修正的目标约束。

## 证据

- 历史隔离实验见 [发现缺口](codex-agent-created-thread-discovery.md)：41 个 DB 会话中，App Server 漏掉 5 个空 preview 会话，按 ID 可读；不是必须枚举全部历史才能读取这些对象。
- 本次核对 `/Applications/ChatGPT.app/Contents/Resources/codex` 为 0.153.4。其 experimental ThreadListParams 提供 cursor、limit、updated_at 排序、项目和 cwd 等过滤，没有时间范围字段。只读探针给 thread/list 添加未来 `updatedAfter`，仍收到旧记录，因此不能声称它已经提供直接的时间下界过滤。
- 本机 `state_5.sqlite` 有 `idx_threads_updated_at_ms(updated_at_ms DESC,id DESC)`；全部本机记录的毫秒时间非空。只读查询时间范围内 21 个候选并使用该索引，EXPLAIN 显示时间范围 SEARCH，无临时排序，单次暖查询约 0.42ms；下一页用 `(updated_at_ms,id)` keyset，执行计划仍为范围 SEARCH。
- 未指定索引的相同全局查询，本机优化器选择 archived/cwd 索引并使用临时排序。LIMIT 本身不能证明没有扫描大量记录。
- 独立只读 App Server 探针只请求第一页 20 条，返回 nextCursor；DB 查询该页时间窗口最多 40 个候选，本次实际 20 个且无新增漏项。另取一个空 preview 候选，`thread/read(includeTurns=false)` 成功。该样本不能证明所有过滤与边界都已完整。
- 探针不调用 start/resume，不读取完整消息，不写原生 DB；结束关闭独立 App Server。脚本暂存 `/tmp/codepet-page-repair-probe.py`，生成 schema 在 `/tmp/codepet-list-schema-installed/`。

## 判断与修正方向

已知漏项不要求 Provider 全量枚举历史。可以让 App Server 与 DB 按一致排序边界分别产生有界候选；合并、去重并补读当前页所需对象。只够当前页就停止，剩余候选及两源进度绑定 Provider cursor。不能只在 App Server 当前页 ID 集合内补字段，否则发现不了 DB 独有对象；App Server 空页时也必须允许 DB 独立提供候选。

时间窗口必须覆盖当前合并页的候选范围，处理同时间戳与 ID 兜底排序；未消费候选不能随着上游 cursor 推进被丢弃。必须先确定排序时间的来源和精度，不能补读后用回退的 metadata 时间重排已发行页面。

项目过滤还需要既有桌面归属映射，不能只信 DB project_id。应优先将可表达的条件下推，映射补充按明确 ID 查询；无法下推的稀疏过滤可能需要多取几个候选批次。保证不预先全量枚举，不等于承诺所有筛选都只检查恰好 20 个索引条目；如果要严格工作量上限，需设计预算耗尽后的继续游标或明确错误，不能假称没有下一页。

现有“把全部结果固定成一个快照”的实现需要改为按需分页会话。跨源并发更新下的全局冻结语义不能凭有界第一页查询自动获得；应明确刷新、去重和续页一致性边界，不通过暗中扫描全部历史维持旧实现。

## 涉及模块与验证计划

- Codex directory：参数化范围/keyset/IDs 只读查询，按 schema 验证索引；检查 SQLite 实际计划、旧秒级 schema、WAL、锁错误。禁止为了优化改写 Codex 原生 DB。
- Codex Provider：移除 list 的全量收集前置，保留项目映射及原生兼容；测试空原生页、DB 独有数据、两源重复、同时间戳和读取失败。
- observer/Host recent：不以全量扫描作为活动能力就绪条件，不以耗尽所有页构建第一页。Hook、未读状态源保持各自职责。
- Remote：列表成员与 cursor 按查询隔离，推送不得将全量历史灌入分页窗口。

下一步用大规模隔离样本记录第一页的 SQL 返回量、索引访问成本和 RPC 数，覆盖项目/standalone/时间/IDs、并发新增与更新时间变化。当前只完成源码、schema、执行计划和本机小样本只读验证，未改生产取数逻辑，也未证明全过滤矩阵或跨版本一致性。

## 实施状态

2026-09-09 已按用户要求实施，当前变更和验证见 [按需分页修复](codex-demand-paged-directory.md)。本页保留调研证据；全量快照不是当前目标。
