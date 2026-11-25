# TaskRun Cognee sidecar 构建、配置与验证

## 范围与证据

2026-11-25，macOS arm64 本机验证。加工引擎是 Python Cognee 1.6.10；Rust Host 提供 Model Gateway 与 sidecar 管理。权威账本、正式用户模型配置和已安装 App 未被本轮测试修改。测试数据均写入隔离目录。

固定 9 Run 历史文档作为框架样本（测试主动标记 eligible，不受原始状态限制），首次加工耗时 516.17 秒，全部 ready。抽屉查询 `490b7f1e-7e6b-45bc-9445-6f362f765f67` 在 semantic、graph、fulltext 三路均 rank 1；原生图回执含 162 节点、371 边，11 chunks、9 documents、126 entities。它是框架回归样本，不代表已对正式账本完成后台回填，也不是完整召回质量基线。

生产 Rust 快照新增了实际 verification records。另用真实 MCP 进程创建隔离样例、调用 finish、等待 Rust 自动加工，再通过 `find-task-runs` 查询：三路命中同一个真实 Run，原生图 12 节点。该样例的验证证据明确标记为隔离样例，未作为业务问题验收。

## 构建

构建机需要 Rust、Node、uv；用户安装后的运行环境不需要 Python、uv 或模型权重随包安装。

```sh
uv run --no-project --python 3.12 sidecars/task-recall/build.py \
  --output artifacts/task-recall-bundle/native \
  --stage src-tauri/resources/task-recall
```

`--target` 必须与构建宿主一致。依赖按 `sidecars/task-recall/uv.lock` 和 hashes 安装；langdetect 的锁定源码在构建机制作 wheel，其余要求预编译 wheel。原生包保留依赖许可证和分发 metadata。

完整 staging 已接入 `scripts/stage_provider_plugins.mjs`，随 `providers:stage` / `build:bundle` 执行；macOS 和 Windows CI 安装 uv。首次构建较慢，源码、lock、目标和签名身份相同时复用缓存。

实际 macOS arm64 包约 377 MiB gzip，展开约 1.10 GiB（具体字节数见构建生成的 bundle.json）；数字来自真实签名包，不含模型权重。App 资源只保存 `bundle.json` 和 `bundle.tar.gz`，完整开发目录在 artifacts，不要提交生成包。

macOS 内部 Mach-O 先签名再生成 manifest。Python 解释器单独附加 `com.apple.security.cs.disable-library-validation`，以加载独立签名的扩展；不是给整个 App 添加该 entitlement。开发签名验证曾发现 Team ID 加载错误，修正后签名包通过真实 Cognee 查询/更新/删除。正式证书签名与公证仍需发布流水线验证。Cognee cache proxy 的签名保留兼容补丁与原始/修改后哈希记录在包内 cognee-patches.json，升级时必须复核。

## 运行与安装路径

App 的 managed 安装会校验压缩包与文件哈希，解压并原子切换到 `~/.codepet/tools/task-recall/versions/<archive-hash>`。stdio `current.json` 保存该独立目录；旧 bootstrap 不识别新增字段时，新 runner 也可读取它。App 移动不改变固定入口。首次解压在后台；本机 debug 安装验证约 82 秒，尚未量取 release 安装耗时，不把它当作生产耗时保证。

开发时默认查找 `artifacts/task-recall-bundle/native`。可用绝对路径 `CODEPET_TASK_RECALL_BUNDLE` 指定已展开包。入口是包内 CPython 的 `-I -B -X utf8 main.py --data-dir <absolute>`；敏感启动配置走 stdin，不放命令行。Rust 清理继承环境，设置最小 PATH、loopback no-proxy、数值库 4 线程及 AWS_EC2_METADATA_DISABLED=true，避免桌面端探测云环境元数据。退出时终止 owner；另一个 Host 可附着同一数据目录的服务。

持久数据：

- `model-gateway/config.json`：两模型设置及上游密钥；主窗口管理、读回脱敏，未使用 OS keychain 加密。
- `model-gateway/usage.sqlite`：上游请求与实际 token 统计，不保存 prompt、响应正文或密钥。
- `tasks/recall/runtime.json`：私有临时端口与鉴权信息，禁止贴进日志或报告。
- `tasks/recall/catalogs/<config-fingerprint>/catalog.sqlite`：结构化文档、FTS、版本和队列。
- `tasks/recall/indexes/<config-fingerprint>/`：Cognee SQLite/LanceDB/Ladybug 派生索引。

## 模型配置

重建 App 后进入“设置 → 模型配置”，填写两套 API base URL、模型名、API Key 与 embedding dimensions，然后启用并保存。默认不开启模型加工。相同 endpoint 下空密码保留已保存 key；更换 endpoint 时须重新填写，避免旧密钥发给新服务。本机服务可无 key。默认支持非流式 OpenAI-compatible chat completions / embeddings，云端 URL 要求 HTTPS；HTTP 只接受 loopback。

本机已验证：

- API base：`http://127.0.0.1:11234/v1`
- LLM：`ddalcu/Qwen3.8-27B-MLX-Serve-4bit`
- Embedding：`Qwen3-Embedding-0.6B-8bit`，1024 维
- 本机兼容开关 plainJson 开启；高级参数 `{"chat_template_kwargs":{"enable_thinking":false}}`

plainJson 只去掉 json_object 的传输约束，schema 仍在 Cognee 的提示与验证中。云端默认关闭该选项。更换模型配置会使用新索引并回填，旧索引暂留。未开启或加工繁忙时全文仍可用；服务未安装或故障时 MCP 明确标记 substring 降级。

Gateway 和 sidecar 均只绑定 loopback，要求随机 Bearer token，拒绝错误 Host 与浏览器 Origin。Gateway 的 reqwest 显式 `no_proxy()` 且禁止重定向。密钥不传入 Python，Python 只拿临时 Gateway token。用量按实际上游请求计数，embedding 拆批会记为多个请求；缺失 usage 的 token 不估算，不显示推测费用。

## 自动化验证

```sh
PYTHONPATH=sidecars/task-recall uv run --no-project --python 3.12 \
  python -m unittest discover -s sidecars/task-recall/tests -v
cargo test --manifest-path crates/Cargo.toml \
  -p codepet-task-lineage -p codepet-task-mcp -p codepet-model-gateway -- --test-threads=1
node scripts/task_mcp_smoke.mjs
node scripts/task_mcp_http_smoke.mjs
node scripts/task_recall_smoke.mjs
npx vitest run frontend/lib/ModelSettings.test.ts frontend/lib/settings/model.test.ts
npm run check:frontend
npm run build
npm run providers:test
npm run check:source-size
```

最后一个 recall smoke 会调用上述本机模型，仅供显式集成验证，不应当默认 CI 无条件运行。测试会创建并清理隔离账本，不修改用户配置。

真实固定样本可复用 Rust ignored test：设置 `CODEPET_RECALL_PROBE_DOCUMENTS` 为既有 `documents.json`，`CODEPET_RECALL_PROBE_OUTPUT` 为隔离输出目录，运行 `cargo test --manifest-path crates/Cargo.toml -p codepet-task-mcp real_packaged_cognee -- --ignored --nocapture`。它检查三路实际来源、singleton、无变化同步无新增模型调用、update 保留 data_id、旧 chunk 移除、原生图删除与 owner 退出。更新阶段有明确标记的合成控制句，不是业务事实。

压缩包安装验证使用 `installs_real_archive` ignored test，设置 `CODEPET_RECALL_ARCHIVE_SOURCE` 和 `CODEPET_RECALL_ARCHIVE_WORKSPACE`。将结果 `installed-path.txt` 指给 `CODEPET_RECALL_TEST_BUNDLE` 后运行 `test_service.py`，覆盖清理 PYTHONPATH/PYTHONHOME/PATH、含空格/中文路径、鉴权、复用、EOF 与重启全文索引。

Tauri 检查沿用现有仓库方式：

```sh
TAURI_CONFIG='{"app":{"macOSPrivateApi":false},"bundle":{"resources":[]}}' \
  cargo check --manifest-path src-tauri/Cargo.toml --lib
```

## 已观察结果与限制

Python catalog/service 7 项（含模型初始化失败时保留全文）、Gateway 2 项、新增快照/融合检查、既有账本和 MCP 回归通过。签名包重复实际模型集成 113.41 秒通过（复用了已有 8 份索引，并恢复、替换和删除一份）；首次 9 份加工耗时应看前面的 516.17 秒。MCP 两种 transport 黑盒和真实模型端到端均通过。

浏览器模拟 IPC 验证了模型入口、820×600 无水平溢出、Tab 焦点、保存与错误恢复；模型设置组件 3 项测试通过。此界面验证不是实际 App 的模型保存 IPC 验收。Rust Tauri 编译通过，保留仓库原有 dead-code 警告。

真实 native semantic 预览回归 7.40 秒通过、未产生模型用量；`preview-graph.json` 包含 152 节点、347 关系及 152 个原生语义坐标。主界面四视图、820×600、空态、繁忙与错误恢复完成 Chromium 验证。

实际回执在 `artifacts/task-recall-sidecar-qa/`：`real/query.json`、`graph.json`、`update-report.json`、`mcp-recall-report.json` 与设置页截图。主界面左侧“任务图谱”展示原生 Graph、Schema、Memory、Semantic 视图。图与语义坐标均由 Cognee 导出，iframe 渲染层、异常状态与依赖兼容处理见 [预览说明](../30-domains/task-management/task-knowledge-preview.md)。加工历史与任务列表主动加工要求 backend API 38 和更新后的 sidecar；按次取消、重启恢复、幂等入队及模型用量归属见 [加工历史](../30-domains/task-management/task-processing-history.md)。真实 smoke 同时检查所有加工请求归属、召回查询不改变该次加工用量，以及主动重新加工保留原生文档 ID 并使用新加工记录计费。

## 2026-11-25 主干分发构建验证

[Build 45540988394](https://github.com/CodeKillerCoser/codepet/actions/runs/45540988394) 从主干 `bcdc2a7aacb6912284223bb59e28ad0348e4addd` 构建 `0.3.12-beta+bcdc2a7`，macOS ARM64 和 Windows x86_64 两个任务均成功；Windows 同时生成 updater 签名文件。模型权重未内置。

| 产物 | 实际字节数 |
| --- | ---: |
| macOS DMG | 452,920,196 |
| macOS sidecar 压缩资源 / 解压文件 | 396,428,329 / 1,188,760,141 |
| Windows EXE | 476,098,643 |
| Windows sidecar 压缩资源 / 解压文件 | 438,772,701 / 1,378,167,257 |

macOS 从 Actions 下载 DMG，核对 artifact SHA-256、`hdiutil verify`、App 版本和 `codesign --verify --deep --strict`；只读挂载后取出 sidecar，逐项验证 27,102 个文件哈希。使用包内独立 CPython 运行服务鉴权、singleton、EOF、重启全文检索测试，并用包内 MCP 可执行文件完成 12 次真实 stdio 工具调用。另在隔离目录实际加载 Cognee 1.6.10、Ladybug 和 LanceDB，图查询和向量读写通过，检查未调用模型。挂载和临时解压目录已清理。

同轮 [Android Release 45540620692](https://github.com/CodeKillerCoser/codepet-remote/actions/runs/45540620692) 构建 Remote `0.1.3+4`，APK 为 114,281,210 字节；517 项 Flutter 测试和 analyze 通过，`apksigner` 校验通过，正式证书 SHA-256 为 `cee49b08b2bf2e20b1b862c1953782e4709615420dd4c7b5b2e5ab9e040e2665`。

验证边界：Windows 已完成原生 CI 构建，本机未执行 Windows 安装或模型召回；macOS 未覆盖原生 App 的模型保存 IPC 与 GUI 完整验收。云端服务、全量用户数据规模和召回精度基线需要后续验证；当前 top-k 向量召回没有统一跨模型置信度阈值。
