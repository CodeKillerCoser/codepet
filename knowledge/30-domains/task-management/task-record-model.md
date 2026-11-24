# Task 与 TaskRun 对象定义

## 背景与目标

任务表示持续追踪的目标；任务执行表示围绕该目标的一次具体处理。一个对话可以处理多个任务，一次执行也可以关联多个对话。2026-11-24 按用户确认的定义实现 schema v4，保留旧账本的执行与验收历史，减少 Agent 的维护和界面常驻内容。

## 当前定义

Task 保存 `id/title/description/projectId/createdBy/createdAt/runs`，后台另存 schemaVersion、revision、updatedAt。title 是一句话目标，description 是背景或现象；不再保存另一份 objective。Task 不再保存类型，类型属于每一次执行。项目名称属于读取时补充的投影，项目 ID 由规范化 Git common directory 映射；同仓库 worktree 共用项目。

TaskRun 保存 `id/title/kind/goal/solution/description/status/validationPlan/agentWorkspace/agentConversation/timeline/createdBy/createdAt`，后台维护 updatedAt 与结束记录。title 是运行卡片的短标题；kind 为 bugfix/feature；goal 与 solution 分别记录本次目标和确定的方案。description 作为可选补充说明及旧自由文本保留，旧内容不自动猜分为结构化方案。这些执行字段不会改写 Task 的背景。runs 按创建顺序保存，最后一项是当前执行，Task 状态从它取得。

- validationPlan：items 为当前清单，records 为实际验收，revision/revisions 保存清单修订与历史快照。检查项有稳定 ID、前提、步骤、预期、适用本地/发布范围及是否必验。验收记录绑定清单版本、阶段、实际被测目标、环境、执行者、逐项结果和证据，并记录当时工作区快照。界面勾表示对应阶段已验证通过，展开内容标明本地/发布、自测/人工；人工接受仍单独计算，不将自测当人工确认；不同目标或环境不能拼凑验收结果。
- agentWorkspace：path、branch、head、upstream、changeStatus、changedFiles、relatedCommits、commitChanges、runFiles、observedAt、error。changeStatus 分别记录是否有未提交改动及待推送提交，允许未知。changedFiles 保存路径、变更类型、是否暂存、改名前路径；冲突不标为已暂存。Git 使用工作台的有界只读命令与 porcelain v2 NUL 解析器，不 fetch。推送状态基于本地上游；修改文件属于共享工作区，relatedCommits 仅接受明确关联的 SHA，不自动认领 HEAD 或全部分支提交。runFiles 是 Agent 在 finish-task-run 的必填 changedFiles 中明确申报的本次执行文件（仓库相对路径）；空数组明确表示没有代码变更，null 表示旧记录未申报，后续观测保留它，不冒充 Git 观测。commitChanges 按明确关联提交保存不可变文件清单（SHA、files、error），与当前未提交文件分开；提交后工作区干净不会清空提交文件。旧快照没有清单时通过 commit-files IPC 按需只读补查，不写回历史。
- agentConversation：每项记录 agent、conversationId、verified。只有明确的 codex 来源读取本地 Codex 索引核实项目，其他来源保持待核实；所有 MCP 写入要求 agentConversation 必填且非空，create-task 放在 initialRun 内；支持 ID 或 codex://threads/<id> 并规范化存 ID，未知时不得猜造。历史记录及 App 人工创建可以为空，展示为历史未关联；新协议不伪补旧来源。Header 不会自动提供对话 ID，不扫描完整历史、不猜造 ID。当前只有已核实 Codex 引用提供跳转。
- timeline：服务记录时间、来源、类型、摘要，可附发布引用与工作区快照。执行摘要演变后，早先的处理原因仍可从时间线检索。Agent 不需要提交整条时间线。

## 执行与状态

主状态仍为 in_progress、ready_for_delivery、published、closed；中文显示进行中、待验收、已发布、已结束。创建 Task 同时建立首次执行；在同次请求携带初始说明和清单即可。继续同次处理只更新当前执行；已发布或已结束后需要重新处理时，创建新 TaskRun。新执行默认进行中，可直接携带新计划；省略计划则复用旧检查项并生成新 ID，验收结果清空。旧执行只读。

finish-task-run 原子保存非空 conclusion、必填对话、必填 changedFiles（无变更也显式 []）、逐项 verification 和工作区；必验本地项在相同计划/目标/环境全部通过才进入待验收，不完整或失败保持进行中，不关闭执行。只有发布项的清单可在明确记录发布待测及原因后完成开发；发布验证失败仍保持进行中。首次切到 published 必须附实际发布版本或记录，提交不代表发布。验收出现失败时回到进行中，旧证据及发布引用继续保留。closed 只能通过 App 人工操作；人工可注明原因结束，也可在必验项有相应人工证据后确认验收通过。Task 的标题和背景可以在 App 单独修订，没有暴露 update-task MCP 工具。

## MCP 与来源

暴露 create-task、create-task-run、update-task-run、finish-task-run、find-tasks 五个工具。四个写入工具返回 taskId、runId、任务 revision、清单版本和检查项 ID 等紧凑回执；更新只提交变化字段。通常开始实施创建一次、实现告一段落更新一次；发布及重新处理再按事实补记。不轮询，不在每条消息后维护。

HTTP 请求携带 X-CodePet-Agent，默认配置为 codex。Rust 从每条请求的上下文取得来源，创建时固化 createdBy，后续更新者写进 timeline，不能改掉创建者。来源是客户端声明的标记；Bearer 认证与 agent/human 权限判断独立，Header 不能授予人工验收权限。保留的 stdio 入口使用 CODEPET_AGENT，缺省 codex，仍暴露相同五工具。

## 存储与迁移

活动账本在 data/tasks/v4/projects/<projectId>/tasks/<taskId>.json。聚合事务沿用 Store 的文件锁、原子 JSON 写入、事务恢复和幂等回执。expectedRevision 是整个 Task 聚合的版本；冲突保留人工草稿。重试先读取已提交回执，再访问工作区，所以路径被清理后仍可重试成功。查询默认返回紧凑摘要；详情只返回当前执行和当前清单，历史须显式请求，执行标题、类型和目标随摘要返回；本次执行文件与未提交文件在 MCP 回复中各最多显示 20 项；提交清单最多 5 个提交、每个 10 个文件，并标记数量和截断，避免挤占对话上下文。完整证据仍保存在账本。

首次读取优先在一个事务中导入 v3；没有 v3 时导入 v2，旧文件不修改。v3 的 Task.kind 复制到各 Run，表示继承旧类型，不能声称当时分别记录过；卡片使用“首次处理 / 第 N 次处理”，goal 继承旧任务标题，solution 留空，description 原样保留。保留 Task/执行/检查项 ID、修订、计划与验证记录、旧 Git 快照、对话、时间线和发布引用；旧 objective 合并到说明。bug/investigation 映射 bugfix，feature 保持。旧 Agent 没有具体来源时记 unknown，不猜为 codex；旧观测缺文件清单时明确提示，之后的当前执行写入会重新观测。旧发布引用没有独立时间戳，迁移事件记录迁移时刻并注明原始发生时间未知。旧 v1 对话抽取不在迁移范围内。

## 涉及模块与验证

- tasks/model.rs、mod.rs、operations.rs：实际 v3 对象、写入职责与原子事务；records_tests.rs 验证 Task/Run 隔离、重试、并发和迁移。legacy/legacy_model 只用于旧数据读取、既有验收规则与迁移回归样本，生产入口不再调用旧写入 API。
- tasks/workspace.rs 与 workbench.rs：共用只读 Git 和 NUL 解析，验证改名、暂存、未跟踪、换行文件名及冲突。
- task-mcp 与 HTTP 适配：验证五工具、官方 SDK、来源按请求隔离、缺失/无效 Header、不可冒充人工、重启及端口边界。
- Tauri task_ledger 与 TaskLedger.svelte：复用同一 Ledger 的人工入口，分别编辑任务和执行。任务头部为标题、描述、项目/创建者/时间，分割线位于基本信息下方。低对比运行卡片承载执行标题/类型/状态/创建者/时间，详情分目标与方案、工作区、验收和活动四区；对话链接放在对应的执行卡片内，作为选中执行之外的独立按钮，不嵌套在 tab 按钮里，点击对话不切换执行。验收整行切换展开，空心圆表示待验收，通过后为独立勾，不显示多余箭头或右侧重复来源。单项记录只追加该 criterionId 的结果，复用相同版本/环境的其他有效项；来源和目标证据在展开的历史记录中保留。移除更新执行、结束执行入口，人工关闭的后台权限规则不变；新执行入口在更多菜单。

具体命令与实测证据见 [MCP 接入与验收](../../40-runbooks/codepet-task-mcp.md)。原生入口测试核对 HTTP 写入与 GUI 读取同一 revision，覆盖人工验收、结束、新执行、历史只读和冲突草稿。迁移测试比较 v2 原始字节，避免“保留历史”只停留在界面。

## 已知边界

不持有工作区完整源码快照，不保证历史内容可精确重建；本地上游不等于实时远端。仓库移动、旧进程在迁移后继续写 v2、其他 Agent 对话索引适配均不自动合并。升级需安装 backend API 30 的 App 并替换 MCP 配置；不依赖仅更新网页资源。安装版人工验收与长对话中主动登记的遗漏率仍需实际使用确认。

## Agent 接入提示词

提示词由 Rust 保存在 data/tasks/mcp-settings/instructions.json，使用共享 Store 的锁与原子写入，带独立 revision。默认引用 MCP instructions.md；自定义保存原文，恢复默认保存空覆盖值，以便后续默认规则更新。主窗口 setup / save-instructions IPC 读写，五个 MCP 工具不能改提示词。HTTP 与 stdio 的 initialize 从同一配置读取；已经建立的客户端会话需重新连接，不宣称实时替换模型上下文。配置冲突不覆盖编辑草稿。自定义文案不改变 Rust 的来源、权限、验收和修订约束。

## 2026-11-25 参数与验收纠正

此前对话字段可省略，导致新执行缺少追溯入口；验收表单默认未测，UI 又只对必需发布阶段的人工结果画勾，所以本地自测记录可能仍显示未验收；变更区只读 status，提交后为空。修正涉及 model/operations/validation（强制字段与原子验收、完成状态）、workspace（提交文件读取）、MCP（五工具及简短提示）、TaskLedger/TaskRunDetail（显式选择验收状态、阶段投影、两类文件）。description 继续保留可选处理说明与旧自由文本，不自动拼成方案，完全重复目标/方案时不重复展示。

每项 verification.results 要一起提交 criterionId、result、非空 evidence（实际结论或证据）；缺失或空白整次拒绝，不写状态、不追加部分结果。验收只接受当前 planRevision。显示优先采用已有发布阶段；没有发布阶段记录时，both 项显示本地结果。不同目标/环境仍不拼接；人工接受只看必需阶段的人工证据，与显示勾独立。

回归防线：contract_tests 覆盖缺失/空/非法链接、结论与状态原子提交、完成/失败/未完成/重试、干净仓库仍有提交文件、改名及换行文件名；HTTP/stdio 黑盒核对真实协议，原生 IPC 与 UI 回归核对状态、逐项保存、历史只读、明暗/窄屏/滚动。旧对话不可凭空恢复；旧工作区被清理时提交文件明确报不可用，不显示伪造结果。

## 任务视图布局收敛（2026-11-25）

TaskRunDetail 固定任务标题/背景/基本信息区，编辑入口为带可访问名称的铅笔图标，运行卡片与执行正文独立滚动；TaskLedger 的列表分隔条默认更窄并支持拖动及键盘调宽。通过勾用语义 green success 色，保留本地/发布、自测/人工语义。

工作区的提交与变更合为一行：文件数、Git A/M/D（及 R/U/?）、关联提交数、待提交/待推送/已推送/推送未知和查看入口。taskChanges 按本次 runFiles 优先过滤；无申报时按明确关联提交文件去重，否则显示共享工作区文件。未知类型保留问号，不将申报路径猜成修改；读取不完整时不能显示成“零变更”。提交状态来自既有观测，推送状态仍基于本地上游，不能当成发布状态。

点击查看变更用唯一 AppSidePanelHost 展示 TaskRunChanges，业务页不建私有抽屉。选择执行时按需补查旧提交文件，异步结果用序号隔离，避免任务切换后串数据；任务/执行切换或卸载清理旧面板。全局框架继续承担桌面第三栏、窄屏模态抽屉、主题、关闭与返回焦点；编辑弹窗保持在侧栏之上。

验收清单复用要保留原项的 ID 和定义，新增要求追加新项。此次最初误改了旧任务前两项，已从第 1 版逐字段恢复原三项并追加本轮四项；历次清单与验收记录未删除。当前模型按整个 planRevision 隔离结果，因此历史版本结果不自动变成新版本通过。修改既有项应有明确范围依据，不能为了复用任务而改写原验收含义。

受影响模块：TaskLedger/TaskRunDetail 负责固定头部和动作布局；TaskListResizeHandle 与当前会话记录负责尺寸；taskChanges/TaskChangeCounts 负责真实分类与未知状态；TaskRunChanges 经全局 host 展示。验证覆盖明暗、390/600/820/1280/1440、短窗口、分栏取消/复位、深链动作、图标编辑、模态叠放及关闭焦点、无横向溢出。没有新增后端接口或提高 API 30。
