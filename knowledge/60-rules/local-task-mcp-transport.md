# 本机任务 MCP 的传输、来源与账本边界

## 规则

任务 MCP 是 App Rust 后台的本机 HTTP 服务，前端只展示配置、任务与执行。人工 IPC 与 Agent 工具共享同一个活动 Ledger；不得由前端代理请求或维护服务存活。当前只暴露 create-task、create-task-run、update-task-run、find-tasks。

## 适用场景

任务接入、Agent 来源、App 启停、数据目录切换、MCP/人工写入及发布验证。

## 推荐做法

服务固定监听回环，验证 Host/Origin、本机凭据和单一有效 X-CodePet-Agent；未启动或端口占用不能展示可用配置。来源经 HTTP 请求上下文传给该次写入，不能用全局变量缓存到另一请求或从工具参数取值。Header 是来源标记，不能授予 human 权限；Task/Run 创建者不随更新变化，更新者写 timeline。旧记录未保存的具体来源使用 unknown，不按当前连接倒填。

配置从实际运行服务生成，包含 url、Authorization 与 X-CodePet-Agent，退出 App 停止服务，关闭业务页面不停止；数据目录变更在重启后统一生效。初始化使用规则与界面说明共用 instructions.md，避免两份规则漂移，不要求用户手动复制说明才会用。

创建 Task 同时建立首次执行，后续保留 taskId/runId/revision，只在关键节点合并写入，不逐文件更新、不轮询。Agent 自测不是人工验收。任务开发本身也在关键节点登记；当前会话未加载工具时，通过已有协议入口登记，不能将“提供了工具”当成“已经登记”。

前端保持任务说明、当前执行、验收清单三个常驻区域。工作区、对话及时间线折叠；Git 文件是工作区共享事实，只有显式 SHA 属于任务关联。旧执行只读，新执行不继承通过结果。变更两个对象时，分别验证对方内容保持不变。

## 来源与验证

来自 2026-11-24 用户的本机 HTTP、主动登记、Task/TaskRun、四工具与来源 Header 要求。[领域定义](../30-domains/task-management/task-record-model.md) 和 [接入 runbook](../40-runbooks/codepet-task-mcp.md) 记录实现边界。

运行领域、迁移、并发与官方 SDK HTTP 测试，真实进程 HTTP/stdio 黑盒及原生 GUI。必须验证不同 Header 的创建者/更新者、缺失和重复 Header、重启稳定、人工权限、HTTP 写入被生产 GUI 读取，以及界面短窗口实际滚动时导航不动。测试使用隔离数据，正式开发任务另作登记；样例通过不能冒充业务问题的人工验收。
