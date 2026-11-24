# Codex 工作台数据溯源与验收

## 适用场景

项目缺失、会话关联数量异常、Git 状态不符，或需要判断当前版本是否已完成交付验收。先确认数据来源与范围，再核对视觉；示例截图的数字不能作为真实扫描结果。

## 数据链路

| 界面内容 | 真实来源 | 用途与边界 |
| --- | --- | --- |
| 项目选择 | `CODEX_HOME/state_5.sqlite` 的 projects/project_roots；后备 global state | 确认扫描哪些项目根目录。选择和刷新只改变草稿 |
| 工作树 | 项目根目录的 `git worktree list --porcelain -z` | 包含主工作区、派生工作树及失效登记，无关联对话也应可检查 |
| 未提交 | 每棵树的 porcelain v2 changes | 暂存、工作区和未跟踪条目；不是任务数。目录按 Git 给出的条目计数 |
| 待推送 / 落后 | `branch.ab` 与本地上游引用 | 不访问远端。只有可比较引用才判断；上游配置已失效不能标记同步 |
| 关联标题与归档 | Codex threads、session index、普通/Zstd rollout 头 | 工作目录标准化后精确关联，子目录通过 Git 顶层核实；排除子 Agent |
| 对话活动 | 关联主对话最新更新时间 | 包含归档，按本地日历日筛选，不表示文件修改时间 |
| 文件预览 | 扫描 HEAD 与实时工作区或 index 版本 | 只读、UTF-8、256 KiB；只允许快照列出的条目 |

## 排查步骤

1. 如项目能展开却无法选中，先运行双引擎 [项目选项点击验收](codex-project-picker-click.md)，区分前端 click 丢失与项目目录读取失败。打开工作台“数据来源”，记录实际目录及项目来源是否使用后备；确认 `CODEX_HOME` 是否与预期一致。
2. 核对草稿与已扫描范围。重置只清结果筛选；新工作树/对话要重新扫描，单树重检只更新 Git。
3. 清除项目、日期、关键词与状态筛选。不要因无对话而隐藏有改动或失效的树；使用“仅看失败工作树”定位失效登记。
4. 数量不符时分清总览基础范围、状态进一步过滤、归档详情开关和每批 30 条的显示限制。多对话共享一份 Git 状态，不能把文件归给某条会话。
5. 运行只读探针：`cargo run --manifest-path crates/Cargo.toml -p codepet-task-lineage --example workbench_probe`。默认仅输出 Code Pet 项目的扫描计数，不打印用户标题或源记录。
6. 数据库不可读时，验证后备会话不只恢复第一份。当前工作台仅扫描 Codex。对应测试：`cargo test --manifest-path crates/Cargo.toml -p codepet-task-lineage --lib workbench -- --test-threads=1`。

## 交付门槛

- 前端：`npx vitest run frontend/styles.test.ts frontend/lib/codexWorkbench.test.ts frontend/lib/navigation.test.ts`、`node scripts/codex_workbench_ui_qa.mjs`、`npm run build`。
- 原生构建：`npm run build:bin -- --debug`。Tauri CLI 会按平台更新 manifest feature；本轮验证后恢复其自动修改，避免把本机生成的配置混入功能变更。
- 系统入口：用受控工作树核对打开目录、文件定位、终端当前目录，以及真实 UUID 对应的 Codex 会话。OS 接受打开请求不等于目标界面已正确显示。
- 安装包：在正常打包资源准备后构建并安装 `.app`，关闭开发服务核对本地资源、偏好持久化和离线扫描。开发二进制构建成功不代表本项已通过。

## 当前对话的原生验收

运行 `node scripts/codex_workbench_native_qa.mjs`。此入口构建独立验收窗口，直接使用生产 `CodexWorkbench.svelte`、`codex_workbench.rs` 与工作台扫描 crate，经真实 Tauri IPC 读取本机 Codex 和 Git，没有 mock 会话或快照。仅偏好写入临时目录，不启动 Provider Host、不替换或退出已安装 App。QA 入口不属于生产 Vite 构建输入，QA 命令只注册在独立 example。

脚本使用当前 `CODEX_THREAD_ID` 与当前工作区，通过 Git common-dir 找到唯一 Codex 项目。基线来自 Git porcelain v1，生产解析器使用 v2；重命名的 NUL 原路径独立跳过。原生窗口点击扫描、选中当前工作树，并验证当前 thread ID 的关联、界面未提交标签、重检和实际 UTF-8 差异预览。截图由 WKWebView 自身生成，随后转换为 PNG。

2026-11-24 本机结果：

- 目录：`/Users/wangxin/.codex/worktrees/a1e0/codepet`，处于游离 HEAD。
- 生产扫描与 Git 基线均为 38 项改动，界面显示“未提交 38”和“38 项未提交”。此数字包含验收工具新增文件，是该时刻状态，不是固定产品值。
- `01a2092b-218a-7010-bf11-2c38a134fb5c` 正确关联到此工作树，关联数量为 1。
- 单树重检通过，`frontend/lib/CodexWorkbench.svelte` 的真实文件预览通过。
- 报告：`artifacts/codex-workbench-qa/native/report.json`；截图：同目录 `native-window.png`。产物位于忽略目录，不随源码提交。
- `/Applications/Code Pet.app` 当时仍运行旧安装包，其资源清单后端 API 为 20；原生验收时开发版为 22；合入主干时因 22 已被另一项原生变更使用，最终工作台契约提升为 23。上述结果证明新版生产组件与 IPC 已原生跑通，不代表旧安装包自动升级。需要安装新版后再核对正式入口及系统动作。

## 当前证据与缺口（2026-11-24）

核心数据测试、前端 QA、真实只读探针、生产前端、开发二进制构建，以及当前对话的原生扫描/重检/预览验收通过。安装包与实际系统动作尚未验收，不应写“全部交付完成”。Windows/Linux 原生链路也未实测。
