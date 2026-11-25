# cognee-rs / Python Cognee 对照

独立实验，不接正式 MCP 或修改业务账本。`driver.rs` 只装配 cognee-rs 的公开 API；抽取、embedding、图写入、更新与检索均由框架执行。Python 仅作为实验调度器，不属于 Rust 分发包。

## 固定条件

- cognee-rs：`c3165875027bfff353f407d031b60fecad0c8b96`，从主分支源码构建；不是已发布 0.2.0 二进制的能力验证。
- Python Cognee：1.6.10，复用此前隔离环境。
- 数据：此前历史输入的 9 个 Run，双方使用相同 `documents.json` 原文。
- LLM：`ddalcu/Qwen3.8-27B-MLX-Serve-4bit`。
- embedding：`Qwen3-Embedding-0.6B-8bit`，1024 维。
- 服务：用户已运行的 `http://127.0.0.1:11234/v1`，无模型下载或云端模型计费。
- 抽取提示词：上级 `development_graph_prompt.txt`。Rust 与 Python 保留自身的分块、结构化输出封装及摘要行为，所以不能把差异只归因于实现语言。

## 构建

需要 Rust、C/C++ 编译器、CMake 与 protoc。本轮 macOS arm64 使用 Rust stable 1.98.1，禁用默认 features、ONNX 和遥测，保留持久化存储与原生可视化。构建目录应放在仓库忽略的 artifacts 内。

```sh
git clone https://github.com/topoteretes/cognee-rs.git /tmp/codepet-cognee-rs-audit
git -C /tmp/codepet-cognee-rs-audit checkout c3165875027bfff353f407d031b60fecad0c8b96
```

将 `driver.rs` 复制进该仓库的 `crates/cli/examples/codepet_probe.rs`，并在该包 `Cargo.toml` 追加：

```toml
[[bin]]
name = "codepet-probe"
path = "examples/codepet_probe.rs"
```

在上游仓库运行，替换 `CARGO_TARGET_DIR` 为本机工作区的绝对路径：

```sh
CARGO_TARGET_DIR=<绝对证据目录>/target CARGO_BUILD_JOBS=6 CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_PROFILE_RELEASE_STRIP=symbols cargo +stable build --release -p cognee-cli --bin cognee-cli --bin codepet-probe --no-default-features --features ladybug,lancedb,sqlite,tiktoken,visualization
```

使用 `bin` 而非 `example` 构建驱动，避免 dev-dependencies 的 feature 合并引起整套依赖重编译。未修改上游加工源码。保留 Cargo.lock 和构建日志作为证据。

## 执行

先按上级 runbook 从空库重跑 Python，指定 `--backend mlx --input-mode history --snapshot-from` 为固定快照。其成功输出目录作为下面的 `PYTHON_RUN`。

```sh
python3 prototypes/cognee-task-recall/rust/run.py <证据目录>/target/release/codepet-probe <PYTHON_RUN> <证据目录>/rust
artifacts/cognee-task-recall-qa/.venv/bin/python prototypes/cognee-task-recall/rust/python_extra.py <PYTHON_RUN>
python3 prototypes/cognee-task-recall/rust/rust_extra.py <证据目录>/target/release/codepet-probe <证据目录>/rust
artifacts/cognee-task-recall-qa/.venv/bin/python prototypes/cognee-task-recall/rust/audit_vectors.py <证据目录>/rust
python3 prototypes/cognee-task-recall/rust/measure.py <证据目录>/target /tmp/codepet-cognee-rs-audit <证据目录>
python3 prototypes/cognee-task-recall/rust/compare.py <PYTHON_RUN> <证据目录>/rust <证据目录>
```

Rust 全部模型请求通过已有 `AuditedGateway` 记录；与 Python 一样，将 JSON object 解码改为提示词与框架结构校验，避免该 MLX 版本的约束解码性能问题。Rust 使用上游 `LLM_STRUCTURED_OUTPUT_MODE=json`，仍由框架验证，未修补模型返回数据。摘要保留 Rust 默认流程，Python 的既有配置使用 `from_extraction`，模型调用数和耗时需要分别看。

`run.py` 的输出目录必须是新的空目录；`--resume` 只允许在同一文档已建库且尚未开始更新时继续评估，保留首次失败回执，不再计一次新抽取。`operations/` 记录公开 API 输入与返回，`model-calls/` 保存实际请求回执。受控更新只改复制的侧栏 Run，随后恢复。原生 hybrid 在恢复并重启后补测，和初次建库的三路结果分开记录。

公开 search 编排器默认不返回 BM25 分数。驱动与 Python 对照一样，直接调用框架原生 BM25 retriever 并打开分数，才能排除零分 top-k 填充。`rust_extra.py` 保留首次评分回执，并核对恢复后的文本块与基线逐字一致。`audit_vectors.py` 只读 Lance 历史版本，确认复用块 ID 时向量值实际改变；没有删除 ID 不能被误报为清理失败。

## 结果边界

- 7 个正例与 1 个负例只用于冒烟检查，无关查询尚未校准拒绝阈值。
- 更新中的共享控制实体在同一文档前后出现，不等于验证了多文档共同持有实体时的删除安全。
- 体积只统计当前 macOS arm64 release 可运行文件、动态依赖和含许可证的 tar.gz；不含模型、数据库、编译缓存或 Python 调度器，不是正式签名 App 安装包。
- `--version` 启动耗时不代表数据库预热或召回延迟。
- 两个原生图均来自真实框架数据库；统计取初次建库，展示取更新后恢复原文的状态，模型重新抽取可能改变少量实体。
- Rust 原生 Semantic 投影视图尚未实现，保留空状态，不从 Python 拷贝坐标。

正式结论与回执位置写入 `knowledge/40-runbooks/cognee-rs-task-run-comparison.md`。
