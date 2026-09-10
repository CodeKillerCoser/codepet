# 任务抽取运行时、接口与用户数据目录

## 背景

首期任务谱系已有对话树、任务图和正文抽取，但模型配置、Skill、工作目录及抽取状态尚未形成应用能力。2026-09-10 的实现将这几部分接入 CodePet 桌面端，逻辑全部使用 Rust，Svelte 只负责配置和呈现。

## 目标

支持任务查询、异步触发、可替换 Skill、摘要实例绑定、持久化配置、会话脏状态和有界定时抽取。沿用 [输入规约](../60-rules/task-lineage-extraction-input.md)：仅最近滚动 48 小时的用户消息与 assistant 正文，工具调用、工具结果、reasoning、未知或未来时间不进入模型。

## 非目标

不修改原始 Codex 记录，不自动验收任务，不安装 Python 运行时。第一版支持 Claude CLI harness；其他 harness 需要实现统一 `TaskExtractor`，不能仅在界面添加一个不可执行的选项。当前扩展是 Tauri 本地接口，尚未加入 Remote Gateway 和手机 SDK。

## 现状理解

`App.svelte` 挂载任务页面；`TaskLineage` 提供对话/任务导航、图/泳道、原文证据和人工验收。新增 `TaskExtractionSettings` 管理摘要实例、模型、Skill、补充提示词、预算、超时、扫描与静默间隔。原有继续对话仍走 Gateway，发送前检查运行/审批状态。

## 实现路径

### 用户目录和数据结构

根目录使用 `configured_app_data_dir(AppSettings)`，尊重已有应用数据目录配置。Windows 默认是 `%LOCALAPPDATA%/code-pet`；也可由用户配置为其 `.codepair`，不另建一套脱离应用设置的根目录。

```text
<data>/
  skills/extract-tasks/SKILL.md
  skills/reconcile-tasks/SKILL.md
  workspaces/task-extraction/run-*/
    SKILL.md, prompt.md, config.json, input.json, result.json
  workspaces/tasks/<hash(providerId:taskId)>/
    task.json, workspace.json
  task-lineage/v1/<hash(providerId)>/
    conversation-tree.json, lineage.json
    threads/*, events/*, tasks/*, jobs/*
    extraction-settings.json, watch.json, latest-extraction.json
    prepared.json, writer.lock, inference.lock
```

Skill 模板首次安装使用 `create_new`，后续初始化保留用户编辑。每批从所选用户 Skill 读取正文，连同补充要求传给模型；工作区保存该次快照便于回溯。技能名限制为安全目录名，文件上限 64 KiB。任务工作区申请按来源和任务确定性分配，重复申请返回同一目录，保存任务及共享 Skill 目录引用；这一步不会替用户创建 Git 仓库或启动任务执行。

数据采用 JSON 文件与写前事务，不引入数据库。`Task` 包含 schemaVersion、revision、rootThreadId、title、detail、episodes、edges、manualCompletion、completionWatermarks。`Episode` 保存 threadId 与 evidenceIds；原始证据保存文件、字节偏移和 generation。人工验收与模型结果分开，模型回写重新检查版本与证据。Prepared 事务支持重启重放。

### 扩展接口

调用 `invoke("task_lineage_request", { providerId, request })`；`request` 是由 method 区分的严格类型对象，未知字段拒绝。`tasks.capabilities` 返回 version=1、支持的方法、harnesses、storage 和 lookbackHours。其余方法先验证已启用 Codex 来源。

| method | 参数 | 返回 |
| --- | --- | --- |
| tasks.list | 无 | 线程、任务、待处理数、诊断和最近模型信息 |
| tasks.messages | threadId | 标准化正文与原始证据 |
| tasks.dirty | 无 | 每会话 state、revision、extractedRevision、pendingMessages、lastChangedAt、lastError |
| tasks.extract | requestId、threadId 或 null | 持久化 Job，立即返回 |
| tasks.jobs | 无 | 最近 30 个 Job |
| tasks.settings.get / set | set 提供 config | 带 revision 的抽取配置 |
| tasks.skills / tasks.skill.get | get 提供 name | 技能名称 / 正文 |
| tasks.workspace.request | taskId | taskId 和工作目录 path |

Job 保存 id、requestId、threadId、state、createdAt、finishedAt、error。相同 requestId 与范围重复提交返回原 Job；改范围则拒绝。每来源只准入一个 queued/running Job，跨进程推理租约防止重叠。应用重启后遗留 Job 标记 interrupted，不自动重复付费；新 requestId 才启动重试。手动触发先扫描，再执行一批；选中主会话包含派生线程，null 表示全部来源线程。

### 配置、状态与调度

默认配置：harness=claude、model=haiku、skill=extract-tasks、每批 $0.25、超时 90 秒、扫描间隔 60 秒、静默 20 秒。harnessInstanceId 为空时使用唯一实例或本机默认；多个 Claude 实例要求选择绑定。配置版本冲突拒绝覆盖。实际模型名称来自 CLI 回执，不能用别名冒充实际模型。

扫描新正文或文件重写时推进 dirtyRevision，工具执行不会推进。抽取捕获输入 revision/count；成功只确认该批水位。推理期间追加的正文保持 dirty，失败不推进水位。过期消息不填充上下文，长正文最多 2500 字符并带截断标记；单批最多 6 条、约 5000 字符，候选任务只向模型传摘要。父子关系仍依据结构证据计算。

后台默认关闭。开启后在应用运行期间扫描，只对静默期结束的确切线程抽取，避免父会话就绪时误选仍活跃的子线程。最多预授权 5 批，运行前扣减，空闲/静默等待不消耗；失败暂停，用户可再次授权。设置和后台开关变化使下一次调度重新计算，已运行批次不会因暂停而撤销。手动及后台抽取均保存 Job，事件加页面轮询同步状态。

## 涉及模块

- `crates/codepet-task-lineage/{src/management.rs,skills}`：用户目录、模板、配置、Job、工作区和模型上下文组装。
- `src/{service,watch,extraction}.rs`：增量水位、静默调度、统一抽取接口、CLI 进程控制与证据校验。
- `src-tauri/src/task_lineage.rs`：组合现有 Provider Host 和应用数据目录，提供本地扩展及后台生命周期。
- `frontend/lib/{taskLineage.ts,TaskLineage.svelte,TaskExtractionSettings.svelte}`：类型调用、交互、异步状态及设置。

## 风险

- 抽取期间新增记录或用户验收：并发回归验证旧结果不吞新消息、不覆盖人工状态。
- 重复收费和遗留任务：请求幂等、租约、预算先扣及重启测试；CLI 失败也可能产生费用。
- 用户 Skill 修改丢失或目录逃逸：首次安装保留测试、路径校验、持久化快照。
- UI 状态不收敛：事件加定时查询，浏览器测试验证排队到完成、配置保存和移动尺寸。

## 测试计划与结果

验证通过：crate 单元/流程测试共 23 项、Tauri `cargo check --lib`、前端定向 Vitest 3 项、Vite build 和 Edge Playwright。浏览器验证覆盖设置与 Skill、异步 Job、申请工作区、证据、图/泳道、焦点、人工验收、480/820/980 宽度。全仓 `tsc --noEmit` 仍为原有 27 项诊断，本次未增加 TypeScript 文件诊断。

用户目录实测使用 `managed_probe`，仅合成两条正文：Rust 安装并读取真实用户 Skill，经本机 Claude `haiku` 返回一个带证据任务，实际模型 `deepseek-v4-flash`，报告费用 $0.013895。回执位于用户数据目录 `workspaces/task-extraction/run-LAB1Ef`，含五份可审计文件。

## 知识沉淀

本文记录现行运行时；早期 [接入评审](../20-product/task-lineage-codepet-integration.md) 保留当时差距与后续范围，[交付记录](../40-runbooks/task-lineage-v1-delivery.md) 记录真实数据试跑。

## 未知项

尚未在 macOS 运行。原生桌面完整交互、多 Claude 实例实际账户隔离与 Remote 接入未做端到端验收；当前浏览器 IPC 模拟测试不等价于这些验收。持久化工作区尚无自动清理策略，应由用户按需要清理历史运行目录。
