# 任务抽取运行时、接口与用户数据目录

## 背景

首期任务谱系已有对话树、任务图和正文抽取，但模型配置、Skill、工作目录及抽取状态尚未形成应用能力。2026-09-10 的实现将这几部分接入 CodePet 桌面端，逻辑全部使用 Rust，Svelte 只负责配置和呈现。

## 目标

支持任务查询、异步触发、可替换 Skill、摘要实例绑定、持久化配置、会话脏状态和有界定时抽取。沿用 [输入规约](../60-rules/task-lineage-extraction-input.md)：仅最近滚动 48 小时的用户消息与 assistant 正文，工具调用、工具结果、reasoning、未知或未来时间不进入模型。

## 非目标

不修改原始 Codex 记录，不自动验收任务，不安装 Python 运行时。第一版支持 Claude CLI harness；其他 harness 需要实现统一 `TaskExtractor`，不能仅在界面添加一个不可执行的选项。当前扩展是 Tauri 本地接口，尚未加入 Remote Gateway 和手机 SDK。

## 现状理解

`App.svelte` 的普通页面保留任务、Agent、连接、用量、个性化、事件六个导航项，设置入口放在内容区右上角。进入设置后，共享 `MainNavigation` 左侧显示设置分区，右侧由 `TaskSettingsPage` 显示配置，目前只有“任务抽取”。`TaskLineage` 专注对话/任务导航、图/泳道、原文证据、手动抽取和人工验收；设置表单不在任务页展开。`TaskExtractionSettings` 管理运行实例、模型、Harness、推理强度、Skill、提取提示词和自动间隔；预算、超时与静默间隔收进高级选项。原有继续对话仍走 Gateway，发送前检查运行/审批状态。

此前 App 仅保存单个 tab，无法从该字段还原已访问页面。现在 `navigation.ts` 的 `createNavigation()` 提供 Svelte 可订阅导航栈，状态含 entries、index、current、canGoBack、canGoForward；App 用 `$navigation` 读取，其他前端调用者可用 Svelte `get(navigation)` 获取快照。navigate 同页不重复入栈，后退后新导航截断前进分支，最多保留 100 项。工具栏按钮调用 back/forward，设置入口调用 navigate。此历史是本窗口生命周期内的页面级记录，不依赖原生窗口或浏览器 History，也不记录任务节点选择、表单草稿和关闭应用前的历史。

导航收起/展开由共享 `main-window.css` 对网格列宽执行 220ms 过渡，配合侧栏位移与透明度。收起时立即 inert，过渡结束后隐藏；展开时恢复可见。保留侧栏纵向滚动和 toolbar 的可访问名称，系统减少动态效果时关闭过渡。UI QA 使用生产任务/设置组件与共享窗口工具栏、CSS 验证切换、保存、过渡中间宽度、inert、减少动态效果和窄屏。

前进/后退与收起按钮组成一个工具栏按钮组，在 macOS 统一使用既有 AppKit 锚点。收起动画期间，toolbar 的负 margin 与外层 padding 同步过渡，防止按钮相对红绿灯短暂位移。QA 复用生产 `MainNavigation`，不再以两项简化导航代替真实导航；其他页面业务内容仍未放入此专项 fixture。导航栈、连接页和窗口 chrome 的 18 项定向测试及构建通过，浏览器验证设置入口、前后退、六项菜单与按钮几何；Mac 实机 AppKit 对齐仍需实际设备验收。

任务页存在任务时默认选择首项并呈现任务图；用户仍可切换对话视角。首次抽取产生任务后也会进入可视化。此次 UI 调整验证包含当前源码的 27 项布局测试、3 项任务视图测试、前端构建与浏览器交互；浏览器使用模拟 IPC，不调用真实模型或发送真实消息。

## 实现路径

### 用户目录和数据结构

根目录使用 `configured_app_data_dir(AppSettings)`，尊重已有应用数据目录配置。Windows 默认是 `%LOCALAPPDATA%/code-pet`；也可由用户配置为其 `.codepair`，不另建一套脱离应用设置的根目录。

```text
<data>/
  skills/extract-tasks/SKILL.md
  skills/reconcile-tasks/SKILL.md
  workspaces/task-extraction/run-*/
    SKILL.md, prompt.md, config.json, input.json, result.json
  workspaces/tasks/<hash(providerId:taskId)>/
    task.json, workspace.json
  task-lineage/v1/<hash(providerId)>/
    conversation-tree.json, lineage.json
    threads/*, events/*, tasks/*, jobs/*
    extraction-settings.json, watch.json, latest-extraction.json
    prepared.json, writer.lock, inference.lock
```

Skill 模板首次安装使用 `create_new`，后续初始化保留用户编辑。每批从所选用户 Skill 读取正文，连同补充要求传给模型；工作区保存该次快照便于回溯。技能名限制为安全目录名，文件上限 64 KiB。任务工作区申请按来源和任务确定性分配，重复申请返回同一目录，保存任务及共享 Skill 目录引用；这一步不会替用户创建 Git 仓库或启动任务执行。

数据采用 JSON 文件与写前事务，不引入数据库。`Task` 包含 schemaVersion、revision、rootThreadId、title、detail、episodes、edges、manualCompletion、completionWatermarks。`Episode` 保存 threadId 与 evidenceIds；原始证据保存文件、字节偏移和 generation。人工验收与模型结果分开，模型回写重新检查版本与证据。Prepared 事务支持重启重放。

### 扩展接口

调用 `invoke("task_lineage_request", { providerId, request })`；`request` 是由 method 区分的严格类型对象，未知字段拒绝。`tasks.capabilities` 返回 version=1、支持的方法、harnesses、storage 和 lookbackHours。本地来源使用稳定标识 `local:codex`，不要求启用 Codex Provider；历史实例标识仍可通过原接口读取。

| method | 参数 | 返回 |
| --- | --- | --- |
| tasks.list | 无 | 线程、任务、待处理数、诊断和最近模型信息 |
| tasks.messages | threadId | 标准化正文与原始证据 |
| tasks.dirty | 无 | 每会话 state、revision、extractedRevision、pendingMessages、lastChangedAt、lastError |
| tasks.extract | requestId、threadId 或 null | 持久化 Job，立即返回 |
| tasks.jobs | 无 | 最近 30 个 Job |
| tasks.settings.get / set | set 提供 config | 带 revision 的抽取配置 |
| tasks.skills / tasks.skill.get | get 提供 name | 技能名称 / 正文 |
| tasks.workspace.request | taskId | taskId 和工作目录 path |

Job 保存 id、requestId、threadId、state、createdAt、finishedAt、error。相同 requestId 与范围重复提交返回原 Job；改范围则拒绝。每来源只准入一个 queued/running Job，跨进程推理租约防止重叠。应用重启后遗留 Job 标记 interrupted，不自动重复付费；新 requestId 才启动重试。手动触发先扫描，再执行一批；选中主会话包含派生线程，null 表示全部来源线程。

### 本地 Agent 数据来源（2026-09-11）

设置页与任务页仅选择“本地 Agent”，当前选项为 Codex（本地）。Rust 复用 SDK 的目录解析：优先 CODEX_HOME，否则用户目录下的 .codex；界面展示实际目录。来源选项和后台轮询均使用同一 local::contexts，未连接、未启动 Codex Provider 也能扫描、查看、配置及整理；目录不存在显示提示，不要求配置来源连接。输入仍严格限定最近 48 小时的用户消息和 AI 正文。

来源 ID 为 local:codex，接口字段 providerId 暂保留兼容命名。首次初始化若存在唯一指向同一本地目录的旧实例及任务存储，则原子创建 local-codex-store.json 指针，沿用旧任务、设置、水位和 Job；后续不再依赖该实例存在。不覆盖已创建的本地存储，不猜测多个匹配实例，未匹配的旧存储保留原位且不自动合并。后台仅调度本地来源，避免旧来源重复运行。

继续对话仍需 Gateway：只在目录规范化后找到唯一对应连接时，返回独立的 connectionProviderId。前端用这个 ID 查询运行状态和发送，用来源 ID 读取证据、验收和分配工作区。未找到唯一连接时状态未知并隐藏发送入口；不得把本地来源 ID 当成 Provider ID。抽取执行的 Claude 实例、模型和推理配置保持独立。

涉及模块：sources/local.rs 负责发现、连接匹配及旧存储指针；Tauri task_lineage.rs 将手动/定时入口接到本地来源；TaskSettingsPage、TaskLineage 和接口类型呈现目录并区分来源与发送路由。

风险与验证：目录不匹配或多实例可能误发消息，由 Rust 唯一目录匹配测试覆盖；升级可能覆盖旧进度，由保留文件、不可覆盖绑定和已有本地库测试覆盖；缺少连接可能阻断页面，由浏览器 disconnected 场景验证查看、扫描、手动整理和设置保存，同时断言无发送入口。浏览器使用模拟 IPC，未在本次验证中重新调用真实模型；实际本地文件解析由 Rust 测试覆盖。Mac 实机目录和原生窗口尚未验证。

本次验证通过：在 src-tauri 运行 `cargo test -p codepet-task-lineage`（28 项）和 `cargo check --lib`；仓库根运行 `npx vitest run frontend/lib/taskLineage.test.ts frontend/lib/navigation.test.ts`（6 项）、`npm run build`、`node scripts/task_lineage_ui_qa.mjs`（含无来源连接、响应式和焦点场景）。Rust 检查仅有既有未使用代码警告。

### Provider 执行链路（2026-09-11）

桌面端的 `ManagedExtractor<ProviderExecutor>` 先准备应用数据目录下的独立运行目录，再调用现有 `ProviderGatewayService`。数据来源选择本地 Agent（当前支持 Codex），自动读取本地会话目录；执行实例单独选择 Claude Provider，两者不互相替代。

执行顺序为 provider.describe → conversation.create（显式 workspaceRoot、模型和推理强度）→ turn.send → 收集对应 turn 的 text delta 并等待权威 terminal → 无流式正文时才尝试 conversation.get 分页回读 → Rust 证据与结构校验 → 事务写入任务。创建的是独立 Provider 会话，不续写用户原会话。桌面抽取不再解析 executable、设置 CLAUDE_CONFIG_DIR 或启动 Harness 子进程。Claude Provider 模型目录补充 haiku，防止默认低档模型被静默替换。

工作区保存 SKILL.md、prompt.md、紧凑 input.json、provider-run.json（会话/turn 引用）、provider-output.txt 和 result.json。技能正文与配置提示词由 Rust 读取并显式放入 turn 输入，不依赖“切换目录即自动发现 Skill”的假设；input.json 与送入模型的证据别名保持一致。

运行期间持有本地 Gateway 连接身份；先订阅事件再发起 turn，避免快速完成事件丢失。优先收集本次 turn 的 text delta（原生历史尚未落盘、或历史 turn ID 不同也可完成）；回读仅接受本次 turn 的 assistant text，拒绝截断和超过 2 MiB 的输出，支持分页并拒绝重复游标；工具和推理项不作为结果。超时、事件错误或等待审批时走 Provider interrupt，并有界等待停止确认。异常不推进抽取水位，定时抽取按既有规则暂停。

Provider 继承其既有本机设置、Hook 和 MCP；工作目录是运行目录，不是安全沙箱。抽取指令要求不使用工具；如果仍请求审批，本次整理会停止，不自动同意。统一 Provider 当前不返回每次抽取的真实模型用量与美元费用，因此这些字段保留 null，不能用请求模型冒充真实回执。

### 配置、状态与调度

设置定义的是任务抽取 Agent：harness、model、reasoningEffort、prompt、skill，以及 automatic / intervalSeconds 定时配置。默认 harness=claude（现有 Provider）、model=haiku、reasoningEffort=low、skill=extract-tasks、自动关闭、间隔 60 秒。高级选项提供超时 90 秒和静默 20 秒。历史 budgetUsd 字段保留以兼容已有配置及独立 CLI 研究示例；桌面 Provider 未提供单次美元预算参数，UI 不再展示可执行的美元上限。harnessInstanceId 为空时只使用唯一已启用 Provider 实例，不回退到直接启动本机 CLI；多个 Claude 实例要求选择绑定。配置版本冲突拒绝覆盖，Agent 配置与定时开关在同一 JSON 事务保存。旧配置缺少新字段时采用低推理默认值；读取时以持久化调度状态反映自动开关及失败暂停。

模型与推理选项取自所选 Provider 的 provider.describe，发送时再次校验 capability revision、模型和推理选项。Claude Provider 将推理强度映射为 `--effort`；某个档位是否生效由选定模型和本机 Harness 决定，依据 [Claude 模型配置文档](https://code.claude.com/docs/en/model-config)。当前执行适配器仍仅支持 Claude。实际模型名称来自 CLI 回执，不能用别名冒充实际模型。用户提示词与所选 Skill 共同构成 Agent 的任务指令，固定证据校验和 48 小时边界由 Rust 强制执行。

扫描新正文或文件重写时推进 dirtyRevision，工具执行不会推进。抽取捕获输入 revision/count；成功只确认该批水位。推理期间追加的正文保持 dirty，失败不推进水位。过期消息不填充上下文，长正文最多 2500 字符并带截断标记；单批最多 6 条、约 5000 字符，候选任务只向模型传摘要。父子关系仍依据结构证据计算。

后台默认关闭。用户开启 Agent 自动提取后，continuous 模式持续按间隔处理当前来源的全部近期历史，只对静默期结束的确切线程抽取，避免父会话就绪时误选仍活跃的子线程。不再每五批停机；空闲不运行模型，失败暂停并显示原因。旧 watch 接口的有界五批模式仍可读取和执行，保存新的 Agent 设置后转为连续调度。设置变化使下一次调度重新计算，关闭自动提取不会撤销已运行批次。

任务页默认展示任务图，另提供“整理历史”和“手动整理历史”。手动可选全部近期历史或当前关联对话，每次处理一个有界批次，增量水位保留；手动和定时调用同一个 ManagedExtractor，使用相同 Agent 配置。整理历史显示最近 30 次 Job 的时间、手动/自动来源、处理范围、状态、耗时及错误；每次运行的配置、输入、Skill 和结果仍留在用户工作区。

## 涉及模块

- `crates/codepet-task-lineage/{src/management.rs,skills}`：用户目录、模板、配置、Job、工作区和模型上下文组装。
- `src/{service,watch,extraction}.rs`：增量水位、静默调度、统一抽取接口、紧凑输入和证据校验。CLI 执行仅供独立研究示例使用。
- `src-tauri/src/task_lineage.rs` 与 `task_lineage/provider_executor.rs`：组合既有 Provider Gateway 和应用数据目录，处理会话创建、流式正文、终态、中断及本地扩展。
- `crates/providers/codepet-provider-claude/src/{provider,client}.rs`：广告 haiku 与 Windows 中断能力；每个 turn 收到 result 后关闭 stdin，由 reaper 确认退出，避免等待下一条输入。
- `frontend/lib/{taskLineage.ts,TaskLineage.svelte,TaskSettingsPage.svelte,TaskExtractionSettings.svelte}`：类型调用、任务可视化、异步状态及分区设置。`App.svelte` 提供独立入口，`main-window.css` 管理共享导航动画。

## 风险

- 抽取期间新增记录或用户验收：并发回归验证旧结果不吞新消息、不覆盖人工状态。
- 重复收费和遗留任务：请求幂等、租约和重启测试；Provider 失败也可能产生费用。当前没有美元硬预算，使用小批输入和执行超时。
- 用户 Skill 修改丢失或目录逃逸：首次安装保留测试、路径校验、持久化快照。
- UI 状态不收敛：事件加定时查询，浏览器测试验证排队到完成、配置保存和移动尺寸。

## 测试计划与结果

首版验证记录：crate 单元/流程测试共 24 项、Tauri `cargo check --lib`、前端定向 Vitest 3 项、Vite build 和 Edge Playwright。浏览器验证覆盖设置与 Skill、异步 Job、申请工作区、证据、图/泳道、焦点、人工验收、480/820/980 宽度。全仓 `tsc --noEmit` 仍为原有 27 项诊断，本次未增加 TypeScript 文件诊断。

此前独立 CLI 用户目录实测使用 `managed_probe`，仅合成两条正文：Rust 安装并读取真实用户 Skill，经本机 Claude `haiku` 返回一个带证据任务，实际模型 `deepseek-v4-flash`，报告费用 $0.013895。回执位于用户数据目录 `workspaces/task-extraction/run-LAB1Ef`，含五份可审计文件。

2026-09-11 Provider 改造验证：任务 crate 25 项测试通过；Claude Provider vertical 12 项通过（含 Windows interrupted terminal、result 后等待 EOF 的收尾回归）。Provider 执行适配器另有 2 项能力/正文过滤测试通过；前端 Vitest 6 项、生产构建和构建后页面的 Edge 交互检查通过，覆盖模型/推理/Skill 保存、定时配置、导航动画和窄屏布局。QA 改用独立构建入口与静态预览，避免开发依赖预构建阻塞，截图已检查。既有测试中两处历史问题一并修正：依赖白名单缺少现有 codepet-provider-data，Windows 路径比较需要双方 canonicalize。可控 Harness 经真实 Host 与 Provider 进程、指定工作目录返回一个任务，验证参数、Skill、输入、证据恢复和 EOF 收尾；它不是模型质量验收。

真实 Windows 调用也通过：本机 Claude、haiku / low、两条合成正文返回一个登录焦点任务，证据恢复为 synthetic-0 / synthetic-1，回执位于本次工作树 artifacts/provider-extraction-probe/workspaces/task-extraction/run-za6Haq。此前 90 秒超时后中断成功；补齐 result 后关闭 stdin，并改用正文事件收集后复跑完成。该验证经过真实 Host / Provider / Harness，未操作完整原生桌面窗口，未扫描真实用户历史。

复现命令：在 crates 执行 `cargo build -p codepet-provider-claude --bins`，在 src-tauri 执行 `cargo run --example task_provider_probe -- <Provider可执行文件> <Claude或claude-stream-fixture可执行文件> <独立输出目录>`。该探针只生成两条合成正文，独立目录保存 Host 状态和抽取回执；真实 Claude 模式会调用本机模型，默认 haiku / low。

## 知识沉淀

本文记录现行运行时；早期 [接入评审](../20-product/task-lineage-codepet-integration.md) 保留当时差距与后续范围，[交付记录](../40-runbooks/task-lineage-v1-delivery.md) 记录真实数据试跑。

## 未知项

尚未在 macOS 运行。原生桌面完整交互、多 Claude 实例实际账户隔离与 Remote 接入未做端到端验收；当前浏览器 IPC 模拟测试不等价于这些验收。持久化工作区尚无自动清理策略，应由用户按需要清理历史运行目录。
