# Codex 桌面 App 会话加载链路与 Code Pet 适配边界

后续实现状态：用户确认并集方案后，列表分页改动与验证记录见 [双源目录分页修复](codex-union-directory-pagination.md)。下文保留当时静态分析与风险评审的证据范围。

## 现象与目标

2026-09-08，用户要求分析桌面 App 如何发现和加载会话，并评估复用相同逻辑。此前已复现原生 thread/list 过滤空 preview，以及 legacy thread/read 元数据时间回退。详细实验见 [会话发现缺口](codex-agent-created-thread-discovery.md)。本文记录实际资源中的调用链，不代表已实现修复。

## 证据范围

运行包 `OpenAI.Codex_26.901.6511.0_x64__2p2nqsd0c76g0`，App Server 0.153.4。解析 app.asar，静态阅读 main-DpnWwRdP.js、src-VqXTPopo.js、app-initial-f87238153a19.js。仅提取资源、只读查询 DB，没有执行提取的 JS、调用 resume 或发送消息。资源副本位于 `C:/Users/17633/.codex/tmp/codepet-app-inspect/`；以下压缩函数名只在这个构建中有效，不是公开 API。

## 还原的加载链路

### 1. 发现候选与目录同步

main 的 `w5` 将本地 App Server 包装为 catalog source。listPage 调用 thread/list，参数为 archived=false、modelProviders=[]、parentThreadId=null、sortKey=updated_at、sortDirection=desc、useStateDbOnly=true，透传 cursor/limit。readItem 调用 thread/read(includeTurns=false)。没有发现此适配器绕过空 preview 过滤的参数。

目录协调器有全量/增量扫描、watermark、分页检查点、失败退避和前台/后台调度；同步结果写入 `.codex/sqlite/codex-dev.db` 的 local_thread_catalog。创建通知 thread/started 直接 observeThread；turn/completed 使源失效。已知单项可以独立刷新，归档/删除有移除路径。因此目录可包含未出现在最新列表页的任务，但不是永久完整性保证。

同等条件的只读 readEntries 查询（host_id + thread_id，missing_candidate=0）找到 R1–R5 全部五项。R2/R3/R4 的目录标题仍为 cwd，更新时间仍为创建时刻，不能认为 App 目录比原生 DB 所有字段都更准确。

### 2. 项目、排序与展示

webview `BTi` 组合项目根目录、目录前缀、threadWorkspaceRootHints 和显式 threadProjectAssignments，生成 includeThreadIds/excludeThreadIds。显式归属会覆盖目录猜测；projectless 另有排除规则。

main `readPage` 在目录中分页：limit 1–100，取 limit+1 判断后续；游标校验 sortKey。updated_at 模式实际主排序字段是 source_recency_at，次排序 source_created_at，最后 thread_id；created_at 模式使用 source_created_at/source_updated_at/thread_id。不能将这个 UI “最近”语义直接替换 Code Pet 已冻结的 active/unread/updatedAt 规则。

标题函数 `kb` 使用 name → 处理后的 preview → cwd → ID。App 可见来源集合允许 agent_created_thread。它有 conversations、threadsById、threadSummaries 等内存状态，展示不等于单次原生响应。

### 3. 补读元数据

`hydrateThreads → s9t → readHydrationThread` 对已知 ID 补读；s9t 并发最多 2。按 hydrationGeneration 和请求开始时状态判断响应是否仍有效；同一响应避免重复应用。若 threadsById 已有更新时间更晚的对象，hydrateThreads 丢弃旧 read 结果；同时间但状态已变化也有保护。读失败记录并跳过，不能将失败自动解释为删除。

`readHydrationThread` 在只要摘要时请求 includeTurns=false；需要完整历史、非 paginated 且无限定轮数时可使用 includeTurns=true；其他分支按 5 turns 分页，并检测重复 cursor。不是所有历史入口都采用同一种方式。

### 4. 打开任务与 resume

`maybeResumeConversation → Rln`：每个会话复用 in-flight resume；有取消/当前 attempt 检查，并检查窗口是否能取得流所有权。

无内存对象时先读 catalog entry（失败再考虑 summary/knownCatalogEntry），用目录标题、时间和 cwd 构建占位状态。随后启动 thread/read(includeTurns=false)，并加载 workspace/权限输入。关键合并点保留已有标题，并用 Math.max 合并更新时间，避免旧 read 覆盖已知较新时间。另有普通 metadata hydration 路径直接覆盖时间，因此不能声称所有入口都防回退。

再调用 thread/resume。tail hydration 路径按条件使用 excludeTurns；非 paginated 分支可传 initialTurnsPage={limit:5,itemsView:full,sortDirection:desc}。paginated 分支由 resume 提供的历史边界继续加载。resume 涉及 writer，本次只做静态分析，未实际调用验证。

### 5. 历史分页与实时合并

`Q4t` 的 paginated 路径先 thread/turns/list(itemsView=notLoaded)，再 thread/items/list 读取对应 turn 的 items；记录每个 turn 的游标及完整性，按 ID 去重，检查重复 cursor，保留旧用户输入信息。该路径还有单次 item 工作量限制；不是整个 App 所有加载路径的统一限额。

`M4t` 按 older/newer boundary 加载，检查 boundary 是否仍有效，拒绝未推进 cursor，再按 turn/item 稳定身份合并。`loadConversationTurns` 复用在途加载。hydrateThreads 在历史响应到达时保留请求期间新增的实时轮次及已有终态，避免旧历史覆盖实时进展。完整加载和上翻分页是独立入口。

## 对 Code Pet 的结论

App 没有摆脱 App Server；它在其上增加目录、已知 ID、事件观察和合并防护。仅照抄 thread/list 参数仍会遗漏空 preview 会话；仅直接读 App catalog 也可能取得陈旧字段。

建议在 Codex Provider 内实现统一候选目录：App Server 列表 + state DB 补充缺失 ID + 已知实时会话；复用项目映射；按 ID 取正式对象，但保留有证据的列表/DB 时间，缺省标题不覆盖已有标题。DB 不伪造运行状态，未读沿用当前 shared store。按“完整候选→过滤/排序→分页”组织结果，不拼接两套上游页。

无需修改现有 Provider wire 来完成发现与摘要合并。若要求复刻 App 的逐 item 部分加载，则必须单独评估消息完整性表达和客户端控制：当前 [分页规约](../60-rules/provider-item-text-and-pagination.md) 明确要求 Provider 透传 caller 的 full turns cursor/limit。不能在现有响应里偷偷返回部分 items，也不能把 App 的 5-turn 策略移入 Provider。当前请求为链路分析，未改该规约或实现。

## 并集方案风险评审（2026-09-08，待实现）

本节评审用户选定的“App Server 列表 + DB 遗漏 ID + 实时事件 → Provider 目录 → 按 ID 读对象 → 字段合并 → 过滤、排序、分页”，不表示实现已完成。目标是主动拉取的持久化任务覆盖；事件是补充观察，不要求 App 在线才能发现任务。

### 已有证据与影响模块

- 实测原生列表 36 项、DB 41 项，缺失 5 项可以按 ID 读取；证明并集能修复这类发现缺口，不能证明两源都未知的任务也能恢复。
- legacy 样本中 list/DB 时间较新，read 时间回退到创建时间。若 DB 严格只保留 ID、最终以 read 覆盖所有字段，时间范围查询仍错误。需要保存用于校正的时间证据；DB 不成为完整对象或运行状态来源。
- 项目映射来自桌面全局状态；样本 DB project_id 和 read.projectId 都为空。必须在项目过滤前复用现有映射，不能用 DB project_id 提前排除候选。
- `sdk/rust/codepet-provider-sdk/src/conversation_atoms.rs` 已有查询快照、事实 epoch 和发布门；目录应接入这些边界，避免建立另一套未读/状态权威。当前普通列表及搜索与查询快照入口不同，需要统一覆盖，否则不同入口仍可能漏项。
- `sdk/rust/codepet-provider-sdk/src/conversation_query.rs` 的 SnapshotPager 缓存完整行，TTL 为 120 秒，最多保留 64 个快照、超限淘汰最老者。冻结 ID 后逐页重新过滤不等价于当前快照语义。

### 风险、处理要求与验证路径

| 风险 | 处理要求 | 验证路径 |
| --- | --- | --- |
| 两个第一页取并集不等于全局第一页；扫描期间上游排序改变也可能跨页漏项 | 合并候选集合后分页；区分扫描完成与扫描中。全量基线加重叠扫描/定期复核只能提供最终收敛，不能声称两个源有共同原子快照 | 相同时间边界、跨页更新、旧任务突然活跃，核对完整集合及重复项 |
| 按最终字段过滤之前的 DB 时间/项目条件过窄，永久丢失候选 | DB 条件必须是最终条件的候选超集；时间字段统一单位和语义，项目先装饰再过滤。不得用 App recency 替换现有 updatedAt | legacy 时间回退、worktree 显式归属、项目清空、时间边界 |
| 旧 read 覆盖新列表/事件；空值覆盖标题；无条件 max 固化错误的未来时间 | 保存字段来源、扫描代次和请求期间的新事实；区别未知与明确清空。为已验证的 legacy 回退设定明确校正规则，不把所有字段都交给最后到达者 | read 延迟期间发生改名/项目移动/完成；未来时间修正；同时间不同事实 |
| 并集只增不减，旧扫描使已归档/删除任务复活 | 区分未列出、临时读失败、明确归档/删除；用代次/删除记录拒绝旧响应，删除策略不能依据原生列表缺席 | 扫描中归档、旧事件晚到、读取超时、恢复归档 |
| 只冻结 ID，补读后排序或过滤字段发生变化，翻页跳项 | 冻结本次查询的成员、顺序及用于筛选/排序的摘要；新事件更新下一代目录。复用完整行快照，或另行设计等价语义 | 翻页时项目移动、改名、活动更新；快照过期和容量淘汰 |
| DB 不可用或单个对象不可读，静默跳过会把部分结果伪装成完整结果 | 有界重试、区分权威删除与故障；现有协议无逐项失败/完整性标记，不能静默降级并宣称完整。失败请求或使用已完成快照必须有明确策略 | 锁/权限故障、App Server 断开、一个损坏任务、首次冷启动无缓存 |
| 每次查询都全量双源扫描并逐 ID 读取，N+1 成本及快照内存增长 | 每个 Provider 实例共享目录、去重在途读取、限制并发；列表已有正式对象可按失效状态决定是否补读。短 DB 事务先结束再做 RPC，不跨网络持有读事务 | 大目录、慢 RPC、多读者、并发失效、120 秒过期/64 快照压力 |
| DB 与 Server 指向不同 home/主机；来源过滤不一致 | 以实例/主机/home 加原生 ID 确定作用域；明确归档及允许的任务来源。空 preview 不等于应隐藏，但 DB 全部行也不自动等于产品可见任务 | 两个实例同 ID、切换 home、内部来源、归档过滤 |
| 字段修复被当成新活动，触发假未读或跨通道污染 | 沿用 shared store 的活动/未读语义，区分元数据校正和实际新事实；事件只补充当前可观察实例 | 时间校正不增未读、Remote 与 Pet 通道隔离、离线后主动重扫 |

### 可行性边界

并集适合作为发现基础，但应合并“带来源的观察”，不能只维护永久增长的 ID 集合。重拉能提高最终覆盖，不能修复两个源同时缺数据，也不能保证并发写入时每次扫描完全一致。DB 读事务与 App Server RPC 不共享快照，扫描失败与真实空集合必须区分。

在不增加部分结果或逐项错误语义的前提下，可在 Provider 内完成目录、字段校正与现有快照分页，无需改变 wire。必须明确：DB 除 ID 外允许保留必要的时间/归档证据；已有列表对象参与字段合并；同一查询固定摘要；不完整扫描不替换完整基线。若产品要求“一项损坏也返回其余项，并准确告知缺了什么”，需要另评估协议表达，不能仅靠内部实现承诺。

本次新增内容为源码与既有实验的设计评审；未实施目录、未执行上述故障或规模测试。

### Cursor 硬约束（用户明确要求）

区分三种游标：App Server 扫描游标、DB 扫描断点、客户端查询游标。前两者只用于目录采集，不能直接作为合并结果的客户端 cursor，也不能把两个游标拼接后声称是全局分页。

客户端首次查询在可用的已完成目录版本上完成字段合并、项目装饰、过滤和稳定排序，冻结摘要结果；后续 cursor 指向这个快照的位置。cursor 应绑定版本、Provider 实例/作用域、snapshotId、规范化查询条件（包括项目、时间范围、排序及读者作用域），并复用现有防篡改校验。不能把实时变化的全局 fact epoch 当作每页必须匹配当前状态的条件，否则持续活动会使用户永远翻不到后面；需要检查现有 epoch/cache 失效边界是否允许已发行快照继续读取。

排序必须含唯一且确定的兜底键，例如时间降序后按原生 ID 升序；DB keyset 的比较方向必须与 ORDER BY 一致。后续页不能重新过滤、重排或静默跳过读取失败项。优先复用完整摘要快照，而非每页重新读取后决定成员。重试同一个 cursor 应返回相同摘要页；页面之间的新任务、新活动及项目移动在新查询中体现。

过期、容量淘汰、进程重启、作用域/条件不匹配或旧版本 cursor，必须按现有错误机制明确失败，不能退回第一页或换快照继续，避免重复/遗漏。快照固定页大小还是允许修改 limit 应沿用并验证现有协议；不能把 limit 当过滤条件随意改变绑定规则。保留窗口需结合实际浏览时长评估，当前 120 秒和 64 快照只是现状，不是已证明足够的配置。

验收必须覆盖：同时间多页、重复请求同一 cursor、翻页中插入/更新/归档/项目移动、上游重复 cursor、DB 同时间断点、过期/淘汰/重启、查询条件变更、多实例及持续实时事件。断言在未过期快照内遍历得到固定集合且不重不漏。内部扫描仍可能受并发写入影响；客户端快照稳定不能冒充上游发现绝对完整。

## 后续验证清单

- 发现：空 preview、Agent 创建、普通任务、归档、首次冷启动和仅事件出现的任务必须全部覆盖；DB 不可用不能报完整空集合。
- 项目：显式归属与 cwd 冲突、worktree、无项目各自验证。
- 合并：legacy read 旧时间、空标题、加载期间新事件、旧 generation、失败与删除区分。
- 分页：固定候选、时间边界、同时间 ID 排序、重复游标、页面期间新活动。
- UI：目录占位→正文首屏→上翻历史，保持焦点、滚动位置和实时终态。
- 已完成的是静态链路分析、App catalog 只读查找以及此前真实 App Server/DB 对照；尚未验证运行中 App 每个具体界面选择了哪个 feature flag 分支，也未完整还原全部 owner/follower 与 durable 云路径。未编译、安装或运行 UI 测试。
