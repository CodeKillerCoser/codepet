# 跨工作区、跨 Agent 的追加活动时间线

## 背景

2026-11-26 用户要求多个 Agent 能跨 workspace 续办和交互，所有 TaskRun 写操作进入 append-only 活动线。核对主干后，跨项目和工作区限制已在领域层解除，创建者也不构成更新权限；实际缺口是 update 的 note 会替代状态/计划等变更摘要，且历史或已完成 Run 没有可追加评论的入口。

## 目标

- 保留评论、回复和实际业务事件，后续修改不覆盖旧事件。
- 多个 Agent 同时评论不需要争抢业务 revision；业务字段仍检查 expectedRevision。
- 身份来自已鉴权连接，事件保留本次对话引用；旧记录不补造来源。

## 非目标

不唤醒或调度其他 Agent，不自动执行评论中的指令，不引入 hook/runner。读取、失败事务和幂等重试不制造业务活动。召回加工队列仍有独立加工历史，活动线记录权威账本的成功写入，不伪装成全部系统访问日志。

## 现状理解

Task 的 projectId 只用于最初归属和分组。Run 可以换到别的工作区或仓库，对话也不要求 cwd 相同；切仓库不得携带不相关 SHA 和文件申报。任何经过 MCP 鉴权的 Agent 可续办同一当前 Run，创建者保持不变，人工完成锁仍限制开发状态和内容写入。

## 实现路径

新增 comment-task-run：taskId/runId/comment/agentConversation，支持 replyTo 和 operationId。评论在已有文件事务锁内加载最新聚合并追加，不要求 expectedRevision；更新聚合 revision 以便读取方发现变化。重试复用回执和 activityId，不能重复追加。回复只引用同一 Run 内已有活动。历史或已完成 Run 可以评论，但不改变开发字段、工作区、验收、人工锁或其他 Run；回执 runId 指向实际被评论的历史 Run。

create/update/finish/人工结束/完成任务/修订元数据均生成服务端活动。一次更新分别记录内容、工作区及代码证据、计划修订、验证、发布、状态转换和评论；note 不再覆盖其他事件。验证失败引起的自动回退同样记录实际前后状态。计划事件只保存该版清单，避免再次嵌套全部历史。记录原字段的 before/after，评论更正通过新评论表达。

事务提交前对已有 Run 和时间线前缀逐项等值比较，拒绝删除、改写和重排；新事件 ID 不可重复。迁移记录已有 ID 和事件原文保留。数据继续位于 tasks/v4，记录格式升级 schema 6，旧 4/5 只读兼容、下一次成功写入保存 6，旧 writer 会拒绝该格式，避免丢失新增事件字段。前端最低 backend API 为 39，需更新原生 Host/MCP 并重连；仅安装前端不能新增 MCP 工具。

事件保存 actor、actorRole、本次 agentConversation、replyTo 和变更快照，时间和 ID 由服务端生成。独立评论不复制旧 Run 的 Git 快照冒充评论者观测；开发更新仍由 Rust 观测实际工作区。历史未知角色和对话保留未知。人工权限不因 X-CodePet-Agent 声称 human 而授予。

find-task-runs 精确读取返回最近 20 条活动，可通过 activityBefore/After 和 activityLimit（1–50）分页；返回 ID、来源、正文和回复目标，不展开所有 Git/变更快照。完整 before/after 保存在账本并由 App 展示。界面区分事件类型、评论和回复，可展开变更并跳转回复目标；打开详情时每 5 秒只读检查选中 Task 的新 revision，不覆盖正在编辑的草稿，也不重置已有焦点。

召回文档纳入事件来源、评论、回复和业务字段变化；排除重复的工作区观测及验证快照，验证证据仍从 verificationRecords 读取，避免 Git 观测噪声进入语义 hash。未变化的旧活动保持原有文档投影，不因空元数据触发全量重新加工；新评论保留唯一 ID 和作者，同毫秒、同正文的不同 Agent 评论不会被语义输入去重。

## 涉及模块

- tasks/activity/model/mod/operations/find/migration：事件模型、追加校验、事务、评论和分页，保证持久化和回执语义一致。
- task-mcp lib/instructions/recall snapshot：新增第六个工具、实际连接来源和召回输入，继续限制定义和说明体积。
- TaskRunTimeline/taskActivity/TaskLedger：展示原始事件、变更和回复，增量读取共享更新；复用全局侧栏、主题和焦点行为。
- 协议、HTTP/stdio/managed 测试及 QA fixture：验证实际进程和前端渲染；fixture 不能作为正式业务写入证据。

## 风险

- 并发评论覆盖或重复：四线程追加、跨重启幂等、历史 Run 回执及真实 stdio/HTTP 测试。
- 评论遮掉业务事件或修改旧记录：组合更新、自动状态回退、前缀篡改回滚和旧 schema 迁移测试。
- 旧 writer 丢字段：schema 6 与 API 39 兼容边界检查；部署必须同步原生部分。
- 轮询覆盖编辑/焦点或窄屏溢出：只读 revision 递增更新、表单忙碌拦截，浏览器检查实时追加、回复跳转、明暗和窄屏。
- 评论被当作用户指令：MCP 说明明确只作为历史上下文，不构成新授权。

## 测试计划与结果

领域、MCP、HTTP、managed 和 lineage 测试共 108 项通过（3 项既有模型/包环境测试保持忽略）；新增并发、不可变前缀、回复与分页验证。真实 task_mcp_smoke.mjs / task_mcp_http_smoke.mjs 通过，工具仍小于 24KB、说明小于 2KB，HTTP 以 Claude/Cursor 来源追加和回复，回执重放不重复。没有启动两个真实 Coding Agent，也没有写正式用户任务。

前端 270 项测试通过，Svelte 检查 0 错误/5 项既有警告，API 39 构建及 Tauri cargo check 通过。Chromium 检查活动同步、回复焦点、变更展开及窄屏；截图在 artifacts/task-ledger-qa/activity-*.png。前端样例是明确的开发数据，不代表已安装 App GUI 验收。提交前执行 check:source-size 与 git diff --check。

## 知识沉淀

对象文档更新六工具和评论例外；追加约束单独记入开发规约，防止未来 UI 或新 writer 引入活动修改入口。

## 未知项

Windows 安装版与实际 App GUI 尚未人工验收。很长的活动线目前随 Task 聚合保存；存储压缩、保留策略和前端虚拟化尚未建立规模基线，不主动截断历史。
