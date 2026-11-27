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

Codex 的项目会话查询使用独立的双游标分页，按来源各自分配额度。Provider 对每批统一排序，分页以后按来源继续读取。项目归属、无缓冲续页和手机显示顺序以[项目身份规约](remote-project-identity.md)为准。以下并集补读规则适用于其余列表模式。

Provider 保留两源进度、未消费候选、已返回 ID 和已访问页。以当前页排序边界判断是否补读；同秒时间精度和同时间戳的 ID 排序需要越过边界才能确认。工作预算耗尽必须显式报不完整，不能返回假末页。

无 cursor 开始新查询；cursor 绑定 route、generation、filter、readerScope，过期和篡改显式失败。只承诺已访问页可重放，未访问历史不预先物化。Native cursor 与 DB keyset 分开保存。DB 查询只读，RPC 期间不持有 DB 事务；仅缺少正式摘要的候选执行 metadata read。

App Server 原生不支持 updatedAfter 时，按 updated_at 降序读取并在下界停止，不能添加无效字段假装下推。DB 使用相同时间/项目语义，保留 native 项目优先、legacy 映射以及未映射项目不属于 standalone 的规则。IDs 查询直接读指定 ID，不启动目录枚举。

IDs 查询的成本不能只按 RPC 数量估算：原生 `thread/read` 对无法解析的 ID 可能遍历历史文件目录。批量补读旧活动/未读索引时，需要共享有界的存在性/路径查找，避免每个缺失 ID 重扫目录；不能把 DB 未收录直接当作不存在，以免丢失刚创建或仅 rollout 可见的会话。诊断必须保留历史目录布局或明确说明缺失；只有 SQLite 的快照会低估落空 ID 查找成本。

Metadata 并发上限按实例共享，而非每次查询分别扩大。取消调用方时，已发出的阻塞 native 请求仍占用名额直到退出；只能合并同 generation/事实版本的在途读取，不凭短期失败建立永久缺失缓存。同一 recent 构建因建立阅读基线而重试时可复用已核实对象；来源变化必须使复用失效，不能被自己的失效提示反复触发昂贵补读。整个构建含等待和重试应有总预算；超时是显式错误，不能伪装成完整页或关闭共享原生进程。当前实现及仍未完成的按排序键补页见 [有界补读修复](../40-runbooks/codex-recent-bounded-metadata-repair.md)。

运行中由 Hook 维护；未读由自有存储维护。Host 收集完整轻量活动/未读成员；声明 orderedIds 的实例按活动、去除活动的未读、日期历史三组有序续页，默认合计 20 条。未声明能力或超过候选上限时保留完整聚合，不能任意截断。建立新的已读摘要基线不能使正在扩展的页面自行失效；实际内容或未读成员变化仍需失效。

有序 IDs 的轻量键不能猜测：当前 Codex 只在已核对的运行时与显式 paginated DB 时间上启用；legacy、未知语义和未落库项先原生核实。Hook 时间必须在排序前投影。首屏正式对象时间不匹配时完整核实；后续未访问页无法保持顺序时明确失效，已访问页仍可重放。正式对象仍来自原生接口，不由 DB 列拼装。见 [实现与回归](../40-runbooks/recent-ordered-attention-pagination.md)。

Remote 的 list 响应和用户创建决定普通列表成员。摘要事件只更新已加载行；翻页期间缓存少量事件，待该页抵达后补上较新内容。

## 来源

[按页补查调研](../40-runbooks/codex-page-repair-feasibility.md)、[按需分页修复](../40-runbooks/codex-demand-paged-directory.md)。原全量快照方案见[历史记录](../40-runbooks/codex-union-directory-pagination.md)，已被本规则取代。

## 验证方式

Codex `union_directory`/`directory::tests` 覆盖 1,000 + 1,000 候选首屏有界、并集剩余项、过滤、重放、篡改、WAL 和旧 schema；保留 0.151/0.152 兼容测试。Host 最近测试覆盖时间窗口按需续页、优先级、scope/fence、失效；Remote 测试覆盖 300 条目录外事件不扩容和翻页竞争。真实 DB 用 EXPLAIN 验证范围索引，安装后观察首屏与空闲事件流。

2026-11-17 实时栈和原生对照见 [recent 超时排查](../40-runbooks/codex-recent-timeout-20261116.md)：5040 个合成历史文件名使不存在 ID 的 metadata 读取中位数从约 0.18ms 增至 36.85ms；新用例应检查落空 ID 的查找次数随批次而非“IDs × 历史目录大小”增长，并覆盖文件布局、未落库和 I/O 失败。

排序证据失配并回退到原生核实时，保留本次构建中已经读到的正式对象与精确落空结果，只补读尚未核实的候选。跨查询仍须重新读取。未读游标应从已固定的读者快照续页，首屏用另一个 SQLite 连接持有写事务验证可读取已提交状态。参见[单 Provider 超时排查](../40-runbooks/codex-recent-timeout-20261116.md#2026-11-19-的-codex-单-provider-超时)。

给 scoped 列表设置工作预算时，要逐层核对 Provider 原生工作、Host 到 Provider 的传输、整个 recent 和手机四处期限。用真实子进程将响应延迟到普通 RPC 期限之后，验证完整结果能在外层预算内返回，普通请求仍按原期限结束。参见[原生压力回放](../40-runbooks/codex-recent-timeout-20261116.md#真实-app-server-压力回放)。
