# Ladybug 读取现有 TaskRun 并复用 Cognee 原生图渲染

## 背景

2026-11-25，用户希望保留 Cognee 原生图的视觉与交互，验证能否以 Ladybug 原生存储脱离 Python 展示现有 task-run。前序 [四库实验](../10-architecture/native-task-run-recall-evaluation.md) 使用自写 SVG 查看器，本轮直接复用 Cognee 的前端。

## 目标与非目标

验证原生图持久化、读回与页面组装不依赖 Python，并保留 Graph、Schema、Memory、Semantic 四个标签页。所有资源应能从本机加载。

本轮不移植完整 Cognee 加工器，不重新调用 LLM/embedding，不把历史模型抽取变成已验证事实，不接正式 MCP 或 Tauri，也不修改生产账本。

## 现状与证据

Cognee 1.6.10 的 `cognee_network_visualization.py` 负责拼装模板；`views/*.py` 只是读取同名 JS。Schema、Memory 等结构及语义 PCA/聚类由 Python 预处理产生，属于渲染所需输入，不能仅复制一份空页面就声称完成替代。

本轮从前序实际生成的 `runs/20261124T195156701196Z/cognee-graph.html` 提取 JSON 字面量，未执行其 JavaScript。内容是 9 个 Run 的既有加工结果：148 节点、325 边，包括 112 实体、74 语义关系。与前序比较用 `graph-baseline.json` 不同，这份 HTML 是原 Cognee 页面实际展示的同一份导出，以保持布局数据与节点 ID 一致。

独立证据目录为忽略的 `artifacts/cognee-task-recall-qa/ladybug-cognee-renderer/`：

- `graph.lbdb`：新的 Ladybug 0.22.3 原生库；节点属性 JSON、真实图边及处理后的视图元数据分开保存。
- `assets/`：原生六个 JS 模块与模板保留逐字节哈希；D3 7.9.0 和 Inter 四字重字体本地化，保留许可证。浏览器不需访问 CDN/Google Fonts。
- `provenance.json`：来源 HTML 哈希、前端文件哈希、版本、图数量及拓扑 hash。
- `verification.json`：完整读回、属性替换/恢复、HTML 注入转义、结构不匹配拒绝、空 PATH 冷启动及本地资源校验。
- `runtime-audit.json`、`ui-qa.json`、四视图截图：运行与界面证据。

Node 与原生驱动从复制后的目录运行，空 PATH 中无 Python/包管理器，页面及 JSON 仍与 Ladybug 读回一致。运行过程只启动明确路径的 C++ 驱动，没有导入 Cognee/Python 包。原有 Python 环境仅提供首次复制的前端源码与历史处理结果，并非运行依赖。

## 实现路径与涉及模块

源码在 `prototypes/cognee-task-recall/native/cognee-renderer/`：

- `stage.mjs` / `fonts.mjs`：复制并记录上游资源、下载前端依赖、导入独立数据库；已有数据库拒绝覆盖，避免破坏实验历史。
- `snapshot.mjs`：仅解析约定 JSON 赋值，检查节点/边引用与拓扑，转义脚本终止符；保留原展示数据协议。
- `store.mjs`：复用父原型 C++ Ladybug 驱动，事务写入节点/边/视图快照；读回由原生查询完成。
- `render.mjs`：替换原模板与数据 token，仅将外部脚本、字体链接改为本地资源；没有重画一个相似 UI。
- `server.mjs`：独立 Node HTTP 服务，每次页面请求从 Ladybug 读回，资源请求限于本地允许列表。
- `verify.mjs`：真实数据库及隔离启动验证。属性更改可反映到图节点和实例名称后恢复原文。

运行前端资源约 1.83 MB，含未做额外压缩优化的字体；Ladybug 动态库约 21.60 MB，实验库约 2.69 MB。均为当前 macOS arm64 文件大小，未计 Node、签名或正式安装包。

## 复跑

仓库根目录运行：

```sh
node prototypes/cognee-task-recall/native/cognee-renderer/stage.mjs
node prototypes/cognee-task-recall/native/cognee-renderer/verify.mjs
node artifacts/cognee-task-recall-qa/ladybug-cognee-renderer/app/server.mjs artifacts/cognee-task-recall-qa/ladybug-cognee-renderer 50547
```

首次 stage 需要前序实验产物及互联网。再次 stage 应传入新证据目录。校验时先关闭占用实验库的 server。已 stage 的目录可独立复制，通过 Node 启动，不需要原 `.venv`。此原型验证 macOS arm64，其余平台尚未构建。

## 验证与风险

自动验证已通过：完整节点/关系/视图数据读回与原处理快照相同；修改 HTTP 实体名称可读回并进入生成 HTML，随后完整恢复；原前端哈希一致；恶意脚本终止符被转义；结构不匹配明确拒绝；空 PATH 冷启动；D3 与字体资源内容哈希匹配。源码尺寸及 diff 检查通过。

界面检查覆盖四标签页、搜索与 Enter 跳转、Schema 实例详情，以及本地资源加载；具体视图元素、尺寸和错误检查以 `ui-qa.json` 为准。

- 展示元数据过期：本轮复用既有 Schema/Memory 结构、语义坐标与聚类；拓扑变化会拒绝展示。新文档、删除、跨 Run 更新需要独立实现元数据重算，并验证所有视图同时更新。
- 属性依赖不完整：当前名称读回刷新仅覆盖部分引用，不是完整失效机制；新增属性变更测试须覆盖 Schema 样例、分组标签、颜色及聚类描述。
- 端侧完整链路：当前新库仅验证图展示兼容，没有在该库重建向量/全文索引；三路召回沿用四库实验的独立证据。后续正式模块需统一版本发布、索引和展示数据。
- UI/平台差异：保留原前端行为，未为 CodePet 重做布局；需要在 Tauri WebView、最小窗口及目标平台补测，不能以 Chromium 展示通过替代应用集成。

## 未知项与知识沉淀

结论是展示层可以脱离 Python 保留原生效果，完整增量加工链路尚未替代。正式接入时可保留这套前端输入协议，由 Rust 或 Node 产出预处理数据；语义布局与聚类算法的迁移需单独对照。本文与前序实验文档记录范围边界，选型确定后再建立长期架构决策。

## 后续：独立浏览器模块

用户进一步要求单独拿出渲染层，新增 `prototypes/cognee-graph-view/`，本地私有包名为 `@codepet/cognee-graph-view`，未发布 npm。`src/index.mjs` 提供 `mountCogneeGraph(host, snapshot)`、`update(nextSnapshot)`、`destroy()`；`src/assemble.mjs` 在浏览器中组装原模板；`frame.html` 与 `src/frame.mjs` 将原脚本的固定 DOM ID、样式及全局变量隔离在独立文档中。它接收 JSON，不导入 Node、Python、Ladybug 或模型代码。

构建脚本仅从前序证据复制已核对哈希的原前端、许可证和示例快照，产物位于 `artifacts/cognee-task-recall-qa/cognee-graph-view/`。该目录不含原生驱动或数据库；开发服务 `scripts/serve.mjs` 只提供静态文件，亦可换成其他静态托管。前端资源约 1.83 MB，示例的真实图 JSON 另计。

浏览器已验证挂载、属性更新、完整快照替换、卸载及重新挂载：仅一个 iframe 存活，宿主页面没有 `_vizNodeById` 全局变量，输入对象没有被原脚本修改。四视图、静态资源请求和窗口检查以新产物中的 `ui-qa.json` 为准。`node --test prototypes/cognee-graph-view/test/*.test.mjs` 覆盖 Cognee 原始字段映射、破损图输入拒绝、脚本终止符转义、嵌套资源路径与 SSR 导入。

当前约束：资源与宿主同源；每次 update 重建 iframe，选择、平移和缩放会重置，调用方需 await 顺序更新。完整四视图仍依赖已加工的 Schema、Memory、语义坐标与聚类；模块不会生成这些数据。Svelte/Tauri 的实际 WebView 接入尚未验证。后续应围绕这些公开接口测试，保留版本/许可证记录，避免将本地提取模块误称为 Cognee 官方维护的独立组件。

封装时实际发现：将远程 URL 替换为绝对本地 URL 后，再替换 `/assets/…` 会重复拼接主机名，导致图数据已进入内存而 D3 未加载、其他视图为空。改为一次匹配替换，并用完整 `script.src` 断言覆盖。挂载 ready 也改为检查四视图入口均初始化成功；仅检查节点数量不足以证明原生渲染可用。
