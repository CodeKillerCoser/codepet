# 最近会话交付与监督计划

## 授权与基线

2026-09-08：用户授权 PM 撰写设计、拆成独立 Codex 任务、监督完成。每项使用独立 worktree；Host 从 v0、Remote 从 main 开始；GPT-6（gpt-6-astra）、推理 high、正常速度。禁止本机编译、构建和测试执行；只做代码、逻辑、协议检查，测试代码可以补齐。

设计权威：[最近会话方案](../10-architecture/recent-conversation-feed.md)。当前状态：设计完成，待任务创建和契约冻结。

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

任务创建后由PM填写：task ID、worktree、实际模型/推理、代码提交、阶段、依赖和代码审查结论。

## 已完成与未验证

- 已完成：阅读Host AGENTS与活文档技能；核对Provider/Host/Remote现状；明确分层、范围与分页方案；生成技术设计和本计划。
- 未完成：协议冻结、实现、代码审查、集成。
- 未验证：所有本机编译、测试、运行时和真机行为（用户要求暂不执行）。
