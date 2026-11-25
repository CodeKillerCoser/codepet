# Cognee 加工框架客户端替代核查

## 背景

2026-11-25 选型调研，尚未决定生产方案。CodePet 希望在 Run 完成后把文档交给现成框架，由框架完成抽取、embedding、图构建、更新和检索；coding agent 只报告任务、进度与结果。模型通过云端或用户安装的 MLX 服务提供。编译型语言优先，Node 也可接受。

前一轮原生数据库实验复用了 Cognee 的抽取结果，证明的是存储和展示，不是替代加工。相关推荐已更正。用户要求继续筛选完整加工框架，将 PostgreSQL + EdgeQuake 保留为备选。

## 决策状态

**继续评估 cognee-rs，PostgreSQL + EdgeQuake 保留备选。** 已完成 [9 个 Run 的真实原文对照与外部模型构建测量](../40-runbooks/cognee-rs-task-run-comparison.md)：Rust 独立跑通加工、更新、召回、原生图与重启，官方 CLI 为 183.50 MiB、gzip 66.55 MiB；相同内容 update 仍重算，Semantic 投影未实现，共享控制实体保留尚未验证。这不是确认可上线或与 Python 完全等价的架构决策。下文的“未实测”描述是初次源码筛选范围；实测结论以上述 runbook 为准。

不将数据库、薄 SDK、单独模型调用库计为完整替代。必要能力包括文档切块、LLM 实体关系抽取、embedding、归并和来源关联、更新删除、多路检索。持久化全文索引与图可视化另列，不能用“支持 hybrid”代替逐项核查。

## 候选与证据

### cognee-rs：最直接的原生候选

[仓库](https://github.com/topoteretes/cognee-rs) 位于与 Python Cognee 相同的 topoteretes 组织。本轮只读克隆到 `/tmp/codepet-cognee-rs-audit`，检查主分支提交 `c3165875027bfff353f407d031b60fecad0c8b96`，并单独读取 `v0.2.0` 标签。许可证 MIT / Apache-2.0 双许可。

已经确认是实际加工框架：

- `crates/cognify/src/tasks.rs` 串联分类、切块、抽取、摘要和索引；`fact_extraction/extractor.rs` 的 `FactExtractor` 调用 LLM 的结构化输出接口生成节点和边，不要求业务方提供抽取实现。
- `crates/cognify/src/graph_integration/` 有归并与数据库去重逻辑，另有边来源记录。`crates/embedding/src/openai_compatible.rs` 支持模型、key 和自定义 endpoint，调用 `/v1/embeddings`；可接外部模型服务，无须内置模型。
- 默认本地运行组合为 SQLite、Ladybug、LanceDB，没有 PostgreSQL 或 Python 运行时要求。Rust 库、CLI、C 和 Node 绑定均有实现。CodePet 的 Rust Host 可以优先评估直接依赖，避免为绑定额外携带 Node。
- 主分支 `crates/lib/src/api/update.rs` 实现清除旧记忆、按原 `data_id` 重加内容、重新 cognify；并非只声明更新接口。但过程不是已证实的跨存储原子发布，源码明确描述失败后可能暂时没有记忆，之后重建。
- `crates/search/` 提供向量、图和 `CHUNKS_LEXICAL` BM25。`hybrid/mod.rs` 的 hybrid 合并块、实体和事实，**没有自动把 BM25 也合入该路径**。`lexical_retriever.rs` 每次从图读取块、按指纹缓存内存语料统计，并非持久化倒排全文索引。按 `\w+` 分词，中文效果必须单独验证。

可视化方面，主分支 [visualization README](https://github.com/topoteretes/cognee-rs/blob/main/crates/visualization/README.md) 与源码确认：它包含预处理器及原 Cognee 前端资产，可输出离线 HTML。Graph、Schema、Memory 有实现；**Semantic 的投影坐标与聚类未实现，显示空状态**。资产记录上游提交 `685de12d2c8cd9bd2e56be92095dbf4347c8f191`。这是项目已有移植，不是本次数据库实验的适配层，也不是使用现有 task-run 证明了新框架的加工效果。

成熟度与分发限制：

- 官方贡献说明把与 Python 90%+ 行为一致作为目标，不能当作已达到的验收结果。`crates/delete/src/lib.rs` 仍注明 soft delete 的孤立实体清理与 Python 有差距，必须验证更新后的残留与多文档共享实体。
- 最新 GitHub release 是 2026-07-30 的 `v0.2.0`，主分支已明显推进。标签中的更新会删旧文档再添加，不能套用主分支“保留 data_id”的说明；标签可视化也是较早实现。验证必须固定标签或提交，不能混用两者的能力和包体积。
- registry 的 `@cognee/neon-darwin-arm64@0.2.0` 解包体积为 256,274,020 bytes；Windows x64 包为 270,597,439 bytes。GitHub 的 macOS arm64 C API 压缩资产为 210,182,385 bytes。它们是不同分发物，不是 CodePet 最终安装增量，也没有证明 Rust 直接集成同样大。
- 默认 features 带 ONNX、文档加载器、AWS 与 PostgreSQL 适配等。源码有 feature 开关，但只保留外部模型与所需存储后的构建体积尚未测量，不能承诺会很小。
- Node 预构建矩阵覆盖 macOS arm64、Linux x64/arm64、Windows x64；没有 Intel macOS 预构建。自编译外部模型配置下的支持需要另测。

### EdgeQuake：完整加工，服务部署成本较高

[官方架构](https://github.com/raphaelmansuy/edgequake/blob/edgequake-main/docs/architecture/overview.md) 确认它实现 Rust LightRAG 文档加工和检索，支持外部模型，并有图界面。生产路径要求 PostgreSQL + pgvector + Apache AGE；内存实现用于测试，不能用它声称生产可完全嵌入。

用户已将其列为备选。`postgresql-embedded` 可管理本地 PostgreSQL 子进程，但 AGE / pgvector 的平台打包、版本升级和完整资源占用尚未验证，尤其不能从 PostgreSQL 支持 Windows 推断所有扩展均可直接分发。

### 其他已检查方案

| 项目 | 检查结果 | 当前判断 |
| --- | --- | --- |
| [GraphRAG-rs](https://github.com/automataIA/graphrag-rs) | Rust，有真实 LLM 实体关系抽取及 async 模型路径；默认快速开始使用规则与 hash。`graph/incremental/manager.rs` 的 entity / relationship / embedding 批量更新处理函数仍是空实现 | 不是单纯数据库，但不能据高级功能名称承诺完整更新生命周期；暂不优先于 cognee-rs |
| [CortexDB](https://github.com/liliang-cn/cortexdb) | Go，不止数据库，包含 graphflow LLM 抽取流水线；但所查 `graphflow/build.go` 对图节点用 `deterministicVector`，没有接语义 embedding；LLM 提供者通过接口与示例适配 | 所查加工路径不能作为完整 Cognee 替代；不把数据库三类索引当成加工闭环 |
| [mnemo](https://github.com/zaydmulani09/mnemo) | Rust，有文本抽取、实体别名归并和图遍历；公开检索流程主要为词法、实体匹配、BFS | 未确认满足必需的语义向量检索与文档替换闭环 |
| [go-light-rag](https://github.com/MegaGrindStone/go-light-rag) | Go，含抽取组件，但 README 明确提供核心组件而非端到端系统，现有图存储依赖 Neo4j | 不符合当前现成客户端加工框架的要求 |
| [Memvid](https://github.com/memvid/memvid) | Rust，文本与向量记忆索引 | 未确认包含所需自动实体关系抽取与图加工，不能作为完整替代推荐 |

另检查 TypeGraph、BrowseGraph、Mastra 及早期 Alchemy；存储建模库、特定浏览器产品、通用 Agent 工作流或尚未验证的早期流水线，不列为已合格替代。没有据此断言所有非 Python 框架都不存在。

## 影响范围

- 业务账本和 `codepet-task-lineage`：仍提供 Run 原始快照及稳定来源标识，需要验证其与框架 document id 的对应。
- Rust Host 与打包：决定直接链接还是 sidecar，验证 feature 裁剪、关闭模型自动下载后的启动及平台构建。
- MCP recall：需要输出 Run 与来源；框架的通用回答不能直接等同于产品需要的 Run 排名。
- 图 UI：优先验证框架原生输出，确认主分支已有预处理能覆盖当前喜欢的视图，不再以复用旧图展示冒充新加工。

## 风险与下一轮验证

1. 固定 cognee-rs 提交，使用已有 9 个 Run 的**原始文档**和相同模型，从空库加工。记录真实模型请求、耗时、图与来源；禁止导入 Cognee 已抽取节点作为替代证明。
2. 改写一个 Run，删除旧词与旧关系，保留另一 Run 的共享实体；重启后重复召回。增加模型失败、进程中断及重复提交，核对残留和恢复。
3. 分别运行向量、图、词法，再查看框架现有融合路径；测中文与跨 workspace、时间偏好、负例拒绝及 Run 归属。若必须自研核心融合或索引生命周期，明确列出缺口，不宣称零加工实现。
4. 渲染新加工出的图，检查 Graph / Schema / Memory；Semantic 标为未实现，不能带入旧库坐标制造完整效果。
5. 构建仅外部模型所需的 release，测实际包体积、冷启动、RSS 与文件依赖；按目标平台实际构建，区分源码可编译、预构建可用和正式签名分发。

## 验证结果与未知项

本轮只进行官方文档、仓库源码、release / npm 元数据核查；没有执行新框架、模型调用或跨平台构建。原始证据在忽略目录 `artifacts/cognee-task-recall-qa/framework-search/`；cognee-rs 源码核查提交见上。

完整 Python 特性一致性、中文召回质量、更新残留、最小构建体积、现有实验库直接兼容性均未知。当前排序仅说明 cognee-rs 最值得先做原始数据验证，不意味着已经找到成熟无缺口的完整替代。
