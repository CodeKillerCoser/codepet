# TaskRun 多路召回与 Host Model Gateway

## 背景

Task 是长期主题，实际需要恢复的是具体 Run。单纯按标题子串查 Task 容易导致 Agent 重复建主题；让 Agent 负责图实体和文档整理也会增加工具调用和时间线噪音。用户选择保留 Python Cognee 的完整加工能力，以独立包运行，不随客户端分发模型权重。

## 目标

- `find-task-runs` 返回真实 Run，并合并语义、图、中文全文检索，跨 workspace 可见。
- 每条结果同时携带 `taskSummary`，展示主题的聚合状态与人工完成锁；Run 自身状态独立保留。汇总从权威账本即时读取，不加入文档哈希，避免主题状态变化触发无关执行的重新抽取。
- Rust 自动整理已完成执行、启动与管理 sidecar；Agent 仍只提交任务、里程碑和结果。
- Host 统一保存模型配置、转发与鉴权、统计实际用量；支持本机 MLX 和兼容 OpenAI 的云端 API。

## 非目标

不更换权威 Task/Run 账本，不让 Agent 提交实体或 embedding。不把 Ladybug 当作加工框架，不实现自己的实体抽取管线。不内置模型、自动下载模型或依赖终端的 Python。当前 Gateway 提供 chat completions 与 embeddings 的非流式 JSON 接口。

## 现状理解

权威记录仍在 `tasks/v4` 的 Task 聚合 JSON 中；每个 Run 有自己的工作区、验证记录和历史。Cognee 1.6.10 使用 SQLite、LanceDB 与 Ladybug。主界面“知识图谱”接受 Cognee 原生可视化 JSON；渲染层仅展示框架输出，不承担加工。原生 App 和独立 stdio MCP 可能同时访问相同数据目录。

## 实现路径

`finish-task-run` 成功写入后发起异步同步；App/stdio 的 Rust watcher 每 10 秒补扫已有记录与人工修改、删除。同步只把 ready_for_delivery、published、closed 状态的执行送入加工队列，开发中的 Run 仅建立全文索引。查询前也同步一次。模型调用发生在账本锁释放之后，完成工具不等待抽取。

Rust 生成每 Run 一份稳定 JSON 文档，包含 Task 背景、Run 目标/方案/结论、真实 verification records、预期验收条件、有效时间线和 closure。Git 自动观测、聚合 revision 等易变数据不进入内容哈希。预期条件与验证结果明确区分，Agent 自测不等于用户验收。结构化原文与索引版本存入 sidecar catalog，权威修改仍走账本。

Python catalog 保存 desired/indexed hash、Cognee data_id、pending/processing/ready/failed/draft 和重试时间。相同哈希不重复调用模型；进程中断恢复 processing；失败退避重试；替换过程中暂不提供旧版语义结果。加工期间又有新版时，旧任务的完成回执不能发布成 ready。删除立即移除全文可见性，再清理 Cognee 文档与图。模型配置更换使用独立目录，避免不同 embedding 维度混存；旧配置索引暂不自动回收。

Cognee 原生 remember/update/delete 完成分块、LLM 抽取、向量和图写入。语义路使用 CHUNKS，图路使用 GRAPH_COMPLETION 的 only_context，避免额外回答生成；全文路使用 SQLite FTS5，对中文建立二元词组。三路按 reciprocal rank 加权，Rust 再加 workspace 与时间软偏好。偏好不能单独制造匹配结果。

来源映射只接受 native chunk/document 的 runId、contentHash，以及 chunk 到 entity/summary 的结构关系。不能扫描正文中的 UUID 来推测归属，不能沿任意语义边传播所有权。Rust 在返回前重新读取权威快照，丢弃删除、未知与过期哈希结果。结果携带路由、排序证据、加工状态；模型离线、进程失败时退回账本子串结果，并明确降级状态。加工繁忙时仍立即返回全文结果。

每数据目录只允许一个 Python owner，由 OS 文件锁约束；其他 Host 附着已有服务。服务只绑定回环随机端口，验证 Host、Origin 和随机 Bearer token。owner 由 Rust 保持 stdin，父进程退出时 Python 退出；Rust 显式停止或崩溃时终止子进程。跨 Host 的同步锁保证旧快照不会覆盖新快照。索引独立于前端窗口。

Model Gateway 是独立 Rust 模块，由 `codepet-host::model_gateway` 暴露，同时供独立任务 MCP 使用。模型请求只允许配置好的两条路由，强制配置中的模型名，禁用重定向，reqwest 显式 `no_proxy()`；不转发调用者的鉴权或任意上游 URL。Gateway 和 sidecar 均要求临时 token，拒绝浏览器 Origin 与错误 Host。sidecar 清理继承环境，以 stdin 获取 Gateway 凭证，不接触配置中的上游密钥。

设置页“模型配置”提供两套 endpoint/model/key、embedding dimensions、本机 JSON 兼容开关与少量高级参数。配置保存在 `model-gateway/config.json`，主窗口 IPC 可修改，MCP 不提供设置工具。返回前清空 key 并标记 hasApiKey；相同 endpoint 下空密码保留旧值；更换地址不继承密钥。Unix 新文件采用 tempfile 的私有权限，密钥未做 OS keychain 加密。用量保存到独立 SQLite，按用途/模型累计实际上游请求、成功数、input/output tokens、是否报告和耗时；未知 token 不估算，未计算价格。

分发包包含 uv standalone CPython 3.12.14、锁定的 Cognee core 依赖、源码和 tokenizer 词表缓存，保留第三方许可证；无模型权重、无启动时 pip/uv。每个平台原生构建，拒绝伪交叉打包。macOS 先签内部 Mach-O，Python 单独附加扩展库加载 entitlement，再记录哈希、压缩。App 仅携带压缩资源，避免外层签名改写已记录哈希。Rust 校验压缩包和各文件、拒绝链接/特殊条目及越界路径，原子安装到 `~/.codepet/tools/task-recall/versions`。稳定 stdio 入口记录独立目录，移动 App 不影响它。

## 涉及模块

- `sidecars/task-recall`：框架加工、持久队列、全文索引、原生图导出、独立包构建。
- `crates/codepet-model-gateway`、Host re-export：两模型转发、配置隔离、用量与生命周期。
- `crates/codepet-task-mcp/src/recall`、HTTP runtime、managed：业务快照、进程复用、融合、安装与降级。
- `src-tauri/src/task_ledger.rs`、模型设置页面：只允许主窗口配置，不新增 Agent 心智负担。
- `frontend/lib/TaskKnowledgePage.svelte`、`public/cognee-view`：主界面原生图预览，离线资源与独立沙箱；兼容处理见领域预览文档。
- 资源 staging 与 CI：本机构建 Python 包后随 App 分发。

## 风险

- Python 依赖体积大：量取实际原生包与压缩包，不用虚拟环境大小代替分发大小；后续裁剪须再跑全管线。
- 索引更新失败或乱序：catalog 重启/重试/新版覆盖测试、原生更新/删除验证、返回前哈希校验。
- 多客户端和孤儿进程：真实进程的 singleton、stdin EOF、退出/重连测试。
- 鉴权与代理：伪造 token、Host/Origin、代理环境、模型覆盖与缺失用量测试。
- 签名/平台差异：签名后运行真实 Cognee，原生 Windows CI 构建；macOS 证书发布与 Windows 完整安装器仍需各平台验收。

## 测试计划

运行 Python catalog/service 测试、Gateway 与账本/MCP Rust 测试、真实 stdio/HTTP 黑盒。以固定 9 Run 文档和用户本机两模型验证三路来源、无变化同步、update data_id 保持和旧 chunk 删除。搬移已安装包，在清理 PATH/PYTHONPATH/PYTHONHOME 的环境下启动。前端组件测试与 820×600 浏览器界面验证覆盖键盘焦点、保存、失败恢复和溢出。具体回执见配套 runbook。

## 知识沉淀

实现、启动参数、实际体积与平台验证状态维护在 `knowledge/40-runbooks/task-run-recall-sidecar.md`；本文件记录业务边界与数据流，不把实验指标当质量承诺。

## 未知项

云端供应商的逐模型计费规则未接入。向量阈值、跨语言召回质量及大规模数据耗时尚未建立生产基线。旧配置索引的自动回收、Windows 运行验证及证书签名安装器需要单独记录结果。
