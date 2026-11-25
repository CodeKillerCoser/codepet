# 任务加工历史、排队取消与实际用量

## 背景

召回原先只保存 Run 当前索引状态，无法回答某次加工何时完成、产生什么结果、用了多少 Token。用户要求从任务图谱时钟入口打开全局侧栏，按次查看加工记录，并能取消尚未开始的任务；任务列表还需提供主动加工入口。

## 目标

- 每次加工独立留痕，显示状态、结果、完成时间、耗时、实际用量。
- 侧栏沿用 AppSidePanel 的完整卡片、标题、调宽和窄屏行为；详情沿用全局 Dialog 的层级与焦点。
- 取消具体排队项；重复扫描、文档更新、进程重启都不能把旧记录误当成新的加工。

## 非目标

本次不重做模型设置和本地环境安装，不改变 Agent 的 finish 输入，也不修改 Cognee 的抽取算法或原生渲染模板。不估算上游未报告的 Token，不补造旧索引的时间及结果。

## 现状理解

权威数据仍是 Task/Run 账本；Rust 快照生成文档，Python Cognee 负责加工，Catalog SQLite/FTS 和原生图、向量索引都是派生数据。每个模型配置 fingerprint 使用独立 Catalog。共享侧栏已有唯一 Host，详情必须 portal 到主题根的 app-window-host，不能在业务容器内另设窗口层级。

## 实现路径

每个 Catalog 增加 processing_jobs，保存加工 UUID、Run、文档 hash/快照、模型名称、阶段和起止时间。runs.job_id 指向当前排队项。旧 ready 记录迁移为已完成且时间/用量未知，不为填历史重新调用模型；重启将未结束项记为 interrupted，再创建新的恢复项。

取消在同一事务中比较加工 UUID、queued 状态、当前 job_id、desired_hash 和删除标记。已开始返回冲突；成功后同版本扫描保持 cancelled，新文档版本才重新排队。旧加工迟到结束不能恢复已取消的新版本。失败项保留独立记录，既有退避重试另建队列项；手动重试也创建新项。切换配置后，非活动 Catalog 的排队/加工项显示 paused，禁止操作；列表合并各 Catalog 并分页。

### 从任务列表主动加工

任务看板卡片、表格任务标题旁及任务详情标题旁提供 Sparkles 图标“加工任务”。点击处理该 Task 所有已结束执行（待确认、已发布、已完成）；只有开发中执行时禁用并说明原因。图标与打开任务的按钮是兄弟节点，避免嵌套按钮或误选任务；主窗口统一灰阶 hover/press。提交反馈留在列表/详情，查看进度仍走任务图谱的加工历史。

Host 的 recall-history/enqueue 校验 taskId 和 operationId，要求已开启模型加工，再同步权威快照。Catalog 在同一事务中检查各 Run：排队或加工中复用原项，失败退避队列立即可领取；取消项重新排队；已完成项建立新的加工记录并持久化 rebuild 标记。manual_requests 保存操作回执，网络重试及重启后的迟到重试使用同一 operationId，不重复加工。前端失败后保留操作 ID，成功后下一次主动点击使用新 ID。

已有原生 data_id 时，rebuild 调用 Cognee update 的 chunk_level_diff=False、data_cache=False，强制该文档重新加工并保留 data_id。保持 incremental_loading 默认开启，避免将其他已完成文档一并重建；普通文档更新仍用默认增量路径。rebuild 标记跨失败、取消和重启保留，文档变更或加工成功后清除。加工不创建业务 Run、不改账本完成状态；新加工仍可取消排队并独立统计 Token。

加工协程设置 ContextVar。私有 Gateway 的地址和 Bearer 均匹配时，httpx 与 httpx2 的异步发送边界附加加工 UUID；锁定的 LiteLLM 与 OpenAI SDK 分别使用这两个客户端。Gateway 校验 UUID 后用独立关联表连接实际 usage row，不把内部追踪头转发给上游。并发查询没有加工上下文，因此不会混入加工 Token；embedding 拆批逐请求归属。缺失或不完整 usage 显示未报告/部分，旧记录不显示虚假的零用量。

加工成功后按原生 TextDocument/DocumentChunk 的 runId + contentHash 及其结构边收集摘要和实体，保存有限快照；不靠正文中出现 UUID 推断归属。实体最多 200 条、摘要最多 20 条，超限明确标注；同名共享实体保留提示。快照读取失败不重复抽取，索引成功与快照不可用分别表达。详情展示结果和运行信息，不保存提示词、密钥或原始模型响应日志。

## 涉及模块

- Python history/catalog/server/results：持久化状态、事务取消、手动入队幂等、跨配置查询和原生结果溯源；engine 委托原生文档重建，model_trace 关联 SDK 调用。
- Rust model-gateway：校验内部归属并按实际请求记账；task-mcp recall 和 Host task_ledger 转发 history 操作、附加用量，仍限制主窗口调用。
- App/WindowToolbar/TaskKnowledgePage：时钟入口、画布布局与完成后刷新；RecallHistory/RecallProcessingDetail/recallHistory：共享侧栏、全局详情及未知值格式。
- TaskProcessingButton/TaskCollection/TaskRunDetail/TaskLedger：复用一个提交控件，在看板、表格和详情中保持动作、忙碌和错误反馈一致。
- webcontent-contract：增加主动加工后 backend API 为 38，须同时更新 Rust Host 与 sidecar，单独更新前端不足以提供历史和主动加工能力。
- QA fixture：只在开发入口模拟排队/失败等界面状态；完成记录读取隔离实际加工回执，产品入口不加载样例。

## 风险

- 取消与领取、版本变化竞争：Python 测试覆盖同版本扫描、claim 冲突、迟到完成、失败重试与重启恢复。
- SDK 换客户端导致漏记：httpx 并发及真实 AsyncOpenAI mock transport 测试覆盖两套客户端；真实集成比较加工前全部请求与归属请求，查询后再次确认加工用量不变。
- 手动重试重复扣用量或误处理开发中执行：Catalog/服务测试覆盖请求幂等、跨任务 ID 冲突、完成后/重启后重放、混合状态、其他任务隔离及重建恢复；Engine 测试约束单文档 update 参数，真实 smoke 检查重建调用模型、保留 data_id、新旧用量分离。
- 别的 Run/旧版本实体串入详情：结构溯源测试覆盖正文 ID、旧 hash、无关 chunk；共享实体不声称独占。
- 窗口覆盖与小屏裁切：浏览器检查全局 portal、点击命中、Escape 顺序、焦点返回、标题省略及底部关闭动作；明暗主题和 1440、820、390 宽度均检查溢出。
- 未更新原生组件：接口返回升级提示，不用模拟结果掩盖服务缺失。

## 测试计划与结果

2026-11-25 macOS arm64：Python unittest 16 项、Rust Gateway/账本/MCP/HTTP/managed 100 项、前端 Vitest 256 项通过；Svelte 检查无错误、保留 5 项已有警告。前端构建和 Tauri cargo check 通过，执行 source-size 和 git diff 检查。具体构建/测试命令沿用 [sidecar 运行手册](../../40-runbooks/task-run-recall-sidecar.md)。

重新构建独立 sidecar 后运行 scripts/task_recall_smoke.mjs：真实 finish 触发一次加工，语言/向量共 8 次请求全部归属，输入 3,611、输出 530，共 4,141 Token；随后 semantic/graph/fulltext 召回成功且加工用量不变。隔离回执位于 artifacts/task-recall-sidecar-qa/mcp-recall-report.json，不修改正式账本和模型配置。

浏览器模拟 IPC 验收侧栏、取消、空/错误状态、长标题、调宽、两层窗口焦点与关闭顺序。图数据和已完成详情来自真实回执；排队/失败状态为明确的开发样例。截图位于同一 artifacts 目录，不视为已安装 App WebView 验收。

主动加工补充验证：Python 21 项、前端 262 项、相关 Rust 23 项通过，主窗口检查无错误；API 38 构建、Tauri 检查及源码尺寸检查通过。浏览器检查看板/表格点击不误打开任务、键盘提交、详情入口、390px 动作点击命中、明暗主题及未配置模型的错误提示。scripts/task_recall_smoke.mjs 使用重新构建的独立 sidecar 真实运行通过：首次加工 9 次请求，手动重建新增 8 次请求（输入 3,567、输出 503，共 4,070 Token），新旧用量分离、原生 data_id 保留；完成后重放同一操作回执不再入队。完整结果在隔离 mcp-recall-report.json 的 manualProcessing/manualUsage 与测试输出中，不代表正式用户任务已执行加工。

## 知识沉淀

任务图谱说明更新入口与刷新行为；全局窗口层级规约继续适用。模型依赖升级时同时复核两种 SDK 发送边界和真实请求归属，避免只验证总用量或只验证语言请求。

## 未知项

Windows 原生安装、实际 App GUI、云端 API 和大量历史 Catalog 的分页性能尚未人工验收。历史保留/清理策略尚未产品化；不主动删除既有历史。
