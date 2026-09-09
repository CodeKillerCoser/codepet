# 最近会话交付与监督计划

> 2026-09-09：Codex 全量 summary reconciliation 已移除，Host 近期历史改为按需续页。当前行为见 [按需分页修复](codex-demand-paged-directory.md)，下文保留原交付历史。


## 授权与基线

2026-09-08：用户授权 PM 撰写设计、拆成独立 Codex 任务、监督完成。每项使用独立 worktree；Host 从 v0、Remote 从 main 开始；GPT-6（gpt-6-astra）、推理 high、正常速度。禁止本机编译、构建和测试执行；只做代码、逻辑、协议检查，测试代码可以补齐。

设计权威：[最近会话方案](../10-architecture/recent-conversation-feed.md)。当前状态：源码实现、独立审查、发现问题修复与本地集成全部完成。Host v0 代码至 `1f6618a`，Remote main 至 `7248092`；未推送，所有编译和运行验证未执行。

## 不可突破的范围

只改最近列表。聊天=list(projectId=null)，项目=list(projectId=xxx)；现有分页、状态、未读语义与阅读进度不变。最近自动分页不等于删除聊天/项目的分页按钮。不得用UI缓存冒充全局recent，不得截断active/unread，不得重置阅读存储。

协议保持 v1，不升级、不新增 v2；在现有 schema/manifest 内增量扩展，通过能力协商保护旧调用。

## 任务分解与依赖

| ID | 独立任务 | 所有权 | 依赖和交付 |
| --- | --- | --- | --- |
| R1 | 协议与 SDK 契约 | canonical protocol/schema/manifest/fixtures、生成器、生成输出；不实现产品逻辑 | 最先冻结准确wire/类型；生成源代码可运行，禁止编译SDK；提交并通知PM |
| R2 | Provider 原子能力与公共状态 | Provider SDK手写状态模块、Codex/Claude/OpenCode adapters、相邻测试 | 可先审计/设计公共模块；R1冻结后接入，缺失迁移机制与PM/R3协商；提交源码与检查结果 |
| R3 | Host 最近分页与事件 | Host应用服务、Gateway facade、manager、recent索引/快照/事件、旧状态接管调用 | 可先编写纯逻辑和案例；R1后接类型，R2后检查事实完整性；提交代码 |
| R4 | Remote 最近列表与原列表隔离 | Remote领域/ports/controller/gateway/home/tests；从R1导出的Dart SDK同步 | 先分离scope和构建controller/UI，按冻结契约接入；触底分页、失效刷新与滚动锚点；提交 |
| R5 | PM集成与独立审查 | PM负责合并次序、边界检查、交叉review；必要时独立审查任务只读 | R1→R2/R3→R4集成；核对所有验收条件和未执行项，不自动构建/推送 |

各任务初始只编辑自己所有权路径。R1若生成DTO导致调用处必须适配，应只给出迁移清单；所有权任务负责修复，避免跨worktree互相覆盖。依赖提交由PM通知后cherry-pick，不能直接修改其他任务工作目录。最后仅报告本任务提交，依赖提交另列。

## 契约冻结门禁

R1必须发布一份可直接给R2/R3/R4使用的准确清单：method/event、请求响应DTO、capability、IDs/时间模式校验、readerScope、revision/eventCursor、cursor错误与生成命令。若无损已读迁移需要协议机制，先与R2/R3/PM讨论，不私自增加Harness/UI专用字段。后续契约变更必须通知所有消费者。

稳定语义：Gateway conversation.recent({providerId,cursor?,limit?}) 返回 conversations/pageInfo/revision/snapshotCursor；Provider active.list/unread.list/markRead 与 list 可选日期/IDs查询；14天与优先级只由Host掌握。

## 验收矩阵（代码/逻辑审查，本轮不执行）

1. 100+ active 时跨多页后才到未读；100+ unread 同理；同一会话active+unread不重复。
2. 旧会话突然active/unread，即使从未出现在Remote列表，也能进首屏。
3. 同组updatedAt降序，等时间稳定ID；未读/active变化不写updatedAt。
4. 14天边界包含、时间推进淘汰、长时间活动不淘汰。
5. IDs补齐无N+1正文读取；日期过滤在分页前；OpenCode创建时间排序不能被误用。
6. readerScope隔离、observedActivityVersion竞态、重启和既有read-state无损保留。
7. 旧cursor/其他scope/其他Provider/generation不可混用；旧异步响应不覆盖新列表。
8. recent成员/游标/计数独立；聊天Standalone请求及项目Project请求不改；历史和分页不丢。
9. 最近没有“加载更多”按钮，自动触底、首屏不足填充、错误重试和刷新锚点都有明确路径。
10. capability缺失和索引错误明确报告，不返回假空/假完整结果。

## 监督方式

PM在本对话维护任务链接、阶段、依赖和提交；读取各任务的阶段结果，对模型配置、目录隔离、代码边界做检查。使用任务状态等待获取有意义进展，不以无变化频繁轮询刷屏。发布后的持续监督由本对话负责；只有完成、失败、依赖需要协调或需要用户决定时通知。

集成前必须确认基线及工作区干净，仅采用本任务已提交改动；不得重置/覆盖用户或其他任务改动。未经明确后续要求，本轮不安装App、不编译、不运行测试，不对远端推送。

## 任务登记

所有任务派发参数均为 `gpt-6-astra` / `high`，要求正常速度、不启用 fast；Host 基于 `v0@160b06f`，Remote 基于 `main@3f20065`，使用各自独立 worktree。

| 任务 | task ID | 工作目录 | 阶段 |
| --- | --- | --- | --- |
| R1 协议 | `01a07d18-9c2e-7253-b79a-f9f3a9218885` | `C:/Users/17633/.codex/worktrees/46e4/codepet`，`codex/recent-conversation-contract` | 契约及 Provider 修复独立复审完成 |
| R2 Provider | `01a07d19-445c-77d0-a823-32c508bcf8fb` | `C:/Users/17633/.codex/worktrees/6ca2/codepet`，`codex/recent-provider-atoms` | 最终 `47d04cd` 已复审并集成为 `ab1ee0c` |
| R3 Host | `01a07d19-82ef-7e52-a159-beb4b9f0da2f` | `C:/Users/17633/.codex/worktrees/7b62/codepet` | 两笔均通过独立源码审查，集成为 `f71a1e8`、`1f6618a` |
| R4 Remote | `01a07d1c-cc0a-7863-b8b3-cadf1fb92fc2` | `C:/Users/17633/.codex/worktrees/9cf0/codepet-remote` | 已审查并集成 main `7248092` |
| R5 独立审查 | `01a07d4b-8501-7b31-8d1f-9058bf285abd` | Remote 独立 worktree，以 git show 读取目标提交 | Remote 两项修复及 Host 两笔提交均通过源码审查 |

2026-09-08 PM 接管决定：R2 的公共 SDK 使用原路径、原 v1 文档，稳定锁文件保护跨进程事务；保留 global latestVersion、callerScope baseline/reads 与全部 fingerprints，首迁移留不可覆盖备份。R3 移除旧内存缓存写入器，向 Provider 注入同一存储路径。两任务直接对齐观察事件的唯一写入边界、Windows 原子替换、损坏文件 fail closed；不增加 bootstrap RPC。该决定是实现方向，尚未通过代码审查。

R1 契约已冻结为 `0d5d9e70d7645ebc554a98322186167f1e2b80e4`，准确清单在其 worktree 的 `knowledge/40-runbooks/recent-conversation-contract.md`。PM 已授权 R2/R3 接入该依赖，并将正式 Dart 导出位置 `C:/Users/17633/.codex/worktrees/46e4/codepet/sdk/rust/target/recent-gateway-dart/` 转交 R4。生成来源为该提交的 canonical protocol，仅执行源码生成与新鲜度检查，未编译、运行测试或推送。主工作区暂未集成实现。

后续阶段记录：R2 产出 `423b772`（共享状态接管）及 `c0d363d`（批量事务、枚举游标认证、依赖锁文件），R3 已将前者取入自己 `codex/recent-host-r3` 分支为 `693f9a5`。PM 源码初审允许第一笔依赖接入，但不代表运行验证。PM 在第二笔发现 no-op 指纹观察仍无条件 persist，可能退化到每 token 全量写盘；已要求 R2 将 dirty 与 activityVersion 分离，只在实际变更时写盘，R3 必须包含修复后才能通过审查。四个实现仍在进行中，主分支尚未集成实现提交。

更新：R2 已提交 no-op 修复 `b904631`、游标验签顺序修复 `4dd5a25`；R3 已取入。R1 通过 `38d1353` 澄清枚举 revision 与事件失效标识独立，Provider 事件必须使视图失效，不能跨体系比较；wire 保持 v1。

R4 已交付 `80f14b8`、`fef0406`（`codex/recent-remote`，工作区干净），正式 SDK digest 为 `sha256:c539baf1106a8ca2c838cb573b645a8c47a3b2efdb9660941457f16e80062d0d`。新增 R5 独立 Remote 源码审查任务正在启动，创建标识 `client-new-thread:9c9b510f-a482-42c4-b161-4cf6e12c516b`，GPT-6/high/正常速度，基于 main 的独立 worktree，只读审查 R4 提交，不运行验证。待拿到正式 task ID 后更新登记。

新的完整性门禁：R2/R3 指出 Claude/Codex 当前无法证明完整发现独立外部进程的活动，不能将 managed 集合广告为全局 active。PM 要求 R1 独立核查原生接口/hooks 的可行支持矩阵，R2 优先完成 OpenCode 完整链路；可以在 Provider 内补全局事件或有界可取消的摘要/活动轮询，但 Host 不轮询，Provider 不加入14天/排序策略。无法保证的能力必须明确 unsupported，不能把所有最近都禁用就声称任务完成。普通聊天/项目继续可用。

PM 范围澄清：上述“完整”按设计原文指 `ProviderInstanceRoute` 对应 backend 实例及原有运行状态语义，不额外要求全机所有独立进程。OpenCode `/api/session/active` 是同 native server 范围，不因此直接判不可用；必须核对摘要 namespace 与状态权威范围一致。Codex/Claude 不能仅因缺少全机 active 就禁用，R1 继续核查同实例完整性和恢复路径。

R5 正式 task ID `01a07d4b-8501-7b31-8d1f-9058bf285abd`，已只读审查 Remote `fef0406`，发现两项 P2 并由 PM 派 R4 修复：尾页持续过期、首屏持续成功时跨自动恢复周期无限重试；删除顶部锚点后追找缺失 ID 可能扫完全量最近。修复要求跨周期预算，以及有序邻居锚点/有界恢复，新增源测试不执行，修复 hash 交 R5 复审。R2 adapter 主提交 `e45d2ae` 已产出，最终能力矩阵仍待同实例范围核查。

R4 通过 `9ef882e` 修复两项 P2，R5 已对该追加提交定向复审，两项可关闭，未发现相关新增 P1/P2。PM 在确认 Remote main 干净、期间新增仅无关 RTC 文档提交 `4417afa` 后，保留该提交并依次集成 R4：`80f14b8→3f69bf0`、`fef0406→70f7de0`、`9ef882e→7248092`。未推送，未编译或运行测试；这仅表示 Remote 源码审查及本地集成完成，不代表 Host/Provider 或整体功能完成。

Host 主 v0 已集成 R1 `0d5d9e7→8a8d0ee`、`38d1353→56da8ce`、原生 scope 审查文档 `4fdd10d→c3e798a`，保留期间其他任务的无关文档提交。原生审查确认三个 Provider 在各自 backend scope 都有可实现路径，R2 已产出 `154dd0b` 恢复 scoped 能力及后台完整摘要核对，SDK dirty 失效补丁 `ad2eb96` 是 Host 必要依赖。

R3 自身 Host 提交 `41c0a7917a9e88b9aa368ef84b2a6336e64a8607` 已完成，工作区干净。只取入该自身提交，父链 R1/R2 依赖应按原作者提交单独集成。R5 已接续独立 Host 源码审查（全局分页、游标/fence、scope/generation、失效及并发）。PM 对接管薄包装/manager 路由初审无阻塞发现，完整复审未结束。

R1 对 Provider `154dd0b` 及前序交叉审查发现两类 P2，已派 R2 修复，未修复前不合入主 v0：扫描旧结果与原生事实事件缺统一原子提交序列，可能用旧 Running 覆盖新 Idle；Claude/OpenCode 采集在代际检查前修改共享缓存，旧扫描跨重启后可污染新实例。要求采集返回独立结果，在共同提交边界内复核 generation/Ready/cancel/epoch 再应用和发布，不接受单次游离 atomic 检查；补源测试但不执行。R1 获授权收到修复 hash 后定向复审。`ad2eb96` dirty 失效接口通过源码复核。

R5 对 Host `41c0a79` 独立源码审查完成，未发现可明确复现的新增 P1/P2；等待 Provider 修复通过后按依赖集成。R2 已提交修复至 `7596d17`，R1 正在定向复审。R2 自身提交按顺序为：`423b772`、`c0d363d`、`b904631`、`4dd5a25`、`e45d2ae`、`ad2eb96`、`154dd0b`、`15aa510`、`b1d94e2`、`e065440`、`7596d17`。其中后四笔分别为 native metadata/cache 源测试、扫描 epoch 与失败前 previous 保留、native 与扫描整批共享提交 gate/Claude 条件安装、OpenCode atomic 扫描不写 runtime cache/旧 scope 回归。不得 cherry-pick R2 整条父链重复取入 R1；R1 协议已在主 v0。

## 最终集成与未验证

- 已完成：设计、v1 协议与 SDK 集成、原生 scope 审查、Remote 实现及两项修复的独立审查/本地集成；Host 实现与查询失败跨 scope 失效补强 `74149f1625c4d5e80933d90c6e2f969bcb636cc3` 的独立审查。
- 全部审查门禁已关闭：R1 对 Provider `47d04cd` 定向复审确认 Claude 同代扫描锁覆盖采集到安装；生产 SDK/stdio/mux 链路整批事件单槽入队，状态去重在成功后提交，ready 通知位于批尾，失败撤销 pending 优先重试。跨重启及事实发布 gate 问题此前已关闭。R5 确认 Remote 两项修复及 Host 两笔提交无新增源码审查阻塞。
- Provider 14 笔自身提交已按序集成：`423b772→96f0070`、`c0d363d→948924a`、`b904631→76de45f`、`4dd5a25→3a4132d`、`e45d2ae→2752d49`、`ad2eb96→3470a74`、`154dd0b→bd6c2e7`、`15aa510→eb22f20`、`b1d94e2→cbe4680`、`e065440→ca89260`、`7596d17→d5a3a77`、`42b3130→86dca59`、`3c46fb8→8994971`、`47d04cd→ab1ee0c`。Host `41c0a79→f71a1e8`、`74149f1→1f6618a`。
- PM 已核对集成后的 Provider/SDK、Host、Remote 源路径与各自最终审查 hash 完全一致（git diff --exit-code），差异格式检查通过；集成时两个工作区均干净，保留其他任务的无关文档提交，未推送。
- 运行边界：活动完整性限于各 Provider backend 实例的既有权威范围；后台摘要核对默认上一轮结束后等待5秒，实际冷启动与大集合成本未测。生产整批队列保留256MB字节预算，超预算显式失败并撤销/重试，不截断候选；自定义 sink 的默认 publish_batch 可以顺序部分成功，不能将生产链路保证自动推广到自定义实现。
- 未验证：所有本机编译、测试、运行时和真机行为（用户要求暂不执行）。
