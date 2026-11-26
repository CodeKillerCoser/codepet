# 任务图谱预览

## 背景与目标

用户希望在主界面继续使用实验中喜欢的 Cognee 原生图界面。入口位于左侧“任务图谱”，和 AgentHub 并列；展示当前活动数据目录中已经加工的真实 Run 图，不向产品分发示例图。Agent 工具不增加参数，也不通过 Agent comment 触发预览。

## 行为

页面打开时先读取最近已发布的图谱；停留期间每 5 秒按版本检查更新，不依赖加工历史侧栏是否打开。隐藏页面暂停轮询，恢复可见时补读。移除重复的页面标题、计数及模型配置/刷新操作栏，计数保留在原生图内。模型配置从设置进入，没有索引时仍提供配置和加工引导。后台繁忙返回 409 时显示准备中，已有图继续可用，并在提示中提供刷新；请求失败可重试。离开页面会取消未完成的渲染并销毁 iframe，不保留隐藏图实例。

窗口工具栏的时钟打开共享全局侧栏“加工历史”，标题、圆角、内边距、调宽及窄屏抽屉遵循全局框架。记录的弹出图标打开全局详情弹窗，持久化、取消及用量归属见 [加工历史](task-processing-history.md)。

Graph、Schema、Memory、Semantic 使用 Cognee 1.6.10 原始模板和 JavaScript，保留搜索、缩放、布局切换、实体检查和主题按钮。只有嵌入尺寸的 CSS 单独适配，避免窄窗口的工具栏重叠。Semantic 的坐标、簇和邻居来自 Cognee 的 `visualize_semantic_json`，不由浏览器随机生成。

## 模块与边界

- `frontend/lib/TaskKnowledgePage.svelte`：页面状态、刷新、空态和模型入口。
- `frontend/lib/cogneeGraph.ts`：通过主窗口 IPC 读取，按 webcontent 的 base URL 加载渲染模块。
- `public/cognee-view`：薄渲染入口、沙箱和尺寸样式；`vendor` 保存未改写的上游视图、D3、字体与许可证，`provenance.json` 记录哈希。
- `recall-graph`：Rust 转发到 sidecar。应用错误或 409 不重启仍健康的索引进程；真实连接故障才恢复进程。
- Python sidecar：读取原生 graph/schema/memory JSON，并按需附加原生 semantic 布局；召回内部的图遍历不计算可视化布局。

渲染 iframe 只允许 scripts，不授予 same-origin。图数据按 JSON 转义后嵌入；模板、D3 和字体均随 webcontent 提供，字体在父页面读取后内嵌。iframe 的 CSP 禁止网络连接，无法读取父页面 DOM 或调用 Host IPC；父子消息校验具体 Window 和一次性 token。页面不接收或展示模型密钥。

## 框架兼容性与冷启动

Cognee 1.6.10 的 `_LeasedValueProxy` 把 `retrieve` 包装成 `*args, **kwargs`，原生语义预览按签名检测不到 `include_vector`，会误走重新 embedding。独立包在构建时仅给该包装函数增加 `@wraps(attr)`，保留签名；不改变抽取、向量读取、投影或聚类算法。构建检查补丁位置唯一，`cognee-patches.json` 保留原始和修改后哈希，升级框架必须复核。

冷启动曾在 Arrow/Pandas 加载阶段超过 45 秒。主线程直接调用约 9.3 秒，但仅迁移线程和限制数值线程并未消除 RPC 超时；相同包和数据仅增加 `AWS_EC2_METADATA_DISABLED=true` 后，真实 Rust → sidecar 请求 7.40 秒通过，且没有新增模型用量。桌面端禁用 AWS 环境元数据探测。Cognee 事件循环保留在 Python 主线程，HTTP 与队列在线程中提交请求；数值库最多 4 个线程，数据库使用框架支持的进程内适配器，由单例 sidecar 提供进程隔离。

## 风险与验证

- 私有数据进入模板：Node 测试覆盖 JSON 脚本转义、无外部资源和原始 vendor 哈希；浏览器确认沙箱不能读父 DOM。
- 刷新打断加工或丢失现有图：Rust 覆盖健康服务 409 不重启；组件覆盖已有图保留、禁用重复刷新和卸载后迟到回执。
- 依赖升级改变接口：固定版本、构建期补丁检查、上游资源哈希测试及真实 semantic 导出回归。
- 窄窗口和异常状态：浏览器使用真实加工快照验证四个视图、820×600、空数据、繁忙及失败后恢复。QA 模拟 IPC 传输，图本身来自真实回执；不冒充实际 App WebView 验收。

运行 `npx vitest run frontend/lib/TaskKnowledgePage.test.ts frontend/lib/navigation.test.ts`、`node --test scripts/cognee_graph_view.test.mjs`、MCP 的 `busy_preview` 与 ignored `real_semantic_preview_export`，再执行前端构建、Tauri 编译与源码尺寸检查。此次组件/导航/模型设置 10 项、离线资源与转义 3 项、繁忙服务回归及真实 semantic 导出通过；真实快照为 152 节点、347 关系、9 个语义簇。完整 sidecar 配置、实际集成命令与平台限制见 [运行手册](../../40-runbooks/task-run-recall-sidecar.md)。

## 未知项

Windows 与实际 Tauri WebView 的沙箱兼容性尚未人工验收；大型语料的布局耗时和节点上限需要基线。四个视图保留上游行为；自动刷新保留所选视图、搜索词、主题和 Graph 平移/缩放，不承诺恢复原生实体详情选择及 Schema/Memory 独立视口。

## 加载性能修正（2026-11-26）

152 节点、347 边的真实加工样本中，基础图导出约 1.03 秒，首次 Semantic 导出约 37.12 秒；同一进程后续 Semantic 约 0.06 秒。浏览器原生视图首次挂载约 0.23 秒，因此不能把主要等待归因于卡片、字体或 Canvas。此前 Rust 始终发送 semantic=true，普通 Graph 首屏被原生语义初始化、投影和聚类阻塞。

API 41 的前端先请求 semantic=false；首次点击 Semantic 时才由绑定 Window/token 的沙箱消息请求原生坐标和簇。等待或失败时 Graph、Schema、Memory 继续可用，失败可重试；返回结果的 previewVersion 必须匹配当前图，避免加工期间把新版坐标应用到旧图。vendor 文件及其哈希保持不变，集成层更新原有簇对象，不重建 iframe 或重复绑定原生控件。

Engine 在每个 fingerprint/dataset 下缓存基础图和完整语义图；派生 JSON 原子写入、权限 0600，缓存损坏回退原生导出。所有索引写入前后均清除两种缓存，失败、删除、手动重建也不能遗留旧图。预览与索引写入共享异步锁。空索引不创建伪造快照；缓存格式和 Cognee 版本不匹配时重建。原生升级改变导出含义时必须更新 GraphCache.VERSION。

同一真实样本在新进程读取已有缓存时，基础图约 1.7 毫秒、语义图约 2.3 毫秒，浏览器挂载约 0.20–0.35 秒。该结果不代表第一次创建语义缓存的成本已经消失；首次 Semantic 仍可能冷启动。原生 Story 坐标已全部固定时，嵌入层直接显示已定位节点，不让加载遮罩阻挡交互，未固定的布局继续使用原生模拟。

回归覆盖缓存复用/重启/隔离/损坏/写入失败失效、按需语义请求、旧接口兼容、沙箱 token、失败重试和四种真实视图。测量与回执在 artifacts/graph-performance；完整证据和复测步骤见 [图谱加载性能排查](../../40-runbooks/task-graph-loading-performance.md)。API 41 同时涉及 Host 与 sidecar，不能只替换前端；本轮尚未替换用户安装版 App。

函数续测：二维去重叠已接入空间网格候选筛选，密集点退回原有 NumPy 向量化步骤；保留种子、40轮与位移公式，不改变 PCA、聚类和 2,000 节点 Semantic 上限。700 节点/1,600边/1,024维合成向量的完整计算3次中位数约446→184ms；正式去重叠函数单独约359→72ms。确定性、RNG、边界、密集点及真实快照一致性通过。函数加速不等同于把预计算迁入加工阶段，安装版亦尚未替换。多次基准和大图测量边界见上述排查手册。

## 边加工边预览（2026-11-27）

Service 原先将预览和整次加工共用 busy 锁，连续队列让已完成的图也经常返回 409。现在每次有效加工完成后，记录 Catalog 结果并发布基础图到 fingerprint/dataset 隔离的 `published/preview-graph.json`；工作缓存仍在原生写入前后失效，发布快照独立保存，下一次写入或失败不清除它。它表示最近一次成功预览，不是正在写入的数据库，也不用于 Agent 召回证据。初次升级会在下一次写入前尝试发布既有索引，尚无首份快照时仍返回 409 并自动重试。

`recall-graph` 可传 knownVersion；版本相同仅返回 unchanged 与 previewStatus，不重复传输图数据。页面保持旧图直到新渲染器就绪，相同版本不重绘；拖动或新图初始化期间发生交互则延后替换。首份结果出现即可浏览，无需等队列清空。首次 Semantic 仍按需准备；加工繁忙时复用同版本语义快照，否则保留基础视图并自动重试，不能混合不同版本的坐标。

37 项 Python、10 项前端、4 项沙箱桥接测试通过；真实既有 152 节点/347 边快照在隔离模拟写锁下约 0.25ms 返回，条件回执 157 字节。浏览器 QA 的首个 409 后自动显示，并在换版后保留 Schema；这些不是已安装 App 的端到端验收。详见 [渐进预览复盘](../../40-runbooks/task-graph-progressive-preview.md)。安装版 API 40 尚未替换，本次需随新原生/sidecar 安装包生效。

## 冷启动快照优先（2026-11-27）

首个基础图请求由 Rust 直接读取已发布快照，按模型配置 fingerprint/dataset/格式版本校验且限制 32MiB，不等组件准备或 Python 启动。连接状态不伪造队列计数；后台就绪后正常读取接续，同版本不重绘。准备阶段每秒检查，其余维持五秒。没有快照仍显示准备；sidecar 在握手和 HTTP 就绪后，主动为旧索引补生成基础图，即使没有新加工任务也会发布。补生成不调用模型或读取写入中的数据库。

本机本地 debug 原生包使用真实 510 节点、1,270 边快照，首次/再次启动的主界面初始化约 1.74/1.65 秒，图谱页面首屏约 251/243ms；后台补生成的隔离真实副本 166ms 握手、2.89s 发布。这些不是正式发行包或全新空数据加工的速度承诺。完整边界、复测和测量字段见 [冷启动复盘](../../40-runbooks/task-graph-cold-start.md)。
