# TaskRun 原生向量、图与全文召回方案实测

> 2026-11-25 更正：撤回将 Ladybug 等数据库作为 Cognee 完整替代的推荐。本页仅验证存储、检索和已有加工结果的展示，不能证明替代文档加工。用户要求使用现成加工框架，不接受 CodePet 自行实现抽取、归并和索引生命周期。后续筛选见 [Cognee 加工框架客户端替代核查](cognee-processing-framework-alternatives.md)。

## 背景与状态

2026-11-25 选型实验，尚未决定生产架构。用户要求模型外置：云端 API，或 App 引导安装 MLX 与本地模型；希望替代 Cognee 的 Python 客户端分发，同时保留向量、图、全文索引与加工图可视化。前序证据见 [Cognee 验证记录](../40-runbooks/cognee-task-run-recall-probe.md)。本次使用该实验快照，未访问或修改生产账本。

## 目标与非目标

目标是验证原生引擎的三路召回、内容替换、共享实体保留、重启恢复、图导出及分发体积。结果返回 Run，workspace 只作为软优先级。

本轮不接正式 MCP/finish 事件，不迁移业务账本，不内置模型，不对各框架重新进行实体抽取，不声称完成大规模性能或跨平台打包验证。

## 现状与实测证据

`prototypes/cognee-task-recall/native/` 提供独立原型。SQLite、Cozo、Ladybug 由同一份 C++ JSON-lines 驱动分别链接 C、Rust、C++ 原生库；Helix 使用发布的 Rust UniFFI 库和 Node 绑定。Python 仅用于一次性导出已有实验库的向量，原生运行与本次报告服务均可脱离 Python。

证据根目录为忽略的 `artifacts/cognee-task-recall-qa/native-comparison/`：`fixture.json`、`comparison.json`、各引擎目录的 `result.json` / `audit.json`、`build.log`。固定 9 个 Run、11 块、112 实体、74 语义关系、118 条块到实体关联，复用 Cognee 抽取回执。文本块向量沿用已有索引，恢复后缺失的 6 个实体向量及 8 个查询向量由同一个 MLX embedding 服务补齐；新增 LLM 调用为 0。

所有引擎读回的完整节点与边属性均与输入一致；8 个查询的原生向量首位结果均与独立精确 cosine 参考计算一致。原生路线使用统一中文双字切分，图路线为实体向量入口加一跳邻接扩展；Cognee 基线使用原生默认分词和自身图检索。不能将结果差异全部归因于引擎，也不能用缓存后建库秒数与 Cognee 的含模型加工时间比较。

| 方案 / 版本 | 向量 / 图 / 全文实现 | 原生文件 / gzip | 本轮实验库 | 融合 Top1 / Top3 |
| --- | --- | --- | --- | --- |
| SQLite 3.54.0 + sqlite-vec 0.1.9 | 精确 KNN / 邻接表 SQL / FTS5 BM25 | 0.33 / 0.10 MB | 8.77 MB | 7/7 · 7/7 |
| Ladybug 0.22.3 | HNSW / 原生属性图 Cypher / FTS BM25 | 24.29 / 6.70 MB | 8.75 MB | 6/7 · 7/7 |
| Cozo 0.7.6 | HNSW / Datalog / FTS TF-IDF | 26.65 / 9.55 MB | 2.54 MB | 7/7 · 7/7 |
| Helix embedded 0.3.3，JS SDK 3.2.0 | HNSW / 原生属性图遍历 / BM25 | 133.11 / 48.12 MB | 3.87 MB | 6/7 · 7/7 |
| Cognee 1.6.10 基线 | LanceDB / Ladybug / 原生词法检索 | 核心依赖文件清单约 1.07 GB | 约 12.82 MB | 6/7 · 7/7 |

体积均不含模型。原生数字是 macOS arm64 实测文件大小；gzip 为逐文件压缩之和，均非签名安装包。SQLite 驱动已静态编入 sqlite-vec，但复用系统 SQLite；Host 已使用 `rusqlite` 的 `bundled` 特性，生产可考虑复用，实际链接增量需测量。Ladybug 包含两个扩展、不含 Explorer。Helix 仅计当前平台原生库，所测 JS 绑定还需 SDK、FFI 和 Node；直接 Rust 链接的最终大小未知。Cognee 的 1.07 GB 是去掉 GLiNER extra 后 167 个依赖发行版 RECORD 文件合计，未包含解释器，不能作为正式包大小。

数据库大小包含固定页/向量块预分配及本轮受控替换，不宜按 9 个 Run 线性外推。SQLite-vec 该配置为精确扫描，没有验证大量向量下的延迟。

SQLite、Ladybug、Helix 的 12 项受控更新检查及三项重启恢复检查通过，包括旧词消失、新词出现、向量替换、更新后的向量索引实际命中、旧关系删除、新关系写入和其他 Run 的共享实体关联保留。仅 SQLite 与 Ladybug 的原型将整份控制替换放在显式事务内；Helix 原型暂用多个请求，不能据此证明整个 Run 发布原子性。

Cozo 的旧词检查失败。独立最小样本 `uniqueoldword → uniquenewword` 使用 `:put` 后，查询旧词仍返回新文本，倒排表同时保留两词；删除再插入仍可复现，整库重建 FTS 后消失。官方最新正式版为 2023-12-11 的 0.7.6，仓库最后推送时间为 2024-12-04（本次 GitHub API 观察）。暂不推荐作为新生产依赖。

## 图可视化

原型报告使用各引擎实际查询导出的节点、边、证据及 Run 关联，提供数据库切换、实体搜索、按 Run 筛选、节点/关系详情、原文查看和 JSON 下载。按 Run 展示的边必须有可定位的引用来源；引用出现不证明关系蕴含成立。

图渲染可由 CodePet 共用一套前端，SQLite 也能展示关系图。Ladybug 另有官方 [Explorer](https://github.com/LadybugDB/explorer)，其快速部署依赖 Docker，不应将 Explorer 容器当作桌面产品的必需组件。本轮验证的是统一前端，没有运行各产品官方查看器。

后续已完成 [Ladybug + Cognee 原生渲染独立验证](../40-runbooks/ladybug-cognee-native-renderer.md)：复用原前端与已有展示元数据，由 Node 从原生库读回并生成页面，运行不需要 Python。该验证没有移植 Cognee 文档加工、拓扑预处理或新的语义布局计算。

## 历史自研路线与涉及模块（已撤回）

以下保留原存储实验的取舍依据，不再作为当前接入建议：Ladybug 的原生图和约 24 MB 库与扩展适合继续做存储对照；SQLite 更轻，适合有限深度遍历。Helix 原生包较大；官方 Rust 嵌入式文档要求 Git 依赖，crates.io 3.0.0 没有 embedded 特性，稳定发布与 Windows 集成未验证。

这些方案仅替代 Cognee 的存储/检索部分。若采用这条已撤回的自研路线，CodePet 原本还需实现以下职责；这正是它不满足当前需求的原因：

- `crates/codepet-task-lineage`：finish 成功后保存完整版本快照，补齐验证结果记录，生成内容 hash、revision 与来源信息；当前工作树 MCP 只有四工具，需与 finish 工具所在分支对齐。
- 新的独立知识模块：持久队列、合并重复加工、模型调用及 schema 校验、embedding、实体归并、按版本发布和删除。JSON 账本继续作为业务事实，图与向量是可重建索引。
- `crates/codepet-task-mcp`：提供轻量 recall，融合语义、全文、图、时间和 workspace，并返回 Run 及命中依据；Agent 继续只报告任务、进度、结果。
- `src-tauri` 与打包脚本：统一知识服务接口、平台链接/签名、扩展固定版本和离线加载；不在业务调用处散落平台分支。
- 前端设置与图页面：云端 API 或本地 OpenAI-compatible 服务配置、健康状态、加工进度与来源图。云端与 MLX 两种模式均保留 LLM + embedding；换数据库不会直接节省同量抽取的模型 tokens。

原自研路线曾粗估为 macOS 1–2 周、完整分发 3–5 周，未做完整拆单；该估算不适用于现成框架接入，不能继续作为当前规划依据。

## 风险与验证计划

- 负例误召回：四条原生融合路线的财务负例都返回 9 个候选；扩充正负例并独立校准拒绝阈值，不能以 7 个正例通过作为上线门槛。
- 模型事实与实体归并：本轮沿用实验 types 和回执，没有证明抽取更好；补齐 finish 验证记录，按边标注事实/计划/已验证结果，评估重名与别名。
- 更新一致性：增加并发版本、失败中断、删除、共享实体多来源及恢复测试；验证发布版本淘汰和 embedding 模型/维度变更后的全量重建。
- 分发与资源：在 macOS arm64/x64 和 Windows 分别构建并测完整包、签名、冷启动与 RSS；扩展需纳入固定版本发行包。Helix 的建索引回执为异步操作，原型已等待激活后才查询。

已运行 `node --test prototypes/cognee-task-recall/native/recall.test.mjs`（3 项）、原有 Python 单测（15 项）、四库真实读写/查询/控制替换/重启及完整图属性核对、精确 cosine 对照、`npm run check:source-size` 和 `git diff --check`。图报告已验证节点与 Run 筛选；最终 UI 检查回执见证据目录。

## 未知项与知识演进

长期多版本索引残留、中文大语料性能、跨平台体积、Rust 静态链接与正式模型费用尚未实测。此次仅记录选型证据，不替代正式 ADR；选型确认后再将业务接口、schema、质量门槛与迁移策略写为长期约束。

主要官方来源：[sqlite-vec](https://github.com/asg017/sqlite-vec)、[Ladybug 原生库](https://github.com/LadybugDB/ladybug/releases/tag/v0.22.3)、[向量扩展](https://docs.ladybugdb.com/extensions/vector/)、[全文扩展](https://docs.ladybugdb.com/extensions/full-text-search/)、[Cozo](https://github.com/cozodb/cozo)、[Helix embedded](https://docs.helix-db.com/database/helix-db/start-here/local-development/embedded-database)、[Helix 索引激活](https://docs.helix-db.com/database/helix-db/query-guides/vector-indexes)。
