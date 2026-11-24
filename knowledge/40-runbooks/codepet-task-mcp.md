# 本地 Task / TaskRun MCP 接入与验收

## 入口与升级

2026-11-25 当前实现要求 backend API 30。仅更新前端不能获得新模型与工具，需安装新版 App。Codex工作台 → 任务与验收 → 更多 → 接入 Agent，从已就绪服务复制完整配置，替换 config.toml 的 codepet_tasks 条目，然后在新对话使用。

```toml
[mcp_servers.codepet_tasks]
url = "http://127.0.0.1:47623/mcp"
http_headers = { Authorization = "Bearer <从 App 复制>", "X-CodePet-Agent" = "codex" }
```

不要复制占位凭据，删除旧 command/args。X-CodePet-Agent 由配置自动附到每条 HTTP 请求，使用 1–64 位字母、数字、点、下划线、短横线，服务统一小写；human/user/agent/unknown 是保留值。它标记调用来源，不能取代认证或授予人工权限。缺失、重复或无效来源返回 400；无效凭据 401，错误 Host/Origin 403。

服务由 App Rust 后台托管，固定回环端口；关闭窗口而 App 仍运行时保持，退出时取消并释放。端口占用会报告失败，不跳到其他端口，也不生成假配置。Bearer 凭据位于当前 data/tasks/mcp-http.token，Unix 权限 0600，重启稳定、不写日志。数据目录切换在重启后统一应用。前端不代理 MCP。

## 五个工具

| 工具 | 输入与职责 |
| --- | --- |
| create-task | title、description、agentWorkspace.path，及必填 initialRun（含非空 agentConversation）；一个事务同时创建任务与首次执行 |
| create-task-run | taskId、expectedRevision、title、kind、goal、solution，按需携带补充说明/工作区/清单/对话；只为已发布或已结束目标开启新处理 |
| update-task-run | taskId、runId、expectedRevision，以及变化的摘要、status、validationPlan、verification、明确关联提交或 delivery |
| finish-task-run | taskId、runId、expectedRevision、conclusion、非空 agentConversation、changedFiles（无变更显式 []）、verification；原子结束开发并按必验本地结果进入待验收或保持进行中，不确认人工通过、不关闭 |
| find-tasks | 按项目/关键词搜索简短列表；taskId 读当前执行，runId 选择历史，history 显式获取历史摘要 |

所有 MCP 写入都必须携带非空 agentConversation，对话 ID 或 codex://threads/<id> 均可；人工入口可空。每项验收必须同时提供 result 与非空 evidence，缺任一项整次不落库。没有 update-task MCP 工具；任务基本信息可在 App 单独修改。kind 在 TaskRun 上，只有 bugfix/feature。状态为 in_progress、ready_for_delivery、published、closed，Agent 不能关闭或记录人工验收。首次 published 要同时提供实际 delivery；提交不证明发布。

最小创建示例（工具参数，不含创建来源和时间）：

```json
{
  "title": "修复首次点击无响应",
  "description": "切换窗口后首次点击偶发无响应",
  "agentWorkspace": { "path": "/absolute/project/worktree" },
  "initialRun": {
    "agentConversation": ["<实际 Codex 对话 ID 或 codex://threads/<id>>"],
    "title": "修复首次点击",
    "kind": "bugfix",
    "goal": "切换窗口后点击稳定响应",
    "solution": "定位并修复事件绑定",
    "validationPlan": {
      "items": [{ "title": "窗口切换后点击", "steps": "切换窗口后点击菜单", "expected": "正常打开" }]
    }
  }
}
```

保留回执 taskId/runId/revision/planRevision/criterionIds。更新省略不变字段，agentWorkspace 只允许提交路径和 relatedCommits，Git 状态与文件由 Rust 观测，不能从参数伪造。新执行省略计划时复用旧清单，但生成新检查项 ID 并清空结果；不要把恢复同次处理当作新执行。

通常开始实施一次 create-task，关键进展合并一次 update-task-run，开发完成一次 finish-task-run；发布和复发再按事实补记。初始化 instructions 自动携带这套规则。接入面板可编辑 Agent 使用提示词，保存后重新连接 MCP 或开新对话使用；恢复默认可撤销覆盖。自定义内容保存在 Rust 配置，不依赖前端页面存活，不修改全局 Codex 配置。纯咨询不登记，不轮询，不逐文件维护。普通回执保持紧凑，五工具定义受 24 KB 上限测试约束，实际数值见黑盒报告；不把这些字节数当作模型 token 计费或自动登记可靠性保证。

agentConversation 输入是实际对话 ID 的数组，来源从请求上下文取得。ID 不确定时不要猜造，先确认实际来源，新的 MCP 写入不允许省略；只有来源 codex 的引用尝试只读查询本地 state_5.sqlite 并核对仓库，缺失保持待核实，已知跨项目引用拒绝。不读取其他 Agent 的历史。Header 不等于对话 ID。

## 数据、迁移与并发

活动数据在 data/tasks/v4，项目映射为 projects.json，聚合为 projects/<projectId>/tasks/<taskId>.json。receipts 保存幂等回执，prepared.json 用于事务恢复；不手工修改。Task 元数据与 runs 分开，完整定义见 [领域文档](../30-domains/task-management/task-record-model.md)。

首次读取优先原子导入 v3，没有 v3 时导入 v2，源文件保持原样。v3 的任务类型继承到执行；原自由文本保存在补充说明，不能猜测拆成目标和方案。旧任务/执行/检查项 ID 和证据保留，旧 objective 合入说明，bug/investigation 标记变为 bugfix。旧 Agent 没有具体名称时显示历史 Agent；缺失文件清单的旧观测不伪装成当前扫描，之后更新当前执行重新读取。不要让旧 App/MCP 进程在迁移后继续写旧账本，升级后重启并替换配置。

expectedRevision 是整个 Task 的聚合版本。冲突保留草稿，用 find-tasks 读取后合并，不自动覆盖人工修改。请求失败用相同参数/operationId 重试；不同参数不能复用显式键。重试先查回执，因此工作区被删除后仍能返回已提交结果。

Git 状态属于共享工作区，上游只比较本地引用；不 fetch。relatedCommits 只接受可核实的 SHA。记录实际被测目标、环境、计划版本、执行者及证据，Agent 自测不能解锁人工通过；不同版本的结果不混用。结束执行仍可注明原因而不声称验收通过。

## 排查

- 400：检查新配置是否含唯一有效 X-CodePet-Agent；不要只保留 Authorization。
- 401/403：重新复制本机配置，检查凭据、Host/Origin，不关闭鉴权。
- 端口被占用：停止冲突服务后重启 App；不指向未知服务。
- revision 冲突：保留草稿并读当前执行，核对 runId 是否仍是当前。
- 原工作区已移除：更新进展可以继续，代码状态明确未知；新执行或切换工作区需要可用路径。
- 界面没显示写入：回到窗口或刷新任务；没有持续轮询。
- 凭据文件损坏：退出 App，备份异常文件后恢复凭据并重新复制配置，不输出 token 到日志。

## 验收命令

```sh
cargo test --manifest-path crates/Cargo.toml -p codepet-task-lineage -p codepet-task-mcp
cargo build --manifest-path crates/Cargo.toml -p codepet-task-mcp
node scripts/task_mcp_http_smoke.mjs
node scripts/task_mcp_smoke.mjs
TAURI_CONFIG='{"app":{"macOSPrivateApi":false},"bundle":{"resources":[]}}' cargo build --manifest-path src-tauri/Cargo.toml --features custom-protocol --bin code-pet
CODEPET_MCP_BINARY="$PWD/src-tauri/target/debug/code-pet" CODEPET_MCP_EMBEDDED=1 node scripts/task_mcp_http_smoke.mjs
CODEPET_MCP_BINARY="$PWD/src-tauri/target/debug/code-pet" CODEPET_MCP_EMBEDDED=1 node scripts/task_mcp_smoke.mjs
node scripts/task_ledger_native_qa.mjs
node scripts/task_ledger_ui_qa.mjs
CODEPET_QA_ENGINE=webkit node scripts/task_ledger_ui_qa.mjs
node scripts/codex_workbench_ui_qa.mjs
npx vitest run frontend
npm run check:frontend
npm run check:source-size
npm run build
TMPDIR=/private/tmp npm run test:webcontent
```

真实 HTTP/stdio 黑盒隔离 HOME/CODEX_HOME 与应用数据，验证五工具、创建来源、工作区文件、重启幂等、冲突和权限。官方 SDK 及原始 HTTP 验证每请求来源、Host/Origin、缺失来源和端口故障。原生测试用临时账本，HTTP 在 Rust 发出，GUI 通过生产 IPC 读取同一 revision，逐项走人工验收、后台关闭、新执行、历史只读与冲突草稿，并核对编辑后的提示词真正进入 HTTP initialize。样例不代表真实业务问题已发布。

浏览器复用原生快照，测试明暗、1280/820/600/390、完整 App 1143 视口与短窗口真实滚动。验证任务编辑与执行字段隔离、运行卡片键盘切换、四区布局、逐项验收与独立保存；同时验证提示词保存/重开/恢复/冲突、文件内容、明暗窄屏、菜单与弹窗焦点及忙碌取消拦截。WebKit 的 display-p3 颜色先转换到 sRGB 再计算。证据位于忽略目录 artifacts/task-ledger-qa/（mcp/http/embedded 报告、native/report.json、native-window.png、layout/）。

## 边界

安装版退出/重启的人工验收、Windows 本机运行和真实长对话自主登记仍待实际使用。测试直接 cargo 编译的临时 macOSPrivateApi/resource 配置不替代正常发行构建；正式交付走 GitHub Build 的 mac 平台。webcontent 默认 /var 与 /private/var 路径别名会使已有测试字符串断言失败，使用 TMPDIR=/private/tmp 后验证。不会修改用户全局 Codex 配置、替换本机安装版或监控已触发的云端构建。

本次真实开发任务“调整任务执行详情并支持编辑 Agent 接入提示词”的初始回执在 artifacts/task-ledger-qa/run-detail-implementation.json；通过已有 MCP 登记，完成后使用新工具与 Header 更新同一任务。它与隔离的验收样本分开记录，Agent 不记录 human 通过或自动结束。

提示词保存限制：非空、最多 16000 字且不超过 64 KiB；保存只作用于当前数据目录。多窗口 revision 冲突时重新打开面板后合并，不自动覆盖。初始化配置损坏返回错误，不悄悄丢掉自定义内容。既有四工具名保留，新增 finish-task-run； X-CodePet-Agent Header 保持不变，新参数要求 MCP 0.4 与原生 API 30。

完成调用示例（使用实际回执，不复制占位 ID）：

```json
{
  "taskId": "<taskId>", "runId": "<runId>", "expectedRevision": 2,
  "agentConversation": ["<实际对话 ID>"],
  "conclusion": "已定位并修复事件注册遗漏，本地验证通过，等待安装包验收。",
  "changedFiles": ["src/menu.rs", "src/menu.test.rs"],
  "verification": {
    "planRevision": 1, "stage": "local", "target": "<实际工作区/commit>", "environment": "macOS",
    "results": [{"criterionId": "<检查项 ID>", "result": "passed", "evidence": "切换窗口后连续点击 10 次，每次均打开菜单。"}]
  }
}
```

待验收使用原 ready_for_delivery 值；有必验本地项未测/失败/受阻时仍进行中。表单必须主动选状态并填结论，不能只写一段“验收通过”的进度摘要。新工具不会录入 human 结果。历史关联提交文件可在展开时通过 commit-files IPC 读取；文件不在时报告读取失败。API 30 必须更新原生 App，单换前端不能获得第五个工具。

finish-task-run 的 changedFiles 必须显式传入；省略拒绝，空数组表示本次无代码修改。路径是仓库相对路径，拒绝空白、绝对路径、父级跳转、仓库元数据路径和重复项。持久化到 agentWorkspace.runFiles，和当前共享未提交文件、明确关联提交文件分开，后续工作区清洁或其他更新不会清空它。
