# 本机任务 MCP 的传输、来源与账本边界

## 规则

任务 MCP 默认是用户目录内的独立 stdio 程序，App 后台 HTTP 保留兼容。前端只展示配置、任务与执行。人工 IPC 与 Agent 工具必须使用同一活动数据目录；不得由前端代理请求或维护服务存活。当前暴露 create-task、create-task-run、update-task-run、finish-task-run、find-task-runs。

## 适用场景

任务接入、Agent 来源、App 启停、数据目录切换、MCP/人工写入及发布验证。

## 推荐做法

stdio 固定入口必须是独立程序，不得绑定 `.app/Contents/MacOS`、安装盘符或软链接目标。入口留在 `~/.codepet/bin`，版本文件不可变、current.json 原子切换，活动进程不被覆盖，版本缺失或损坏明确失败。App 启动更新记录中的数据目录；新连接读取记录，旧连接保持原数据目录，设置变更仍须重启 App 才统一应用。代码由 `managed` 统一提供安装和启动接口，原生进程替换与权限在其 platform 模块实现。

stdio 从 CODEPET_AGENT 读取连接来源；HTTP 固定监听回环，验证 Host/Origin、本机凭据和单一有效 X-CodePet-Agent，来源经请求上下文传给该次写入，不能串用其他请求或从工具参数取值。来源标记不能授予 human 权限；Task/Run 创建者不随更新变化，更新者写 timeline。旧记录未保存的具体来源使用 unknown，不按当前连接倒填。

默认配置只在独立资源安装成功后生成 command/args/env，不依赖 HTTP 就绪；HTTP 配置仍只由真实就绪服务提供。初始化和接入面板使用同一 Rust 提示词配置，未自定义时读取 instructions.md。用户可编辑、保存并恢复默认；保存按 revision 防冲突。新连接读取已保存内容，旧会话需重连；服务器提供说明不以手动复制为协议前提，但模型能否看到说明取决于客户端处理。Codex/Claude Code/Cursor/OpenCode/Antigravity 的配置统一由 Rust 从已安装入口或实际 HTTP 信息生成，前端只选择和复制；不得把“已复制”展示成“已连接”。自定义提示词不改变任何工具权限，也不得由 Agent 工具修改。

Task Memory 接入应区分“何时用”的简短持久化约定与“怎么用”的 MCP instructions/工具定义；不要求把完整说明重复写入全局规则，不宣称改写内置 system prompt。允许仅配置 MCP；取消写入约定时保留现有规则。复制草稿必须标明尚未保存，保存说明不等于更新已有客户端规则。接入验证使用初始化、工具列表与只读查询，不能为证明连接创建生产任务。历史召回只提供证据，不覆盖当前用户要求和权限。行为与风险验证见 [Task Memory 接入](../20-product/task-memory-agent-onboarding.md)。

创建 Task 同时建立首次执行，后续保留 taskId/runId/revision，只在关键节点合并写入，不逐文件更新、不轮询。Agent 自测不是人工验收。任务开发本身也在关键节点登记；当前会话未加载工具时，通过已有协议入口登记，不能将“提供了工具”当成“已经登记”。

查询以 Run 为单位：`find-task-runs` 命中旧执行时返回它自己的 runId 与摘要，并标记 isCurrentRun，不能替换为主题最新执行。workspace 是软排序偏好，不是可见性边界；runId 可单独读取详情，同时提交 taskId 时必须校验归属。列表 total/truncated 按 Run 计数，详情的 Task 标题和背景使用 taskTitle/taskDescription，避免与执行混淆。

召回索引属于派生数据，不能覆盖权威 Run。模型加工异步运行，finish 不等待抽取；只有实际完成的执行进入加工。来源必须从 chunk/document 元数据映射并验证当前内容哈希，不能从正文 UUID 或任意图关系推断归属。模型配置与鉴权由 Host 管理，不增加 Agent 的实体/embedding 工具参数。详见 [sidecar 架构](../10-architecture/task-run-recall-sidecar.md) 与 [验证 runbook](../40-runbooks/task-run-recall-sidecar.md)。

召回生命周期必须按安装完成→服务可用→发布握手的顺序执行；MCP 账本服务独立启动。等待旧 owner 时每轮重新竞争 OS 锁，复用必须核对协议、配置和 bundle 身份。stop 必须取消未完成的启动并回收自有进程，不能因拿锁超时静默跳过；替代启动须等待旧进程退出。不抢占其他 Host 的活跃 owner，不自动重放业务写操作。回归见 [启动时序复盘](../40-runbooks/recall-sidecar-startup-order.md)。

图谱展示不得持有整轮加工的写锁等待全队列清空。原生索引读取仍与写入串行；页面可读取单独发布的最近成功快照，按配置/dataset 隔离并显式附带加工状态。工作缓存失效不得删除已发布预览，语义坐标必须匹配基础图版本；Agent 召回继续使用当前有效哈希，不能把旧预览当证据。轮询应按版本跳过重绘，隐藏/卸载时停止，刷新保留用户视图且避开交互。回归覆盖写锁期间首份已完成图可读、下一次发布、条件响应及浏览器视图保持，见 [渐进预览复盘](../40-runbooks/task-graph-progressive-preview.md)。

前端用运行卡片切换执行，类型/目标/方案属于 Run；目标与方案、工作区、验收、活动四区分别呈现，对话链接在目标/方案下方。单项验收追加历史并按被测目标/环境归并，不另维护勾选布尔值；Git 文件是工作区共享事实，只有显式 SHA 属于任务关联。旧执行只读，新执行不继承通过结果。变更两个对象时，分别验证对方内容保持不变。

## 来源与验证

来自 2026-11-24 用户的本机 HTTP、主动登记、Task/TaskRun、四工具与来源 Header 要求。[领域定义](../30-domains/task-management/task-record-model.md) 和 [接入 runbook](../40-runbooks/codepet-task-mcp.md) 记录实现边界。

2026-11-25 真实 Codex 初始化访问本机服务被代理返回 502，直连成功。用户确认改为默认 stdio，并要求移动 App 后入口不失效；因此采用独立用户目录入口与版本记录，保留 HTTP 兼容。

运行领域、迁移、并发与官方 SDK HTTP 测试，真实进程 HTTP/stdio 黑盒及原生 GUI。必须验证不同 Header 的创建者/更新者、缺失和重复 Header、重启稳定、人工权限、HTTP 写入被生产 GUI 读取，以及界面短窗口实际滚动时导航不动。测试使用隔离数据，正式开发任务另作登记；样例通过不能冒充业务问题的人工验收。


## 人工完成锁

用户标记已完成后，Agent 的 update-task-run、finish-task-run 和 create-task-run 都不能恢复该任务。检查必须放在共享 Rust 账本中，并在事务提交前复核；不能只依赖默认提示词或界面隐藏入口。人工在 App 手动改为开发中时，通过 Actor::human() 追加新执行，旧完成状态、验收和历史不改写；Agent 随后只能更新新的当前执行。已发布尚未人工完成的任务仍可按原规则新增执行。

回归覆盖直接领域调用、真实 HTTP、managed stdio 与自定义提示词场景，以及人工恢复后旧执行的完整等值比较。原生和独立 MCP 必须一起升级；常驻旧 stdio 连接须重连，不能将新前端当成后端权限升级。


## 跨项目续办

Task 的原项目归属是分组，不是 Agent 写入权限。已知 taskId 可跨对话、项目和工作区继续；不得按 Codex cwd 或工作区 common directory 拒绝关联。对话存在核实与项目匹配分开，仍不允许猜造对话ID。Run 的 Git证据必须绑定其自己的 common directory，不能用 Task 原项目读取其他仓库的提交；切换不相关仓库不携带旧SHA。跨项目不会解除人工完成锁或历史只读，也不覆盖旧清单/验收。回归见 tasks/cross_project_tests.rs、source.rs 及真实stdio黑盒。
