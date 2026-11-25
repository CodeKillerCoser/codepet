# 任务知识图谱预览

## 背景与目标

用户希望在主界面继续使用实验中喜欢的 Cognee 原生图界面。入口位于左侧“知识图谱”，和 Codex 工作台并列；展示当前活动数据目录中已经加工的真实 Run 图，不向产品分发示例图。Agent 工具不增加参数，也不通过 Agent comment 触发预览。

## 行为

页面打开时读取一次，手动“刷新图谱”获取新快照。顶部显示实际节点与关系数，并可进入模型配置。没有索引时显示配置和加工引导；后台繁忙返回 409 时显示准备中，已有图继续可用。请求失败可重试，离开页面会取消未完成的渲染并销毁 iframe，不保留隐藏图实例。

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

Windows 与实际 Tauri WebView 的沙箱兼容性尚未人工验收；大型语料的布局耗时和节点上限需要基线。四个视图保留上游行为，刷新会重置图内选择、平移与缩放。
