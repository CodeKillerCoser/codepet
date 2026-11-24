# 本地任务 MCP 接入与验收

## 适用范围与入口

2026-11-24 的 Codex 任务账本。当前默认入口是 App 的 Rust 后台提供的 Streamable HTTP；前端只展示配置和账本，不承载 MCP。关闭主窗口而 App 仍运行时服务保持，退出 App 时停止。旧 stdio 入口保留兼容。

安装版须包含 backend API 27；仅更新前端不能获得 HTTP 服务。“Codex工作台 → 任务与验收 → 接入 Codex”在服务就绪后提供完整配置。替换 Codex config.toml 中原来的整个 codepet_tasks 条目，移除 command/args，然后在新对话中使用：

```toml
[mcp_servers.codepet_tasks]
url = "http://127.0.0.1:47623/mcp"
http_headers = { Authorization = "Bearer <从 App 复制的本机凭据>" }
```

不要直接复制占位凭据；App 提供的是可用值。URL、http_headers 是 [Codex 支持的 HTTP MCP 配置](https://developers.openai.com/codex/mcp/)。HTTP 采用 [官方 Rust SDK](https://github.com/modelcontextprotocol/rust-sdk) 的 StreamableHttpService，保留 2025 客户端兼容，并通过官方 SDK 客户端协商验收。任务工具只需短请求，服务使用无会话 JSON 响应，不维护长对话状态。

服务只监听 IPv4 回环；固定端口被占用会报告启动失败，不跳到其他端口或把其他服务误报为可用。验证 Host/Origin 与本机 Bearer 凭据；不开放跨站浏览器写入。凭据自动保存在当前数据目录 tasks/mcp-http.token，Unix 权限 0600，重启不轮换，不写日志。正常使用只需从 App 复制一次完整配置。

开发可用 `cargo build --manifest-path crates/Cargo.toml -p codepet-task-mcp` 后运行 `codepet-task-mcp --http`；显式端口 0 仅用于隔离验收。打包程序的 `--task-mcp-http` 是无窗口验收入口。旧 `--task-mcp` 和独立程序不带参数仍为 stdio，不再由 App 默认推荐。不要与正常 App 同时争用固定端口。

## 日常使用

项目 AGENTS.md 可加入 `crates/codepet-task-mcp/instructions.md` 的短约定。开发阶段通常只有两次 tasks_save：方案确定/开始实施时登记目标与清单，实施告一段落时登记简短进展、自测、明确关联的提交并设 phase=ready_for_delivery；实际发布后再合并一次 phase=published 与 delivery 版本/发布记录。主状态仅进行中、待交付、已发布、已结束，发布前的排查与验证不切分更多状态。待交付的提交/推送说明由 Rust 从已有 Git 观测生成，明确区分共享工作树和关联 SHA，不增加维护调用。首调 workspace 是项目或 worktree 的绝对路径；后续保留返回 taskId/revision，省略不变字段。无需为每个步骤建任务、反复查询状态或维护整个对话的摘要。

恢复旧目标或复发时才 tasks_find。已结束的任务复发，用 tasks_save 的 reopenReason 新开轮次，旧清单自动沿用但没有通过结果。方案变化时可同时提交 plan；不要先重开再额外调用一次仅复制清单。

threadId 可省略；MCP 标准不会保证传来当前 Codex ID，也不从长对话猜造。显式提供后，Rust 只查询一次本地 state_5.sqlite 的 cwd 并核对仓库。不可核实的引用显示待核实，已知其他项目的引用拒绝关联；缺少引用不会阻塞任务登记。没有扫描 rollout 全文。

## 数据与权限边界

实际位置为 App 路径管理器的 data/tasks/v2，默认 macOS 是 `~/Library/Application Support/code-pet/tasks/v2`，自定义目录从 bootstrap settings 加载。projects.json 保存服务生成的项目 UUID 与规范化仓库路径映射；每个任务在 projects/<projectId>/tasks/<taskId>.json。receipts 保存重试回执，prepared.json 是事务恢复日志，不手工编辑。

MCP 固定写 agent，自测不会成为 human；人工验收和关闭仅通过主窗口 Rust 适配器。验收目标、环境、清单修订和结果分开，结束与通过分开。工作树 dirty 是共享观测；任务提交必须显式登记 SHA，不自动把 HEAD 或分支上的全部提交归给任务。上游比较只用本地缓存，不执行 fetch。

## 失败处理

- revision 冲突：保留草稿，用 tasks_find 读取当前任务并合并所需字段，使用新 revision；不得自动重写用户改动。
- 请求超时：用原参数/operationId 重试。省略 operationId 时服务根据精确请求指纹幂等；不要在重试时更换参数却沿用显式操作键。
- 账本忙：服务短暂重试文件锁；仍失败时不阻塞开发，在下一关键节点重试并说明未登记。
- 仓库搬迁/清理：历史记录仍存在；目录映射自动修复未实现。只更新任务文字允许原工作目录不可读，Git 事实保持未知/历史观测。
- HTTP 服务未启动：保持 App 运行；检查“接入 Codex”报告的端口/凭据错误。端口占用时停止冲突服务后重启；凭据文件损坏时先退出 App，再备份/移除该文件并重新复制生成的配置。切换数据目录需重启并重新复制本机凭据。
- App 未及时显示 MCP 写入：返回窗口时刷新，或点击“刷新任务”；没有持续轮询或依赖前端同步任务状态。

## 已运行验证

以下命令在本机 macOS 执行。真实 HTTP/stdio 测试隔离 HOME/CODEX_HOME 和应用数据，原生 UI 使用临时账本、真实生产 IPC 及样例任务/验收文本，不污染正式账本，不代表样例问题已真实发布。

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
node scripts/codex_workbench_ui_qa.mjs
npx vitest run frontend
npm run build
npm run check:frontend
CODEPET_SIZE_BASE=origin/main npm run check:source-size
TMPDIR=/private/tmp npm run test:webcontent
```

HTTP 传输测试覆盖官方 SDK 客户端、2025 兼容请求、并发写入幂等、同端口重启与凭据稳定、未授权和错误 Host/Origin 拒绝、端口占用时账本仍可用。真实进程验收在没有前端时完成初始化、两工具枚举、读写、重启读取与冲突拒绝。原生 QA 从 Rust 发出 HTTP MCP 写入，再由生产 UI/IPC 核对相同任务 ID 和修订，并验证配置包含 URL/凭据而不含 command。测试客户端直连回环，不继承执行环境代理。

stdio 兼容检查创建/实现/发布三个关键节点写入、紧凑回执、真实 dirty 观测、来源核实、重启幂等、修订冲突与拒绝冒充 human。原生检查四个主状态的筛选、已发布在验收前保持独立、自测与人工验收隔离、结束勾选与可选说明、新轮次回到进行中且沿用清单但未测、旧轮只读、并发冲突保留草稿。布局检查复用原生验收快照，在 light/dark、1280/820/600/390 下检查溢出、动作可见性和焦点返回；另用显式布局样例覆盖 8 条任务、3 项清单、本地/发布不同进度与失败项。验证任务行不超过 100px、验收内容在第一屏、辅助信息默认折叠、键盘展开和失败项优先展开。状态标签覆盖浅蓝/琥珀/绿/灰四种背景，并在浅/暗两套主题检查文字对比度至少 4.5:1、小面积标签及不溢出。该样例仅存在 frontend/qa，不进入产品数据。

证据在忽略目录 `artifacts/task-ledger-qa/`：mcp-report.json、http-report.json、http-embedded-report.json、native/report.json、native/native-window.png、layout/。具体工具定义与回执字节数记录在 mcp-report.json；预算测试防止无意扩大工具面。该测量不是实际模型 token 消耗或自动调用可靠性的保证。

合入共享 UI 重构后的生产前端使用 `npm run check:frontend`，结果为 0 errors；源码长度检查没有超限。此前旧配置的全量 tsc 检查包含测试/QA 和资源声明错误，不作为生产检查通过的替代证据。webcontent 的默认临时目录 `/var` 与返回的 `/private/var` 别名使旧路径字符串断言失败，使用规范临时目录后 11 项通过。直接 cargo 构建沿用原生 QA 的 macOSPrivateApi=false 配置；正常发布应走现有 Tauri 打包入口。

## 尚未验收

Windows 本机运行、安装版退出/重启的人工验收、当前 Codex 对话加载 MCP 后的自主调用行为，以及长对话中的漏登记率。接入不会自动修改用户 Codex 全局配置、替换本地安装版或改写项目 AGENTS；提交与打包结果以 Git 历史和 GitHub Actions 记录为准。

## 本次修复的真实任务

本轮通过已有 stdio MCP 在正式账本登记“任务 MCP 改为本机 HTTP”（回执在忽略目录 `artifacts/task-ledger-qa/current-http-fix-task.json`），关联当前对话与 4 项验收清单。实现完成后通过新 HTTP 接口记录同一任务的里程碑。它是实际开发任务；上文隔离验收任务是样本，二者不可混称。未通过主窗口人工验收前，不记录 human 通过或自动结束。
