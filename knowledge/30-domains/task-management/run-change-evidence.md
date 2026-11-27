# 任务执行的文件差异留存

## 背景与目标

2026-11-25 用户指出文件变更只有路径和提交号，希望查看 diff，并确认提交、推送、清理工作区之后仍能回看。此前 AgentWorkspace 只保存状态及文件元数据；TaskRunChanges 没有读取代码差异。提交对象还在本地时可以重读，但工作区路径删除或 Git 对象回收会使旧入口不可用。

目标是以低维护成本保存本次执行的差异证据，沿用全局侧栏。Agent 仍只需在关键节点登记明确关联的提交，finish-task-run 申报 changedFiles；不新增上传代码工具，不把 patch 放进 MCP 查询回包。本期提供只读统一 diff，不实现代码编辑、暂存、提交或远端 Git 服务。

## 数据来源与保存路径

Rust `tasks/diffs.rs` 在账本事务 apply 成功后准备差异，与任务和回执一起由 Store 原子提交。事务失败、修订冲突或幂等重试不会重复写入证据。

- 关联提交：从项目登记的 Git common directory 读取 SHA 相对第一父提交的变更，初始提交相对空树。只读命令禁用外部 diff、textconv 和着色。不猜测分支所有提交属于本次执行。
- 工作区：只读取明确申报的 runFiles 与当前未提交列表的交集，按 HEAD 到工作目录保存；包含暂存与未暂存改动，未跟踪文本按新增文件保存。未申报范围时不抓取共享工作区的全部代码。内容相同的工作区快照使用内容指纹去重。
- `tasks/v4/diffs/<taskId>/<runId>/index.json` 保存版本目录、文件类型、增删行数、观测及保存时间；同目录 `commit-<sha>.json` 和 `workspace-<fingerprint>.json` 保存每个文件的元数据及 `patchRef`。新快照的正文保存在相对于 `tasks/v4/` 的 `documents/<SHA-256>.diff`，按偏移和长度读取单文件并核对摘要。原先内嵌 patch 的快照继续可读。
- 已成功保存的提交差异不会因重读失败被覆盖，工作区恢复干净也不会删除旧快照。旧任务打开变更视图时可从仍存在的关联提交补存，不从现时工作区伪造历史未提交差异。回填不修改 Task revision、清单或验收结论。

保存后，即使工作区乃至整个仓库删除，也可以从 App 数据目录回看。未曾保存且已不存在于 Git 的内容不能恢复；删除 App 数据同样会删除证据。工作区代码状态仍是 observedAt 当时的观测，推送状态仍依赖本地上游，不当成实时远端查询或发布证明。

## 展示与接口

TaskLedger 仍将变更数、提交数、提交/推送状态合并为一行，查看入口使用框架 AppSidePanel。TaskRunChanges 用版本选择、可展开文件列表、A/M/D/R 类型和 +/− 行数显示；有 runFiles 时按执行范围过滤所选提交文件。每个文件展开后 TaskFileDiff 经 `file-diff` IPC 按需读取，显示 Git hunk、原/新行号和增删背景，代码自动换行。切换任务、执行、版本及卸载会隔离过期请求。

新增原生 `change-diffs`（仅返回元数据）和 `file-diff`（仅允许已登记快照内文件）操作，backendApiVersion 从 30 升到 31，必须同时升级原生 App。MCP 继续五工具，参数不变。前端不得用热替换资源假装旧原生具备新能力。

视觉参考：[VS Code 文件列表与差异编辑器](https://code.visualstudio.com/docs/sourcecontrol/overview)、[GitHub 逐文件查看与统一/分栏 diff](https://docs.github.com/en/pull-requests/how-tos/review-pull-requests/reviewing-proposed-changes-in-a-pull-request)。Git 对象可回收的边界依据 [git-gc](https://git-scm.com/docs/git-gc)。本期选择适合窄侧栏的统一 diff，未增加分栏切换。

## 限制、风险与验证

单文件差异上限 256 KiB，每次准备最多保存 8 MiB patch，每个版本最多 300 文件；二进制、非 UTF-8、冲突、超限及读取失败明确显示原因，不显示伪造的零行数或空成功。Store 拒绝符号链接重定向；未跟踪文件复用工作台的路径边界检查，拒绝工作树外链接。Git 和文件操作都由 Rust 统一模块承担，没有散落前端系统命令。

`diffs/tests.rs` 覆盖真实提交、删改和重命名、二进制/超限、未跟踪与外部链接、仅申报文件、清理工作区、删除整个仓库、旧 linked worktree 删除后从 common directory 补存、读取不改变 Task revision、非法版本与路径。`taskDiffs.test.ts` 检查多 hunk、多文件行号与缺少末尾换行。原生 task_ledger_native_qa 覆盖真实生产 IPC 的差异归档与渲染；refinement QA 覆盖 Chromium/WebKit、明暗/窄屏、展开与无横向溢出。引用布局回归覆盖移动账本并删除仓库后读取同一补丁、拒绝越界范围和读取旧内嵌快照。验收和旧迁移测试必须继续通过。

未验证项：Windows 原生运行、超大仓库大量关联提交下的耗时和存储增长尚未实测；当前没有自动清理已保存差异的策略。
