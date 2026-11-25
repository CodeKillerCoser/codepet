# Task Memory 的 Agent 接入

## 背景

2026-11-25 用户将当前任务能力限定为 Task Memory，要求参考其他 Agent 产品改进接入，暂不引入 hook 或 runner。源码核对发现，原安装请求要求把整段使用说明写入“系统提示词”，但实际目标是客户端的全局规则文件；同一说明又由 MCP initialize 返回，容易重复维护。复制编辑器草稿也不代表服务器已经保存该内容。

## 目标

- 先找回相关目标、方案与验证证据，再决定续办或创建任务。
- 区分连接配置、简短持久化约定和详细 MCP 使用说明。
- 保留验收证据、人工完成锁及低频里程碑写入；现有六工具包括追加协作评论。

## 非目标

不增加独立消息板、Agent 唤醒、hook、runner、任务调度或文件认领；Run 内的追加评论和回复见 [活动线](../30-domains/task-management/task-run-activity.md)；不在用户点击复制时自动修改客户端配置或全局规则。

## 现状理解

MCP initialize 的 instructions 是服务器使用说明，协议允许客户端用于模型上下文，没有要求客户端必须将其作为 system prompt。客户端持久化规则是另一条加载渠道，也不等于替换内置提示词。当前 App 保存的自定义说明覆盖默认值，新连接读取保存值；保存不改写已有客户端规则。

Concord 的接入会同时配置 MCP 与调用时机，AAMB 则提供读取、回复、订阅的使用指南。本次借鉴两者的接入分工，使用现有 find-task-runs 找回 Task/Run，不额外创建一份记忆存储。

## 实现路径

默认说明与工具描述优先引导检索相关执行、复用匹配任务，仅按关键节点登记，保存可用于后续恢复的结论和证据。历史内容需要结合当前代码与用户要求核实，不能当作新的授权；已完成任务仍由人工恢复。

接入弹窗提供默认勾选的“同时配置简短持久化约定”。勾选时，复制的安装请求要求在对应客户端规则中合并一个有固定边界标记的简短段落，只说明调用时机，并指向连接中的说明和工具定义；取消时仅安装 MCP，保留现有规则。界面保持直接编辑 MCP 使用说明，不新增常驻协议参数说明区。

复制内容保留精确的客户端配置和说明原文，详细说明明确仅供核对。草稿与已保存说明不同时，在界面回执和复制文本中都提示尚未保存，持久生效需在 App 保存并重连。安装验证要求初始化、读取工具列表及一次 find-task-runs 只读查询；空结果不是连接失败，不为验证创建生产任务。

## 涉及模块

- `crates/codepet-task-mcp/instructions.md`、`src/lib.rs`：默认工作流及工具描述是仅连接 MCP 时的发现入口；不更改状态机和参数。
- `frontend/lib/taskAgentInstall.ts`：分别生成连接步骤、可选简短约定、说明核对内容及只读验证要求。
- `frontend/lib/TaskAgentSetup.svelte`：选择接入范围，区分说明保存和安装指令复制结果，保留 revision 冲突草稿。
- 安装请求测试与 `scripts/task_ledger_ui_qa.mjs`：验证五种客户端、两种接入范围、草稿、失败重试和键盘/窄屏行为。

## 风险

- 说明或规则配置被误报为已连接：安装请求分别要求实际验证结果，界面只显示已复制。
- 草稿被误当作当前服务器配置：比较草稿与保存值，复制原文但明确未保存；既有 HTTP/stdio 测试验证真实初始化仍读取保存值。
- 详细说明重复进入全局规则：测试检查固定标记段内只含简短约定，取消时不存在规则路径和标记段。
- 新选项挤压布局或失去操作入口：Chromium/WebKit 回归覆盖浅暗主题、320/390/820 宽度、焦点、键盘切换、复制与保存。

## 测试计划

运行安装请求与任务前端测试、Svelte 检查、源码尺寸检查、任务 UI 双引擎回归及任务 MCP 测试（包含真实 HTTP/stdio 和说明保存/恢复/重连）。构建前端并目视检查接入弹窗；浏览器 fixture 不代表安装版或其他 Agent 客户端已经实际接入。

## 验证结果

2026-11-25 当前 worktree 初始没有前端依赖，先执行 `npm ci --no-audit --no-fund`，没有改写锁文件。以下最终检查通过：

- `./node_modules/.bin/vitest run frontend/lib/taskAgentInstall.test.ts frontend/lib/taskLedger.test.ts`：13 项通过，包含接入范围、详细说明与持久化段落分离、草稿和代码围栏。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-task-mcp`：19 项通过、3 项依赖独立 sidecar/本机模型的既有测试忽略；覆盖真实 HTTP/stdio、说明持久化与恢复、五工具及说明大小约束。
- `node scripts/task_ledger_ui_qa.mjs` 与 `CODEPET_QA_ENGINE=webkit node scripts/task_ledger_ui_qa.mjs`：最终逐引擎串行通过，覆盖五个客户端、两种接入范围、保存冲突、复制失败重试、键盘及浅暗主题 320/390/820 宽度。并行启动曾因现有 Vite 端口竞争失败，不作为通过证据。
- `npm run check:frontend`：0 错误、5 条既有警告；`npm run check:source-size` 无违规；`npm run build`、`git diff --check` 通过。
- 目视检查 `artifacts/task-ledger-qa/layout/chromium/agent-setup-light-320.png` 与 `agent-setup-dark-820.png`，复核选项、图标、说明和保存入口；QA 页默认说明直接读取服务器文件。

本轮生成工作区前端产物与任务 MCP 测试程序，未替换本机安装版。生产账本仅做查询；已有“Task MCP 与任务管理”被人工完成并锁定，未修改其记录或另建替代任务绕过锁定。

## 知识沉淀

通用约束写入 [本机任务 MCP 规约](../60-rules/local-task-mcp-transport.md)，本文件维护接入产品行为。依据：[MCP InitializeResult](https://kotlin.sdk.modelcontextprotocol.io/kotlin-sdk-core/io.modelcontextprotocol.kotlin.sdk.types/-initialize-result/index.html)、[Codex 持久化规则](https://learn.chatgpt.com/docs/agent-configuration/agents-md)、[Concord 接入](https://github.com/Get-Concord-AI/concord-mcp/blob/main/docs/claude-code.md)、[AAMB 使用指南](https://aiagentmessageboard.com/skill.md)。

## 未知项

各客户端是否将服务器说明加入模型上下文、加载的消息角色及重连方式，需要各自实现与实际版本证据，不能从 MCP 连接成功推断。提示词不能保证模型每次主动调用。用户已保存的自定义服务器说明继续原样保留，采用新默认值需主动恢复默认；已有客户端规则不会自动更新。
