# Task 与 TaskRun 对象定义

## 背景与目标

任务表示持续追踪的目标；任务执行表示围绕该目标的一次具体处理。一个对话可以处理多个任务，一次执行也可以关联多个对话。2026-11-24 按用户确认的定义实现 schema v3，保留旧账本的执行与验收历史，减少 Agent 的维护和界面常驻内容。

## 当前定义

Task 保存 `id/title/description/kind/projectId/createdBy/createdAt/runs`，后台另存 schemaVersion、revision、updatedAt。title 是一句话目标，description 是背景或现象；不再保存另一份 objective。kind 只有 bugfix、feature，在界面作为小标记。项目名称属于读取时补充的投影，项目 ID 由规范化 Git common directory 映射；同仓库 worktree 共用项目。

TaskRun 保存 `id/description/status/validationPlan/agentWorkspace/agentConversation/timeline/createdBy/createdAt`，后台维护 updatedAt 与结束记录。description 记录本次处理原因、确认结论和方案；修改它不会更新 Task 背景。runs 按创建顺序保存，最后一项是当前执行，Task 状态从它取得。

- validationPlan：items 为当前清单，records 为实际验收，revision/revisions 保存清单修订与历史快照。检查项有稳定 ID、前提、步骤、预期、适用本地/发布范围及是否必验。验收记录绑定清单版本、阶段、实际被测目标、环境、执行者、逐项结果和证据，并记录当时工作区快照。自测和人工验收不能混为通过；不同目标或环境不能拼凑验收结果。
- agentWorkspace：path、branch、head、upstream、changeStatus、changedFiles、relatedCommits、observedAt、error。changeStatus 分别记录是否有未提交改动及待推送提交，允许未知。changedFiles 保存路径、变更类型、是否暂存、改名前路径；冲突不标为已暂存。Git 使用工作台的有界只读命令与 porcelain v2 NUL 解析器，不 fetch。推送状态基于本地上游；修改文件属于共享工作区，relatedCommits 仅接受明确关联的 SHA，不自动认领 HEAD 或全部分支提交。
- agentConversation：每项记录 agent、conversationId、verified。只有明确的 codex 来源读取本地 Codex 索引核实项目，其他来源保持待核实；没有 ID 可以为空。Header 不会自动提供对话 ID，不扫描完整历史、不猜造 ID。当前只有已核实 Codex 引用提供跳转。
- timeline：服务记录时间、来源、类型、摘要，可附发布引用与工作区快照。执行摘要演变后，早先的处理原因仍可从时间线检索。Agent 不需要提交整条时间线。

## 执行与状态

主状态仅 in_progress、ready_for_delivery、published、closed。创建 Task 同时建立首次执行；在同次请求携带初始说明和清单即可。继续同次处理只更新当前执行；已发布或已结束后需要重新处理时，创建新 TaskRun。新执行默认进行中，可直接携带新计划；省略计划则复用旧检查项并生成新 ID，验收结果清空。旧执行只读。

首次切到 published 必须附实际发布版本或记录，提交不代表发布。验收出现失败时回到进行中，旧证据及发布引用继续保留。closed 只能通过 App 人工操作；人工可注明原因结束，也可在必验项有相应人工证据后确认验收通过。Task 的标题、背景和标记可以在 App 单独修订，没有暴露 update-task MCP 工具。

## MCP 与来源

只有 create-task、create-task-run、update-task-run、find-tasks 四个工具。三个写入工具返回 taskId、runId、任务 revision、清单版本和检查项 ID 等紧凑回执；更新只提交变化字段。通常开始实施创建一次、实现告一段落更新一次；发布及重新处理再按事实补记。不轮询，不在每条消息后维护。

HTTP 请求携带 X-CodePet-Agent，默认配置为 codex。Rust 从每条请求的上下文取得来源，创建时固化 createdBy，后续更新者写进 timeline，不能改掉创建者。来源是客户端声明的标记；Bearer 认证与 agent/human 权限判断独立，Header 不能授予人工验收权限。保留的 stdio 入口使用 CODEPET_AGENT，缺省 codex，仍只暴露四工具。

## 存储与迁移

活动账本在 data/tasks/v3/projects/<projectId>/tasks/<taskId>.json。聚合事务沿用 Store 的文件锁、原子 JSON 写入、事务恢复和幂等回执。expectedRevision 是整个 Task 聚合的版本；冲突保留人工草稿。重试先读取已提交回执，再访问工作区，所以路径被清理后仍可重试成功。查询默认返回紧凑摘要；详情只返回当前执行和当前清单，历史须显式请求，文件清单在 MCP 回复中最多显示 20 项并标记截断。

首次读取在一个事务中导入 v2，旧文件不修改。保留 Task/执行/检查项 ID、修订、计划与验证记录、旧 Git 快照、对话、时间线和发布引用；旧 objective 合并到说明。bug/investigation 映射 bugfix，feature 保持。旧 Agent 没有具体来源时记 unknown，不猜为 codex；旧观测缺文件清单时明确提示，之后的当前执行写入会重新观测。旧发布引用没有独立时间戳，迁移事件记录迁移时刻并注明原始发生时间未知。旧 v1 对话抽取不在迁移范围内。

## 涉及模块与验证

- tasks/model.rs、mod.rs、operations.rs：实际 v3 对象、写入职责与原子事务；records_tests.rs 验证 Task/Run 隔离、重试、并发和迁移。legacy/legacy_model 只用于旧数据读取、既有验收规则与迁移回归样本，生产入口不再调用旧写入 API。
- tasks/workspace.rs 与 workbench.rs：共用只读 Git 和 NUL 解析，验证改名、暂存、未跟踪、换行文件名及冲突。
- task-mcp 与 HTTP 适配：验证四工具、官方 SDK、来源按请求隔离、缺失/无效 Header、不可冒充人工、重启及端口边界。
- Tauri task_ledger 与 TaskLedger.svelte：复用同一 Ledger 的人工入口，分别编辑任务和执行。默认只展示任务说明、当前执行、验收清单；长说明可展开，工作区、对话、验收历史和时间线归入折叠的执行记录。

具体命令与实测证据见 [MCP 接入与验收](../../40-runbooks/codepet-task-mcp.md)。原生入口测试核对 HTTP 写入与 GUI 读取同一 revision，覆盖人工验收、结束、新执行、历史只读和冲突草稿。迁移测试比较 v2 原始字节，避免“保留历史”只停留在界面。

## 已知边界

不持有工作区完整源码快照，不保证历史内容可精确重建；本地上游不等于实时远端。仓库移动、旧进程在迁移后继续写 v2、其他 Agent 对话索引适配均不自动合并。升级需安装 backend API 28 的 App 并替换 MCP 配置；不依赖仅更新网页资源。安装版人工验收与长对话中主动登记的遗漏率仍需实际使用确认。
