# Cognee TaskRun 召回最小验证

## 背景与范围

为判断现有 TaskRun 能否作为后台知识加工的输入，新增独立 Python 原型。Task 是主题，文档和召回结果的业务粒度是 Run；workspace 只参与软排序，不能隔离其他工作区的相关执行。目标是验证实体抽取、增量更新、多路召回和图展示；本次没有修改正式 MCP、Rust 账本或应用 UI，也没有确定生产选型。

输入定义来自 [Task / TaskRun 对象模型](../30-domains/task-management/task-record-model.md)，数据位置和并发语义见 [Task MCP runbook](codepet-task-mcp.md)。直接读取 v4 JSON，不调用可能触发迁移或恢复事务的 Store。

## 环境与证据

2026-11-25 本机 macOS 实测，Cognee 固定为 1.6.10、Python 3.12。证据在忽略目录 `artifacts/cognee-task-recall-qa/runs/20261124T185318003193Z/`，`latest.json` 指向最近一次运行。

首轮新下载了两个模型，不是 CodePet 原有能力：

- embedding：FastEmbed 的 `sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2`，384 维。
- 抽取：Cognee `gliner_demo` 使用的 `fastino/gliner2.5-base-v1`。没有调用云端 LLM。

共享模型缓存约 1 GB，隔离 Python 环境约 1.9 GB，均在上述忽略目录。数据库为本地 SQLite、LanceDB 和 Ladybug。该 embedding 实际上下文窗口使框架将块大小收紧到约 126 tokens；不能按请求配置的 512 推断实际分块。

固定快照包含 9 个真实 Run。每个 Run 导出一份 Markdown，保留原始中文、业务 ID、目标、方案、验收及最近有效进度，省略迁移噪声和重复工作区观测。评估时另一个进程更新了一份源 JSON，因此源哈希对比为不同；原型的只读防护没有记录写入尝试。哈希不同本身不能证明是原型写入。

### 实际观察

`report.json`、各召回原始回执、`graph-baseline.json`、更新前后图和 `assessment.json` 支持以下结果：

| 项目 | 本次结果 |
| --- | --- |
| 基线图 | 153 节点、192 条结构关系，其中 26 个实体；没有抽出语义业务关系 |
| 实体质量 | 主要是 MCP、Rust、CSS 等技术词，中文领域概念较弱，存在 TaskRun 别名未合并 |
| 更新机制 | 文档 ID 稳定、旧文本/图内容移除、旧块向量删除、保留块向量存在、新旧实体替换、共享实体 ID 稳定、相同内容更新无操作、副本恢复检查全部通过 |
| 更新耗时 | 一次受控替换约 2.15 秒，相同内容约 0.05 秒；仅是此次观测 |
| 关系可信度 | 英文控制文本只写 `uses`，模型额外抽出 `acquired`、`collaborates_with`，不能当作真实事实 |
| 拒绝无关查询 | 未实现；财务付款负例仍返回多个语义/图候选 |

7 个正例的目标 Run 在检索前根据源任务标注，另有 1 个负例。结果是小样本冒烟检查，不能外推为生产准确率：

| 召回路线 | Hit@1 | Hit@3 |
| --- | --- | --- |
| 语义块 | 5/7 | 6/7 |
| 原生 BM25（去除零分候选） | 1/7 | 2/7 |
| 图上下文 | 5/7 | 7/7 |
| 自定义融合 | 5/7 | 7/7 |

图上下文会使用结构图和向量召回，命中不能证明抽出了有用的业务关系。原生 BM25 默认分词不适合中文连续文本，且会补零分 top-k；本原型保留原始回执并过滤零分候选，没有替换中文分词器。

## 实现路径与涉及模块

- `prototypes/cognee-task-recall/dataset.py`：读取账本、导出每个 Run 的规范文档、计算内容哈希，保持业务来源可追溯。
- `engine.py`：将所有数据库和缓存定向到实验目录；调用真实 Cognee 导入、更新、图导出和三路检索。通过 `verbose` 返回的对象及来源元数据还原 Run ID，不从拼接后的上下文或相似标题猜归属。
- `probe.py`：固定快照、标注查询、执行评估及受控更新；合成的版本标记和库名只写入副本，最后恢复副本原文。
- `readonly.py`：Python audit hook 拒绝源目录中的写入、删除和改名；这不是操作系统级沙箱，原生数据库依赖也必须配置独立路径。
- `report.html`、`render_report.py`、`serve.py`：提供无外部资源依赖的证据页、原始 JSON 和 Cognee 原生图。默认 Run→实体图是根据文本块来源生成的投影，明确区别于抽取出的语义关系。
- `test_dataset.py`、`ui_qa.mjs`：验证来源映射、跨 workspace 候选、只读保护，以及图切换、节点查看、文档预览、焦点和窄屏布局。

融合使用固定加权倒数排名：语义 1、BM25 0.7、图 0.4、时间 0.04、workspace 0.05，分母常数 10；没有用本次命中结果调参。仅有时间或 workspace 匹配不会独立生成候选。正式召回接口尚未定义。

## 复跑步骤

在仓库根目录执行；首次同步依赖和运行可能下载模型。默认只验证上述固定本地组合，替换模型需要重新验证维度、窗口和抽取质量。

```sh
UV_PROJECT_ENVIRONMENT="$PWD/artifacts/cognee-task-recall-qa/.venv" uv sync --locked --project prototypes/cognee-task-recall --python 3.12

artifacts/cognee-task-recall-qa/.venv/bin/python prototypes/cognee-task-recall/probe.py --source-root "$HOME/Library/Application Support/code-pet/tasks/v4"

python3 prototypes/cognee-task-recall/serve.py
```

服务只监听回环地址，在输出的 URL 打开报告。`--prepare-only` 只导出文档；`--resume` 可指定同一 output-root 下的已导入运行目录，复用其固定快照，不代表重新采样最新账本。一次运行包含的更新实验会改变实验数据库，避免并发复用同一运行目录。

## 验证与风险

已运行：

```sh
python3 -m unittest discover -s prototypes/cognee-task-recall -p 'test_*.py'
python3 -m compileall -q prototypes/cognee-task-recall
npm run check:source-size
git diff --check
```

UI QA：`node prototypes/cognee-task-recall/ui_qa.mjs <报告URL> <证据目录>`，需要 Playwright；可用 `CODEPET_PLAYWRIGHT_MODULE` 指定模块路径、`CODEPET_CHROMIUM_EXECUTABLE` 指定已有 Chrome。此次使用本机 Chrome，覆盖 1280 与 390 宽度，原生图可渲染且没有页面错误，截图和回执保存在证据目录。

- 误抽关系与别名：下一轮用中文代表性样本对比 LLM 抽取、受限实体/关系 schema，逐边检查来源；本轮控制样本已证明不能默认信任 demo 抽取。
- 无关结果与中文关键词弱：增加负例，验证拒绝阈值和中文分词，分别记录单路与融合结果。
- 活跃账本并发变化：用固定快照评估，保留源哈希和只读回执；准备正式订阅更新时验证 revision、删除及失败重试。
- 依赖体积与平台差异：本轮只验证 macOS Python 原型，sidecar 打包、Windows、启动恢复和资源上限需独立验证。

## 未知项与后续知识演进

云端或其他本地 LLM 的中文抽取质量、长期更新后的残留关系、文档删除、并发更新、较大语料的延迟及费用均未验证。本次未证明 Cognee 适合生产，也没有比较 Graphiti。后续实验沿用原始证据与独立指标，确认选型后再记录架构决策；无需将实验配置写入正式 Agent 接入规则。

## MLX Core 上的 LLM 与 embedding 对照

用户随后安装并启动了 `ddalcu/Qwen3.8-27B-MLX-Serve-4bit` 和 `Qwen3-Embedding-0.6B-8bit`。本机 `http://127.0.0.1:11234/v1/models` 可见两者已加载；实际小请求返回结构化中文 JSON 和 1024 维单位向量。此路线复用已安装模型，不再下载推理权重。

`--backend mlx` 固定使用两个独立的 OpenAI-compatible 本地接口。`mlx_gateway.py` 是实验用回环转发器：限制转发路径和抽取并发，记录实际输入、输出、耗时及错误，不记录凭据或向量数组。`development_graph_prompt.txt` 约束开发领域概念、关系、计划/实施/验证状态与逐字证据。Cognee 仍负责真实的图、向量写入及更新；原始图没有被展示层改造成“看起来合理”的关系。

首个串行转发尝试中，一次真实抽取约 125 秒；同批其他请求在排队时达到 180 秒超时。随后 3 路并发也出现超时，两次尝试已中断，不能当作完成结果。MLX Core 日志显示约束 JSON 解码单路约 13.7 token/s、三路每路约 4.6 token/s；移除 `json_object` 解码约束后，一份真实文档约 22.4 秒、759 输出 tokens、解码约 47 token/s，返回合法的 8 节点 / 4 关系 JSON。该观察不是严格同输出的性能基准。

当前适配器仅移除 Cognee fallback 发出的 `json_object`：完整 schema 仍在提示词中，Cognee 仍用 Pydantic 校验并记录失败；真正的 `json_schema` 请求保持原样。按单路转发、每段最多 8 实体 / 8 关系重新评估。请求和失败日志保留，耗时不代表没有排队的单请求性能。API 模型别名不能被 Cognee 1.6.10 的该 embedding 适配器解析为 Hugging Face tokenizer，当前会警告并回退近似计数；服务声明 32768 上限，实验分块 cap 为 4096，仍需核对实际请求 token 数。

复跑时先用首轮固定快照和原输入比较模型配置，再让同一套 MLX 配置读取完整有效进度历史：

```sh
artifacts/cognee-task-recall-qa/.venv/bin/python prototypes/cognee-task-recall/probe.py --source-root "$HOME/Library/Application Support/code-pet/tasks/v4" --snapshot-from artifacts/cognee-task-recall-qa/runs/20261124T185318003193Z --backend mlx --input-mode latest --skip-update

artifacts/cognee-task-recall-qa/.venv/bin/python prototypes/cognee-task-recall/probe.py --source-root "$HOME/Library/Application Support/code-pet/tasks/v4" --snapshot-from <上一条成功运行的证据目录> --backend mlx --input-mode history
```

`history` 输入保留有时间的有效活动，过滤工作区观测，仅对完全相同的事件去重，并标注状态、计划和“预期验收条件不是通过证据”。两轮使用相同的业务记录，但从 demo 到 MLX 同时改变模型、提示词和分块，不能把结果差异单独归因于 LLM。关键词路线仍使用原生 BM25，没有在这一轮同时更换中文分词。

这里的“历史输入”指首轮导出快照中已有的进度历史，不等于整个 Run 的全部原始信息：没有读取完整对话或代码，首轮导出也未保存 `validationPlan.records`，测试结果主要来自进度自述。正式 finish 快照应补齐这些结构化验证记录，再单独评估，避免把验收定义当作实际结果。

`llm-assessment.json` 和报告展示真实抽取、逐字引用不匹配以及接口错误；引用存在不等于事实蕴含正确，需逐边审阅。`model-calls/*.pending` 表示尚未完成的调用，不是成功回执。报告的语义图、Run 来源投影和原始底层图分开展示。

设计上以 `finish-task-run` 成功落库后固定版本，再异步加工和更新索引；CodePet 负责快照、去重、失败重试与版本淘汰，Agent 只报告进度和结果。此原型尚未接入真实 finish 事件。为了对照沿用的 9 个历史 Run 中有 1 个仍在进行中，离线索引它不证明 finish 触发流程或人工验收已完成。

### MLX 对照结果与集成限制

成功结果目录分别为 `runs/20261124T195044796867Z`（原输入）和 `runs/20261124T195156701196Z`（历史输入），均位于同一忽略的证据根目录。前一轮的 9 次 LLM 结果实际计算于 `runs/20261124T194415123073Z`，该次向量写入遇到 85 条批量请求失败。用同一批文本分别请求 1 条、8 条均成功；适配器随后将大批量拆成不超过 8 条，并按原索引合并向量。成功重建时通过 `--reuse-model-calls-from` 复用完全相同请求哈希的成功回执，不重新计算 LLM，回执保留 `reusedFrom`。原始失败记录没有删掉，重建耗时不能当成完整推理耗时。

| 配置 | 实体 / 语义关系 | 融合 Hit@1 / Hit@3 | 图路线 Hit@1 |
| --- | --- | --- | --- |
| GLiNER + MiniLM 原输入 | 26 / 0 | 5/7 · 7/7 | 5/7 |
| Qwen3.8 + Qwen3 Embedding 原输入 | 85 / 65 | 6/7 · 7/7 | 7/7 |
| 同一套 Qwen 配置 + 有效进度历史 | 112 / 74 | 6/7 · 7/7 | 5/7 |

增加历史没有进一步提高这组融合命中率，图路线首位命中反而下降，不能以节点更多证明质量更好。历史输入分成 11 块；原型将每个 Run 的全部原始块输出汇总为一份可读知识文档，排除控制更新和恢复时的重复抽取，不再次调用 LLM。图可按抽取回执里的来源筛选 Run，也保留全图和 Cognee 原生图。

历史轮真实导入、抽取和建索引约 323 秒；含召回、受控更新及导出的整轮约 420 秒。受控替换约 24.7 秒、重复更新约 0.05 秒。一次 LLM 接口 500 经框架重试恢复，保留在审计中。不能将所有请求耗时相加当作墙钟耗时，其中包含排队。

10 项文档/向量/重复更新/恢复检查和 4 项实体替换检查全部通过；各路召回成功且未新增 LLM 调用。所有观察到的向量均为 1024 维。财务负例仍有 6 个语义候选和 5 个图候选，未实现拒绝阈值。15 份图抽取回执中有 1 条引用不逐字、3 份违反数量约束；这些是审计观察，不会被自动标成可信事实。引用存在也不能证明关系蕴含，下一步需结构校验、证据拒绝与逐边质量标注，不能仅凭本轮小样本进入生产。

14 项单测通过，覆盖多块归属、控制样本排除、回执复用、向量批次合并与旧只读边界；源码尺寸和 diff 检查通过。浏览器检查涵盖知识文档预览、Run 筛选、语义/来源/原始图、节点与键盘焦点、1280/390 宽度及原生图。完整证据见各运行目录的 `ui-qa.json` 和截图。正式 MCP、真实 finish 事件、生产持久队列和打包仍在本次范围之外。

### 更新控制样本更正

核对实际更新输入时，发现历史轮 `update-input-after.md` 出现 `2026/13/54`、`2026/13/55`。原因是控制样本用全局 `replace("1/2", "3/5")` 修改宽度，同时匹配了日期 `2026/11/24`、`2026/11/25` 的子串。问题只存在于实验副本的控制输入，原始任务账本未写入；原始报告和样本保持原样，不能把它们视为正确的业务更新输入。

`probe.py` 的 `make_update_fixture` 改为只替换明确的宽度语句及 CSS 后备值。新增回归测试验证日期、其他比例及其他 CSS 数值保持原样。业务更新的正式输入应由结构化 Run 重新渲染完整文档，实验数值替换不能复用为生产逻辑。

修正后的独立复核保存于历史轮目录的 `update-fixture-recheck/`，包含前后完整文档、调用回执、更新结果和恢复后的图；它复用实验数据库，完成后恢复原始文档。其 `report.json` 是更正测试的证据，历史轮原报告保留作审计。原有召回对比发生在控制更新之前，不受错误测试日期影响。单测总数增加为 15 项。
