# cognee-rs 与 Python Cognee 的 TaskRun 对照

## 背景与范围

2026-11-25 本机 macOS arm64 实验。用户要求用现有 TaskRun 原文比较抽取、更新、召回、原生图，并实测不内置模型时的分发体积。正式 MCP、业务账本、finish 事件与 App 集成均未修改。

**结论：cognee-rs 已经独立跑通加工闭环，不需要 CodePet 重写抽取。仍不能当成 Python Cognee 的完整等价替代。** 本轮发现重复 update 会重算、Semantic 投影未实现；词法能力是内存 BM25，不是持久化全文倒排索引。共享控制实体未抽出，ID 保留这一项没有完成有效验证。

## 固定输入与环境

- Python Cognee 1.6.10；cognee-rs 固定主分支提交 `c3165875027bfff353f407d031b60fecad0c8b96`。没有用发布版 0.2.0 的包冒充主分支能力。
- 双方从空库读取同一份 9 个 Run 历史文档快照：此前 `runs/20261124T195156701196Z` 的业务记录，保留有效进度，省略完整对话与代码。该历史快照未包含结构化验证 records，且其中一个 Run 当时尚在进行中，不能称为正式 finish 集成测试。
- 同一 `development_graph_prompt.txt`，同一用户已安装的 MLX LLM `ddalcu/Qwen3.8-27B-MLX-Serve-4bit` 和 embedding `Qwen3-Embedding-0.6B-8bit`，1024 维。模型服务为本机 `127.0.0.1:11234`，本轮没有下载模型权重或调用付费云端模型。
- 7 个检索正例和 1 个财务付款负例，目标 Run 在检索前标注；workspace 仅在既有固定融合中作为软偏好。
- 两侧均使用原生 SQLite、Ladybug、LanceDB。模型由外部 API 提供。Rust 保留默认独立摘要阶段，Python 沿用 `SUMMARY_METHOD=from_extraction`；分块和结构化输出封装也不同，因此不是仅比较实现语言的性能基准。

证据根目录：`artifacts/cognee-task-recall-qa/cognee-rs-comparison/`。Python 新运行在其 `python/runs/20261124T214704071309Z/`；Rust 在 `rust/`。可查看 `index.html`、`comparison.json`、两侧 `report.json`、`model-calls/` 和 Rust `operations/`。这些大文件保持忽略，不提交业务数据或模型权重。

## 抽取与召回

| 指标 | Python Cognee | cognee-rs |
| --- | --- | --- |
| Run / 文本块 | 9 / 11 | 9 / 9 |
| 实体 / 语义关系 | 111 / 78 | 71 / 53 |
| 总节点 / 总关系 | 147 / 325 | 104 / 215 |
| 入库关系逐字证据缺失或不匹配 | 1 / 78 | 0 / 53 |
| 向量 Hit@1 / Hit@3 | 6/7 · 7/7 | 6/7 · 7/7 |
| 原生 BM25 Hit@1 / Hit@3 | 1/7 · 2/7 | 1/7 · 2/7 |
| 图上下文 Hit@1 / Hit@3 | 6/7 · 7/7 | 6/7 · 7/7 |
| 原生 hybrid Hit@1 / Hit@3 | 7/7 · 7/7 | 6/7 · 7/7 |
| 固定权重融合 Hit@1 / Hit@3 | 6/7 · 7/7 | 6/7 · 7/7 |

Rust 的 9 个块分别逐字包含完整的 9 份文档，没有因块数更少截掉原文。两侧实体名直接交集仅 23 个，既有命名粒度差异也有抽取范围差异，不能用“节点更少”推断归并更好或覆盖更差。

逐字证据检查不验证事实蕴含或人工验收。两侧仍生成提示词未允许的 `reported` 状态，Python 有 4 条、Rust 有 6 条；提示词不是硬 schema 约束。无关财务查询仍返回向量、图或 hybrid 候选，未实现拒绝阈值。

三路检索通过 `only_context` 获取对象与来源，不调用回答 LLM。原生 hybrid 与自定义融合分列；hybrid 在受控更新并恢复、重启后补测。BM25 两侧均调用框架原生 retriever 并打开分数，再排除零分 top-k 填充，未替换中文分词器。

## 更新与重启

只在复制的侧栏 Run 上追加控制标记，并将明确宽度语句从 1/2 改成 3/5、ProbeOldLibrary 改成 ProbeNewLibrary，最后恢复原文。两侧实际调用自己的 update API。

| 行为 | Python | Rust |
| --- | --- | --- |
| 文档 ID 稳定，原文与图内容更新 | 通过 | 通过 |
| 旧实体移除、新实体存在 | 通过 | 通过；按框架小写名称核对 |
| 向量更新 | 删除旧块向量、保留未改块向量 | 复用原块 ID，实际替换文本与向量 |
| 共享控制实体 ID 保留 | 通过 | 未验证：未抽出 ProbeSharedLibrary |
| 相同内容再次 update | unchanged，约 0.087 秒 | 再加工，约 24.68 秒 |
| 恢复原文并重启 | 文档、块与向量读回通过 | 文档、所有节点 ID 与块向量读回通过 |

Rust 的块 ID 在前后版本相同，因此“没有被删除的块 ID”不是删除失败。`vector-version-audit.json` 通过只读 Lance 历史版本核对：同一块在旧、新版本具有不同文本 hash 和向量 hash，1024 维不变，前后向量 cosine 约 0.9988785。没有在审计中重写或重新生成这些向量。恢复后 Entity / EdgeType 索引中没有发现 ProbeOldLibrary 或 ProbeNewLibrary 控制内容。

Rust 此次变更约 39.25 秒，Python 约 30.28 秒；初次导入分别约 406.39 秒、481.82 秒。这些是一次真实观测，存在摘要策略、分块、模型重试和编译并行等差异，不用于宣称语言或框架更快。整个流程的模型 HTTP 请求均没有最终接口错误，但模型可能返回不合 schema 的 JSON，并触发框架校验重试。

本次共享控制实体前后出现于同一个文档；即使 Python 通过，也不能外推为多文档共同持有实体的删除安全。并发更新、中断恢复和大规模历史版本清理仍需单独验证。

## 原生可视化

两侧均从自己真实加工的数据库导出，没有复用前序 Python 图作为 Rust 加工结果。

- Rust Graph、Schema、Memory 页面可渲染；模板与之前喜欢的 Cognee 风格一致。本轮实际检查了 Schema 的类型卡片、关系与节点数量。
- Python Semantic 显示 9 个聚类；Rust 显示空状态。Rust 缺的是投影与聚类实现，不是本轮没有生成 embedding；向量索引、检索和重启读回已经验证。
- 抽取统计取初次建库；对比页嵌入的原生图均在控制更新后恢复原文时导出。再次抽取会改变少量节点或名称，图内计数可能不同于初次统计。
- 对比页提供框架切换、原文、实体与关系、查询、更新回执、两侧原生图以及 JSON 证据链接。原生图可以单独打开。窄屏对比页没有横向溢出；原生图自身的桌面交互不等于已经适配 CodePet 移动布局。

截图和 UI 回执位于同一证据目录：`comparison-desktop.png`、`comparison-mobile.png`、`python-schema.png`、`rust-schema.png`、`python-semantic.png`。

## 不内置模型的实际体积

Rust 1.98.1 release，`--no-default-features`，只启用 `ladybug,lancedb,sqlite,tiktoken,visualization`，`debug=0`，`strip=symbols`。CMake/protoc 为构建工具，不进入分发包。保留 Cargo.lock 及构建日志。

| 分发物 | 实际大小 |
| --- | --- |
| 上游官方 CLI 可执行文件 | 192,410,480 bytes，183.50 MiB |
| CLI 单文件 gzip | 69,779,935 bytes，66.55 MiB |
| CLI + MIT/Apache 许可证 tar.gz | 69,785,979 bytes，66.55 MiB |
| 实验 JSON-lines 驱动 | 约 181 MiB，精确值见最终 measurement |

`cargo-tree.txt` 确认没有 `ort` / `ort-sys`；`otool -L` 只有 macOS 系统库，没有需另带的非系统动态库。未包含 MLX、模型权重、Python 调度器、运行数据库或编译缓存。Rust 驱动加工与检索期间子进程峰值 RSS 约 281 MiB，不含外部 MLX 进程。

这是真实可运行的当前平台产物，不是签名 App 安装包，也没有测 Windows / Intel macOS。直接链接进现有 Rust Host 的安装增量可能不同。默认嵌入式存储链仍较重，不能把“Rust”直接等同于“几 MB”。本轮未进一步做 LTO 或替换存储来人为压小体积。

## 实现与复跑

驱动和报告位于 `prototypes/cognee-task-recall/rust/`，步骤见该目录 README：

- `driver.rs`：装配公开 remember/update/search、原生 BM25 和 visualize API，没有实现实体抽取或图构建。
- `run.py`：调度固定数据、受控更新与重启。模型请求通过既有 `AuditedGateway`；不修改生产账本。
- `python_extra.py`、`rust_extra.py`：补测原生 hybrid、显式 BM25 分数并核对来源。
- `audit_vectors.py`：只读历史索引，区分“复用 ID 更新”与“删除旧 ID”。它需要实验 Python 环境，只属于审计，不进入分发包。
- `compare.py`、`report.html`、`measure.py`：生成证据页和实际分发体积回执。

本轮评估脚本曾把原生 hybrid 传给只有三路权重的旧融合函数，触发 KeyError；建库已完成且未开始更新，所以修正后显式 resume 评估，没有重新抽取。首次错误保存在 `evaluation-attempt-1.json`。另修正公开 lexical 接口不返回分数、实体名小写和复用块 ID 的评估口径，保留 `report-before-score-recheck.json` 与原始 operations。不能把这些调度问题归因于框架。

## 已运行验证与后续边界

已完成真实 release 构建、9 文档加工、8 查询、控制更新与恢复、重启读回、历史向量版本审计、原生图和对比页浏览器检查。`python3 -m unittest discover -s prototypes/cognee-task-recall -p 'test_*.py'` 17 项通过；包括新增的融合路线分离和实体名称大小写回归检查。`npm run check:source-size`、Python 编译检查和 diff 空白检查通过。

下一步若进入接入验证，主要风险与检查路径是：

1. 加工幂等与成本：在 CodePet 提交版本前做内容 hash 去重，再验证相同 finish 事件重投不会触发 update 重建。
2. 事实质量与来源：扩大中文标注样本，硬校验状态枚举与引用，并验证 Run 来源映射；不能以此次正例通过作为上线准确率。
3. 更新生命周期：增加多文档共享实体、删除、并发、中断和旧版本清理检查，关注 soft delete 的已知实现差距。
4. 全文与 UI：分别评估持久化中文全文需求及 Semantic 视图缺口，不能把已有内存 BM25 或空页面计为满足全部要求。
5. 分发：在目标平台构建签名产物，测 Rust Host 链接增量、冷启动与资源；当前结果只证明 macOS 原型。

本轮不改变生产选型。cognee-rs 可以继续作为首要候选，PostgreSQL + EdgeQuake 仍保留备选。
