# Codex 漏项修补恢复双源按需分页

## 现象

2026-09-09 用户发现手机聊天数量达到 236+，并追问为何启动时扫描全部已有会话。用户重新明确原方案：App Server 按 filter 查询，Codex DB 用相同语义补漏，对外仍只有一个 list；活动状态由 Hook 推送，未读由自有存储维护。

## 证据

- 修改前 `provider.rs::collect_directory_summaries` 先循环耗尽 native list，再 `read_evidence` 读取 DB 全目录，补读遗漏及 loaded/observed ID，最终固定整个摘要快照。
- `provider/conversation_observer.rs` 每 5 秒重复上述收集，启动时把历史摘要作为 delta 发布；`atomic_facts_ready` 决定活动能力是否可见。
- Host `gateway/recent.rs::collect_recent` 又会耗尽 14 天范围内的 list 页。
- Remote `DeviceSession::_upsertEventConversation` 会把尚未加载的事件摘要加入普通列表；数量取自该列表，因此会随后台摘要推送增长。
- 本机只读 EXPLAIN：新的毫秒分支使用 `idx_threads_updated_at_ms` 范围 SEARCH；旧 NULL 毫秒分支使用同一索引等值 SEARCH。20 行查询约 0.1 ms。时间相同的 ID 排序、两个已限长分支的合并仍可使用小范围临时排序，不能声称所有临时排序均被消除。

## 根因

把“这一页尽可能完整”扩成了“先枚举所有可发现任务并冻结所有页面”，又用全量协调器的成功作为活动能力前置条件。批量事件入队修复解决了队列重试，但没有修复这个查询边界。

## 引入历史

已有排查确认 `bd6c2e7` 引入 summary reconciliation，`a1f9986` 统一双源全量目录。此前实现记录保留在 [固定快照方案](codex-union-directory-pagination.md)，2026-09-09 用户明确取代该目标。批量事件入队问题独立见 [近期能力与重复刷新](recent-unsupported-and-repeated-refresh.md)。

## 修复方案

- Codex 新增 `provider/directory_list.rs`：两源独立 cursor/keyset、保留未返回并集与已发行 ID、按排序边界补读。20 条批次；同秒边界读到可确定顺序为止。单页最多 128 轮补读，超过预算显式报不完整。
- DB 下推归档、时间、项目、IDs；毫秒与旧 NULL 时间分支分别限长，避免 COALESCE 排序整个现代目录。只读最高版本 state DB，正式摘要仍从 App Server metadata read 获得；保留 legacy 时间修补和桌面项目映射。
- 已访问页保留供重放；随机 token 绑定完整查询、scope 和生命周期，只有已发行 offset 可用。最多 64 个查询、120 秒 TTL。未访问页不承诺历史时点的不可变全库快照。
- 删除 Codex 周期 summary observer。活动能力依据 Hook 就绪状态；IDs 查询可补读 Hook 指定的尚未被 DB 收录的任务，无 loaded-list 扫描。
- Host 保留 active/unread 明确 ID 集合及排序，14 天历史只填补当前页缺口；后续读取沿用 Provider cursor。新增已读摘要基线不使自己的尾页失效，已有摘要变化或新未读成员仍触发失效。
- Remote 的普通列表成员由 list 和用户创建决定；事件只修改已加载行。翻页期间最多缓存 512 条较新摘要，合并该页后应用并清空，避免请求竞争丢失标题更新。

## 涉及模块

- Codex Provider：原生/DB 查询和归属映射发生在这一层，必须统一 filter 与分页边界。
- Provider data/Host 阅读状态：新基线与真实活动变更需区分，否则每次续页都会使自身 cursor 失效。
- Host recent：防止上层重新耗尽 Provider 所有页，维护关注优先级与事件 fence。
- Remote DeviceSession：确保数量和成员来自已加载页，同时保留实时字段更新。

## 验证结果

- Codex 新增 1,000 native + 1,000 DB 候选测试：首屏 20 条只请求一个 native 批次，第二页接续剩余项；重放不再请求来源。过滤、归档、旧 schema、NULL 毫秒、项目映射、IDs 直读、cursor 篡改均有覆盖。
- Codex 单元测试最终 69 项通过；全套 vertical 52 通过、1 忽略；另 1 条既有图片断言在 `provider_vertical.rs:695` 失败，修改前已有，未更改图片行为。真实 native runtime 测试仍默认忽略。
- Host 新增 1,000 条近期历史按需分页测试，以及现有 scope、优先级、fence、失效与普通列表隔离测试。
- Remote 相关 78 项测试通过，静态分析无问题。新增 300 条目录外摘要不扩容，保留翻页中的事件竞争、较旧时间的新标题和用户创建行为。
- Provider data 29 项通过；Host 单元 62 项、manager_gateway 集成 30 项通过，recent 专项 15 项通过。跨 Provider 集成最初遇到一次 Claude 握手超时，复核暴露测试读取真实 Codex DB；增加独立 dataDirectory 后集成通过。Host 的 Provider 源 cursor 过期会转为 recent_cursor_expired，并由 Remote 重建首屏，有注入过期测试。
- 桌面 `npm run tauri build -- --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'` 成功，签名校验后覆盖 `/Applications/Code Pet.app`；Codex Provider 源/安装包 SHA-256 一致，为 `220fc08e0155b7e34fb415b8907b1070c0332f8c41748caca3b3c90b05621ca4`。
- Remote `flutter build apk --debug` 成功，使用已有 release key 重签，通过 `adb install -r` 覆盖 emulator-5556，保留配对数据。
- 实际 UI：Codex 0.153.4 在线，聊天首屏 20+，最近首屏 20+ 可继续滚动到后续任务；项目列表可进入，页面和底部搜索/新建布局正常。09:15:24Z–09:19:15Z 共观察 62 条 Codex 摘要事件，仅涉及 3 个变化中的任务；09:15:48Z 后能力 revision 稳定，无全历史摘要灌入。此观察覆盖查询与滚动，不能声称完全没有合理的活动变化刷新。
- 验证产物：`/Users/wangxin/Documents/Codex/CodePet-builds/demand-page-fix-20260909/`，包含构建/测试日志、安装截图和去掉任务内容的事件统计。

## 回归防线

[双源按需分页规约](../60-rules/codex-union-directory-pagination.md)。测试必须检查读取次数与剩余候选，不只检查首屏返回数量。检查列表装饰不能导致副作用事件回流、既有图片失败不能被误记为本次分页回归。

## 规约候选

已更新目录分页规约与最近架构文档，废止全量冻结目标。

## 未知项与风险

- 大量同秒任务、低选择率的过滤会增加补读；用 128 轮预算显式失败，不冒充末页。旧 DB 缺少时间索引时可能需要 SQLite 内部排序，不等同于读取所有 rollout。
- 未访问页随原生数据变化，继承 native cursor 的一致性边界；已返回 ID 去重，已访问页重放。不是全库历史事务快照。
- 本次真实运行验收为 macOS + Android 模拟器；Windows 原生运行未复测，兼容性主要由已有 fixture 与只读 SQLite 测试覆盖。
