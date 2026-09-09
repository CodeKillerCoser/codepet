# Codex 双源按需分页边界

## 规则

同一 `conversation.list` filter 分别查询 App Server 和 Codex DB，按 ID 取并集、补齐当前页摘要、排序后返回。首屏不能预先枚举全部历史；不能用启动扫描或定时扫描向客户端灌入历史摘要。2026-09-09 用户明确修正了此前“全量候选超集 + 冻结所有页面”的方案。

## 适用场景

Codex 普通、项目、standalone、updatedAfter、ids 列表，Host 最近页，以及 Remote 列表成员和数量。

## 反例

- 为修补 App Server 空 preview 漏项，先读完 DB 和所有 native list 页。
- 两源各取一页，截取并集后丢弃剩余候选，或直接透传其中一个源的 cursor。
- 用全量 summary poll 的成功与否决定 Hook 活动能力是否就绪。
- 用收到的摘要事件直接扩充普通列表，导致未翻页也从 20+ 增长到 236+。
- 将 SQLite `LIMIT` 当成已走索引的证据；在 ORDER BY 使用 COALESCE 后可能排序整个范围。

## 推荐做法

Provider 保留两源进度、未消费候选、已返回 ID 和已访问页。以当前页排序边界判断是否补读；同秒时间精度和同时间戳的 ID 排序需要越过边界才能确认。工作预算耗尽必须显式报不完整，不能返回假末页。

无 cursor 开始新查询；cursor 绑定 route、generation、filter、readerScope，过期和篡改显式失败。只承诺已访问页可重放，未访问历史不预先物化。Native cursor 与 DB keyset 分开保存。DB 查询只读，RPC 期间不持有 DB 事务；仅缺少正式摘要的候选执行 metadata read。

App Server 原生不支持 updatedAfter 时，按 updated_at 降序读取并在下界停止，不能添加无效字段假装下推。DB 使用相同时间/项目语义，保留 native 项目优先、legacy 映射以及未映射项目不属于 standalone 的规则。IDs 查询直接读指定 ID，不启动目录枚举。

运行中由 Hook 维护；未读由自有存储维护。Host 先聚合活动/未读 ID 所需摘要，时间窗口历史按最终页面缺口续取。建立新的已读摘要基线不能使正在扩展的页面自行失效；实际内容或未读成员变化仍需失效。

Remote 的 list 响应和用户创建决定普通列表成员。摘要事件只更新已加载行；翻页期间缓存少量事件，待该页抵达后补上较新内容。

## 来源

[按页补查调研](../40-runbooks/codex-page-repair-feasibility.md)、[按需分页修复](../40-runbooks/codex-demand-paged-directory.md)。原全量快照方案见[历史记录](../40-runbooks/codex-union-directory-pagination.md)，已被本规则取代。

## 验证方式

Codex `union_directory`/`directory::tests` 覆盖 1,000 + 1,000 候选首屏有界、并集剩余项、过滤、重放、篡改、WAL 和旧 schema；保留 0.151/0.152 兼容测试。Host 最近测试覆盖时间窗口按需续页、优先级、scope/fence、失效；Remote 测试覆盖 300 条目录外事件不扩容和翻页竞争。真实 DB 用 EXPLAIN 验证范围索引，安装后观察首屏与空闲事件流。
