# Codex PM 创建任务的列表发现缺口

## 现象

2026-09-08，用户报告 PM 任务 `01a07cbe-33e3-7610-8148-e62cf3715107` 创建的任务在手机无法加载，后续确认 Code Pet Remote 和官方手机端都找不到。尚无手机请求日志，不能把列表漏显示与点击后历史加载失败视为同一问题。

## 复现路径

Windows 本机 Codex 0.153.4。启动安装版 `codex.exe app-server` 的独立只读探针，initialize 使用 clientInfo.name=code-pet、experimentalApi=true。只调用 thread/list、thread/read 和 thread/turns/list，结束关闭探针进程；未调用 resume、start 或发送消息。

## 证据

PM rollout 的 McpToolCall 记录确认使用 create_thread 创建 project/worktree 任务。只读查询 `.codex/state_5.sqlite`，以下五项均 archived=0、source=vscode、thread_source=agent_created_thread、history_mode=paginated，rollout 文件存在。PM 本身 thread_source=user。

| 任务 | ID | 最新一轮读取 item 数 |
| --- | --- | --- |
| R1 | 01a07d18-9c2e-7253-b79a-f9f3a9218885 | 13 |
| R2 | 01a07d19-445c-77d0-a823-32c508bcf8fb | 349 |
| R3 | 01a07d19-82ef-7e52-a159-beb4b9f0da2f | 3 |
| R4 | 01a07d1c-cc0a-7863-b8b3-cadf1fb92fc2 | 2 |
| R5 | 01a07d4b-8501-7b31-8d1f-9058bf285abd | 8 |

- thread/list(limit=100, sortKey=updated_at, sortDirection=desc, useStateDbOnly=true) 返回 36 项，不含五个目标。
- 显式设置 schema 中全部十种 sourceKinds，仍返回 36 项、nextCursor=null，不含 R1。按 R1 精确 cwd 查询为 0 项、nextCursor=null。
- 五项 thread/read(includeTurns=false) 均成功，状态 notLoaded；thread/turns/list(limit=1, itemsView=full, sortDirection=desc) 均成功。只验证最新一轮，未宣称完整历史全部可读。
- 桌面 list_threads 返回的普通任务列表也未包含这五项。
- 当前 Code Pet 安装版日志只见启动记录，未检索到五个目标 ID；不能由此推断手机没有尝试过读取。

## 初步影响范围

- Codex 原生任务枚举：缺口已在独立原生接口复现，发生在 Provider 和手机渲染之前。
- `crates/providers/codepet-provider-codex/src/protocol.rs::thread_list_params`：当前传递原生列表参数，依赖上游返回候选；未实现补充发现这些任务。
- `crates/providers/codepet-provider-codex/src/client.rs`：metadata read 和 full turns 分页路径在探针中可读；不能因列表遗漏认定历史损坏。
- Remote 列表及详情：只有手机应用与错误信息确认后才能判断实际受影响路径。

## 当前假设

### 桌面 App 资源与独立目录（2026-09-08）

读取运行中的 MSIX 包 `OpenAI.Codex_26.901.6511.0_x64__2p2nqsd0c76g0/app/resources/app.asar`，解析 ASAR 索引后仅提取目标 JS 到 `.codex/tmp/codepet-app-inspect/`。没有修改包或加载执行其代码。

- `.vite/build/src-VqXTPopo.js` / webview app-initial 中，可见来源集合明确允许 agent_created_thread；create_thread 的实现传入此标记。该标记不是 legacy 历史模式。
- `.vite/build/main-DpnWwRdP.js` 的 `w5` catalog adapter 使用 thread/list(useStateDbOnly=true, sortKey=updated_at) 同步目录，按 ID 刷新则用 thread/read(includeTurns=false)，还订阅原生通知。coordinator 在 thread/started 时 observeThread，在 turn/completed 时失效刷新。
- 桌面自己的 SQLite 位于本机 `.codex/sqlite/codex-dev.db`，`local_thread_catalog` 独立保存 display_title、source_updated_at、source_recency_at、thread_source 等。标题选择函数 kb 依次使用 name、处理后的 preview、cwd、thread ID；不会因为 preview 空而必然无法创建目录项。
- 实际目录中 R2/R3/R4 均存在，missing_candidate=0，但 display_title 是各自 worktree 路径，source_updated_at 仍为创建时刻。这证明目录不等于原生列表，也不保证字段始终新鲜，不能声称 App 已恢复这三个缺失标题。
- 旧任务 019f1ec2 的目录保留标题“修复 Pages 部署失败”和 source_updated_at=1783395091。随后同一个独立 App Server 的对照请求确认：thread/list 返回 updatedAt=1783395091，thread/read(includeTurns=false) 返回 updatedAt=1782927458=createdAt。故此样本是 list/read 字段语义或实现分歧，不是 App Server 所有接口都拿不到正确时间。
- webview app-initial 的部分注册/resume 合并路径使用 Math.max(existing.updatedAt, incoming.updatedAt)；另一个 metadata hydration 路径直接覆盖更新时间。不能概括为 App 所有入口都有防回退保护。App 还维护独立 recencyAt，UI 排序不应简单等同单个 read.updatedAt。

legacy/paginated 是 historyMode 两种历史表示/加载路径。旧样本是 legacy；本次 PM 子任务为 paginated，两种问题不能混为一谈。本机 thread_history_1.sqlite 有 thread_turns/thread_items 和按 rollout offset/ordinal 记录的投影表；前端 paginated 分支使用专门的 turn/item 分页加载。legacy 不表示归档、失效或 state DB 已废弃。没有将公开文档中对 paginated 支持程度的描述推广到当前实测版本。

### 二进制 SQL 与隔离副本实验：空 preview 过滤已复现

分析对象为安装版 `C:/Users/17633/AppData/Local/OpenAI/Codex/bin/8e5b6932251c2c1c/codex.exe`，295408944 bytes。只做静态 mmap 字符串提取，没有修改二进制。文件偏移 `0xe09bf9c` 可见 `AND threads.preview <> ''`，邻近 SQL 片段包含 archived、section、project、source、model_provider、cwd、标题搜索、排序及 cursor 条件；关联源码路径字符串为 `state\\src\\runtime\\threads.rs`。另有 preview 非空的可见列表索引。未找到字面量 agent_created_thread，但字符串未检出本身不是不存在相关处理的证明。

通过 SQLite backup 从只读连接建立隔离 state DB 副本，CODEX_HOME 指向临时目录，调用相同安装版的 thread/list(useStateDbOnly=true, limit=100, sourceKinds=全部类型)。只修改副本中 R2 的 preview，不修改 thread_source 或任何真实数据库字段：

| 副本状态 | 列表数 | R2 可见 |
| --- | --- | --- |
| 原样副本 | 36 | 否 |
| 仅 R2 preview 改为非空 | 37 | 是 |
| R2 preview 恢复空 | 36 | 否 |

这将空 preview 与列表遗漏建立了受控因果证据；R1–R5 全部符合该空值条件，且非空 preview 的另一个 agent_created_thread 可见。因此应以空摘要可见性过滤作为当前解释，不再用 agent_created_thread 标记解释直接过滤。没有为修复而向真实 DB 写入伪摘要。

二进制还包含 SELECT threads.updated_at_ms AS updated_at 等投影，以及按 ID 查询 SQL、状态库 upsert 和 legacy→paginated 单向提升逻辑。能恢复 SQL 字段与片段，但未做机器码控制流反编译，不能称为完整还原函数。

时间线索：此前 17 项稳定不一致样本全部 history_mode=legacy，且原生 thread/read.updatedAt 全部等于 DB created_at。支持旧历史读取路径以创建时间初始化/回退 updatedAt 的推断，但具体分支和触发条件尚未通过控制流或独立字段干预实验确认。不能据此认定 thread/list 也采用该时间。修复设计应分别核对 list/read 的时间权威。

二进制 SHA256、片段偏移保存于 `.codex/tmp/codepet-binary-strings.json`；副本 A/B/A 结果保存于 `.codex/tmp/codepet-binary-list-ab.json`。临时副本路径为 `C:/Users/17633/AppData/Local/Temp/codepet-list-ab-ymht04wm`。原数据库未写，未恢复 writer、发送消息或修改业务实现。

### DB ID → App Server 对象可行性验证

用户要求验证 DB 只筛选 ID、正式对象仍取 App Server。使用正常 Host 环境（移除 CODEX_INTERNAL_ORIGINATOR_OVERRIDE），initialize 返回 code-pet/0.153.4。SQLite mode=ro、timeout=1 秒；短事务取得分页 ID 后关闭 DB，再请求原生对象。没有 resume/start。

| 检查 | 结果 |
| --- | --- |
| DB 未归档 ID | 41 项，查询约 2.47 ms |
| updated_at 降序、ID 升序，每页 7 条 keyset 查询 | 7/7/7/7/7/6，与同一静态集合的完整排序一致，无重复遗漏 |
| 原生 thread/list | 36 项，约 20.77 ms，遗漏恰为 R1–R5 |
| 4 路并发 thread/read(includeTurns=false) | 41/41 成功、ID 一致，总计约 69.68 ms；单请求最大 16.82 ms、响应最大 2478 bytes |
| 最近 14 天成员 | DB 与原生对象均为 23 项，本次成员一致 |
| 项目归属 | 41 项 DB project_id 和原生对象 projectId 均空；现有 global-state 项目映射能将 R1–R3 解析到 Host 原生项目、R4–R5 解析到 Remote 原生项目 |
| 目标标题 | R1/R5 原生 name 非空，R2/R3/R4 的 name 与 preview 均空；现有 mapper 将回退显示 ID |
| 更新时间 | 二次读仍有 17 个旧任务 DB updated_at 与原生 updatedAt 不同，原生较早，最大差 467633 秒；不是简单秒/毫秒单位问题 |

重要修正：DB 共 6 项 agent_created_thread，其中另一个任务在原生列表可见。遗漏的五项 preview 全空，不能将 agent_created_thread 本身断言为充分过滤条件。查询可见性实现仍未确认。

结论：读取链路可行，无需 Provider wire 变更，但不是可直接替换全部列表语义的完成证明。项目筛选必须复用现有归属映射，不能仅 SQL project_id；标题缺失仍需明确 fallback；日期查询若以原生对象为准，不能盲信 DB 时间预筛选的完整性，也不能把 DB 页次直接当作原生对象全局排序。当前样本没有最近 14 天跨界，不保证任意截止时间。生产分页应冻结候选及排序并复核变化，不在网络 RPC 期间持有 SQLite 事务；运行状态继续沿用 owned-server 观察，独立探针 notLoaded 不代表全局空闲。

本次数字仅为 41 项、本机暖机环境的单次探针，不代表大集合或手机端延迟。初始两次脚本因语法错误在执行前退出，修正后的探针及时间/归属复核成功。未编译、安装、修改 Provider 实现或执行真机验收。仅保存元数据与计数：`.codex/tmp/codepet-db-id-probe-20260908.json`、`codepet-db-id-probe-details-20260908.json`，不保存历史正文。

后续数据来源核对：项目映射来自 `.codex-global-state.json` 的 `thread-project-assignments`（thread → 桌面 project ID）和 `app-server-project-id-by-legacy-project-id-by-host`（当前 host 的桌面 project ID → 原生 project ID），与现有 Provider 的 parse/decorate 路径一致。R2/R3/R4 在 state DB 的 title、name、preview 均为空，原生对象 name/preview 也为空；不能归因于仅 DB 缺失。

更新时间反证：差距最大的旧任务 `019f1ec2-004e-7c02-8504-28110ca8a181`，DB updated_at=1783395091，与 rollout 最后一条记录 2026-07-07T03:31:31.542Z 及文件 mtime 对齐；原生 read updatedAt=1782927458（2026-07-01T17:37:38Z）反而更早。R2/R3/R4 的 DB updated_at 也与 rollout 最后记录秒级对齐。故现有证据不能解释为 state DB 停止更新。另发现 thread_history_1.sqlite 包含 thread_turns/thread_items/thread_history_projection_state/thread_realtime_items，说明存储有多个组成部分，但本次未证明这些表在具体原生 read 路径中的优先级，不能据此宣布 state_5.sqlite 被替代。

### 实验参数实测

后续使用同一安装版独立 App Server，initialize 返回 Codex Desktop/0.153.4，明确启用 experimentalApi=true。thread/list 共用 limit=100、useStateDbOnly=true、全部十种 sourceKinds、updated_at 降序，仅改变以下过滤参数：

| 参数 | 结果 |
| --- | --- |
| 不加关系过滤 | 36 项，nextCursor=null，不含 R1–R5 |
| parentThreadId=PM ID | 0 项，nextCursor=null |
| ancestorThreadId=PM ID | 0 项，nextCursor=null |
| 两个参数分别传入不存在的 UUID | 均为 0 项 |
| 同时传 parentThreadId 和 ancestorThreadId | -32600，明确提示两者互斥 |
| parentThreadId=123 | -32600，明确提示需要字符串 |

这证明本机运行版支持并校验上述实验参数，没有静默忽略；此前导出的 ThreadListParams.json 未列出它们，不能把导出 schema 的缺项当作运行时不支持的证据。实验关系过滤仍无法发现五个目标。尚未确认它们是否没有原生父子关系，或另有可见性过滤。探针未恢复 writer 或发送消息，结束关闭独立子进程。

agent_created_thread 的上游枚举可见性是高相关线索；尚无上游实现或受控修改证明该字段是唯一过滤条件。sourceKinds 全集无法恢复 R1，因此不应简单增加 subAgent 过滤项作为修复。未修改原生数据库进行试验。

## 已排除方向

五项没有归档，文件没有缺失，原生按 ID 的 metadata 和最新一轮历史读取均可用。worktree 查询仍为空，单纯调整项目目录归组不能解释全部证据。

## 下一步排查与验证

确认手机应用及“列表没有”还是“点击后失败”；取得对应错误和请求方法。若为列表问题，核实原生是否提供 agent-created 发现开关，再设计 Provider 兼容；若为详情问题，按实际 ID/trace 检查 resume 和历史分页，不能用本次一页探针代替完整加载验证。任何兼容方案应同时检查普通任务、PM 任务、归档、项目范围和游标完整性。

## 未知项

手机现场、完整历史、上游过滤实现及引入版本未确认。本次仅只读诊断和文档新增，未修改业务实现、Codex 数据库或现有任务状态；未编译、运行仓库测试或安装应用。
