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

主状态仍为 in_progress、ready_for_delivery、published、closed；中文显示开发中、待确认、已发布、已完成。创建 Task 同时建立首次执行。已发布后可以新增 TaskRun；用户标记已完成后则锁定，Agent 不能更新或追加执行。只有用户在 App 明确选择“改为开发中”时，由人工入口追加新的 TaskRun，以改变 Task 的当前状态；旧执行仍保持已完成。新执行默认开发中，省略计划时复用旧检查项并生成新 ID，验收结果清空；旧执行、验收证据、提交与时间线原样保留。

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
- Tauri task_ledger 与 TaskLedger.svelte：复用同一 Ledger 的人工入口，分别编辑任务和执行。任务头部为标题、描述、项目/创建者/时间，分割线位于基本信息下方。主区只展示任务与执行摘要，点击执行在 App 框架侧栏查看目标与方案、工作区、验收和活动；对话链接位于侧栏的目标与方案下方。任务列表支持收起并保留调宽偏好。验收整行切换展开，空心圆表示未验收，通过后为独立勾，不显示多余箭头或右侧重复来源。单项记录只追加该 criterionId 的结果，复用相同版本/环境的其他有效项；不将历史或 Agent 自测变成人工通过。人工完成与恢复操作仅针对当前执行，权限及锁定规则由 Rust 保证。

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

本机交付复核发现一个补充场景：已保存的 commitChanges 有 11 个文件，侧栏提交明细可见，但摘要仍显示未确认。原因是 Svelte legacy reactive 先计算 changes，再通过函数间接同步赋值 commits；异步补查会触发下一轮刷新，掩盖了已缓存快照的问题。改为显式响应式初始化 commits，再计算投影；cached-files 用例覆盖同一已保存快照的主区数量、A/M/D 和全局侧栏一致性，保留原异步补查场景。

## 可读性、验收结构与差异证据（2026-11-25）

Task 状态枚举不变，当前页面将 in_progress 显示为“开发中”、ready_for_delivery 显示为“待确认”。任务正文改为 14px，元信息保留层级；次要文字使用正文与面板混合的较深颜色，明暗主题对比度检查至少 4.5:1，不提高大面积装饰的视觉重量。

验收点仍由 title 表达，展开仅显示验收方式、预期结果、验收结果。既有 precondition 合并显示在方式内，steps 为实际操作；新清单编辑不再单独要求前提字段。结果内并列状态和非空结论，不再把 evidence 拆成“说明”，移除“验收记录”折叠块；后台 records/revisions 原样保留，人工权限及阶段/目标/环境隔离不变。记录结果的动作仍按单项追加，并同时提交状态和结论。原清单不得为本轮反馈改写，新要求追加新项。

变更侧栏新增逐文件 Git 统一 diff 和本地留存，提交元数据补查改走项目 Git common directory，并可回退已保存清单，避免绑定已清理的 worktree 路径。涉及 tasks/diffs（归档）、mod/store（同事务与回填）、task_ledger IPC（按需读取）、TaskRunChanges/TaskFileDiff（文件与行号）、taskDiffs（解析）。API 升为 31，需要原生更新，具体来源、保留范围、容量与验证见 [任务执行的文件差异留存](run-change-evidence.md)。


## 验收结论行简化（2026-11-25）

用户截图框选了结果中的“本地自测通过”和“记录结果”入口。TaskRunDetail 移除这一整行，验收点的勾、失败/未验收标记和通过数仍由结构化状态计算；展开的验收结果仅显示 evidence，无结论显示“暂无验收结论”。TaskLedger 同步移除已无入口的验收表单及回调，不另造页面动作。MCP/原生写入的逐项状态与结论约束、历史记录和人工权限保持不变。

验证：任务 UI QA 检查明暗主题和不同宽度的结论区无状态行/按钮，键盘折叠行为正常；原生 QA 经生产 IPC 写入单项人工验收后验证结论显示及勾汇总，继续检查新执行/历史隔离；保存忙碌和冲突草稿验证移至仍存在的任务编辑弹窗。


## 待确认执行的人工完成（2026-11-25）

当前待确认执行的选中卡片，在底部显示轻量“标记完成”按钮；历史、开发中和已完成执行不显示。按钮独立于切换执行的 tab，避免嵌套按钮；保存期间禁用完成和任务编辑操作。TaskLedger 使用既有 close-run 人工 IPC，携带 taskId/runId/expectedRevision、accepted=false 和“用户标记完成”说明，不新增 MCP 工具或原生接口。后台继续保存 closed 枚举，界面统一显示“已完成”。

人工完成表示用户决定本次执行处理完毕，不自动追加逐项人工通过结果。原验收项、证据和版本保持不变，时间线记录 human 来源；当前筛选为待确认时，成功后切到全部状态以保留所选任务，焦点回到任务标题。失败或修订冲突保留待确认状态，可刷新重试。已完成后必须由用户手动改为开发中，才会追加一次新执行并保留历史；Agent 不能自行恢复。

验证覆盖 Chromium/WebKit 的入口条件、键盘完成、忙碌防重复、失败/冲突重试、筛选与焦点、明暗和窄屏布局。原生隔离账本从尚无验收结果的待确认执行点击实际按钮，检查刷新后 closed/closure 与 human 时间线落盘，验收清单和记录逐字保留。生产账本只验证按钮展示，不代用户完成真实任务。此改动沿用 API31。


## 用户完成锁与人工恢复开发（2026-11-25）

用户进一步确认：手动标记已完成后，不能由 Agent 再次开启，只有用户手动改回其他状态才可继续。发现此前 create-task-run 同时允许 published/closed，Agent 可绕过 update-task-run 对 closed 的拒绝。现由 operations.check_new_run 在访问工作区前及事务内同时验证：已完成只允许 Actor::human() 走人工状态操作，Agent 返回“任务已完成并锁定”；已发布的新一轮行为不变。自定义提示词、来源 Header 或 stdio 环境标记不能授予人工权限。

App 的已完成卡片与更多菜单将动作明确命名为“改为开发中”，没有通用的自动重新开启入口。打开表单默认标题“继续处理”、沿用目标与工作区、清空旧方案，默认沿用上一轮验收定义；用户可以改用可读取的同项目工作区。提交调用已有人工 create-task-run，旧执行保持 closed，新执行为开发中、createdBy=human，时间线写明用户手动改变状态；新执行不继承旧验收结果或提交关联。已完成/已发布筛选在成功后切到全部，保持新执行可见并返回任务标题焦点；取消、失败和修订冲突不解除锁定。

受影响模块：tasks/operations 负责真实权限与事务，TaskLedger/TaskRunDetail 负责明确人工入口和新执行表单，MCP instructions/工具说明负责告知 Agent，既有 managed stdio 安装负责独立二进制更新。webcontent API 升为32，需同时部署原生程序和独立 MCP，不能仅替换前端声明规则已生效。已运行的 stdio 进程保留原版本，重连后使用新安装版本。

验证：领域测试对拒绝后的整个 Task 做 JSON 等值比较，对人工恢复后的旧执行做完整等值比较；真实 HTTP 与 managed stdio（含已建立连接和自定义提示词）验证 Agent 无法新增执行。双引擎 UI 验证人工动作、取消返回焦点、失败保留草稿、终态筛选后可见性和历史只读；原生 IPC 验证旧执行验收、完成记录、工作区与时间线完整保留，新执行 ID/检查项 ID 独立且结果清空。

## 任务概览与执行侧栏（2026-11-25）

多次执行合并到同一任务后，原界面把任务信息、运行导航和整段执行正文同时铺开，导致阅读层级混杂。现将 TaskRunDetail 收敛为任务概览：固定任务头部，下方独立滚动执行摘要；每条显示序号、标题、目标摘要、状态和元信息，按现有执行数组倒序展示，并保留原序号。任务列表可由工具栏收起，隐藏时移除分隔条和键盘焦点入口；展开恢复原宽度，切页后保留本窗口偏好。

TaskRunContent 单独承载执行正文，以 15px 正文、16px 分区标题和较深的主题文字提高可读性。点击执行才通过唯一 AppSidePanelHost 打开，桌面显示全局第三栏，窄屏使用框架抽屉；页面不创建局部侧栏。代码变更在同一面板切换，返回保留验收展开状态、正文位置与查看变更按钮的焦点。关闭侧栏返回原执行按钮，切换任务或离开页面释放侧栏。完成/人工恢复后回到概览；历史执行始终只读。保存失败在当前侧栏内显示，避免窄屏被遮在抽屉后。

相关模块：TaskLedger 负责列表展开状态、面板路由与人工动作；TaskRunDetail 负责信息层级和执行入口；TaskRunContent 负责原四区正文；taskLedger.css 让主题、焦点和状态样式同时作用于主区与框架内的侧栏；codexWorkbenchSession 只保存窗口级列表偏好。本次无数据结构或 Rust/MCP 协议变化，仍使用 API32，既有清单与历史不迁移。

风险与验证：侧栏替换 diff 可能丢失阅读位置，通过同一内容实例、独立视图返回与焦点回归检查；隐藏导航可能留下不可见焦点，通过 inert、移除分隔条、键盘和切页测试；主题和窄屏可读性由 Chromium/WebKit 的 390/600/820/1280/1440 尺寸、15px 正文、4.5:1 次要文字对比度及无横向溢出检查覆盖。人工完成、恢复与原历史不变仍通过生产 IPC 的隔离账本验证，不操作用户已完成的任务数据。

本次验证通过：`node scripts/task_ledger_ui_qa.mjs`、`node scripts/task_ledger_refinement_qa.mjs` 及各自 `CODEPET_QA_ENGINE=webkit` 版本；`node scripts/task_ledger_native_qa.mjs`；四个任务/差异/工作台会话 Vitest 文件共20项；`npm run check:frontend`（0错误、5项既有警告）、`npm run check:source-size`、`npm run build:webcontent`（API32、16资源）。本机安装前端 v26 后，经真实 App 检查任务列表收起/展开、7次执行、详情/变更返回与关闭焦点，已完成状态保持不变。浏览器布局样例不作为新的业务验收证据。
