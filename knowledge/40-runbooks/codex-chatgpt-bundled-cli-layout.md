# ChatGPT 内置 Codex 新目录漏检

## 现象与证据

2026-11-05 用户报告本机安装 ChatGPT，但 Code Pet 未发现 Codex；没有安装 Claude Code/OpenCode，却显示 Provider 在线及 runtime-not-found。首次排查仅定位；后续修复与验证记录见文末。

本机检查结果：

- `/Applications/ChatGPT.app/Contents/Resources/codex` 不存在。
- 原生可执行文件存在于 `/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex`，运行 `--version` 返回 `codex-cli 0.166.0-alpha.5.1`；`app-server --help` 成功。此证据不代表完整 App Server 协议兼容性已经验证。
- `Contents/Resources/codex-cli/bin/codex` 是 shell 包装入口，会转发到上述原生文件。
- 用 `PATH=/usr/bin:/bin:/usr/sbin:/sbin` 启动 `/bin/zsh -ilc`，shell 正常退出，最终 PATH 中没有 ChatGPT 内置 Codex 目录，`command -v codex/claude/opencode` 均无结果。当前聊天执行环境的 `command -v codex` 则能找到应用内原生文件，不能用此环境代表 Finder 启动的应用。
- 安装版 Code Pet 为 `0.3.9-beta+85e6306`。进程表显示它启动了三个内置 Provider 可执行文件；这些是 Code Pet 自带插件，不是三个工具的安装证据。

## 定位结论

`crates/providers/codepet-provider-codex/src/provider.rs::discover_codex_candidates` 已列出 Codex.app 与 ChatGPT.app，但 macOS 固定候选只拼接 `Contents/Resources/codex`。因此问题不是完全不认识 ChatGPT 名称，而是没有覆盖新嵌套目录。SDK `local_runtime::discover` 只枚举当前及登录 shell PATH；普通 shell 未配置该路径时不能兜底。本机路径检查与版本探测支持这条漏检原因；未对运行中的安装版发起 inventory RPC 或修改其配置。

`crates/codepet-host/src/providers/manager.rs::start_enabled` 按 manifest 的 enabled 启动插件。`ProviderHostState::connection_views` 展示插件连接状态；`runtime_views` 单独读取安装列表，找不到兼容候选时生成 runtime-not-found。这解释了 Provider 在线和运行时缺失同时出现，尚未实现用户要求的“未安装则隐藏并不加载”。

## 建议修复范围与验证

- Codex Provider：保留旧路径，补充 ChatGPT/Codex 应用的新原生 CLI 相对路径；优先直接探测原生文件，按规范化路径去重。不把应用品牌变化等同于 CLI 协议变化。
- SDK 共享发现与进程层：继续复用既有验证、超时、PATH 和平台边界；不要在前端或 Host 散落产品目录判断。
- Provider 启动与 UI：自动发现后加载及隐藏未安装项是另一项生命周期调整，不能仅通过隐藏错误冒充安装检测已修复。
- 回归：临时目录覆盖旧/新 bundle、仅新路径、缺失文件及重复入口；GUI 精简 PATH 下运行 inventory-only 原生测试。随后验证当前 Codex 的 App Server 握手，避免把 `--help` 成功当成协议全部兼容。

## 已执行与限制

执行了源码/文档检索、应用目录及进程只读检查、登录 shell PATH 探针、原生 CLI `--version` 和 `app-server --help`。未启动模型请求，未改本机配置、安装应用或检测代码；未执行编译测试。尚未证明整个文件系统中绝不存在 Claude/OpenCode，仅确认上述发现路径中不可见。


## 修复结果

Codex Provider 增加 macOS bundle 候选函数，Codex.app 和 ChatGPT.app 均保留 `Contents/Resources/codex`，同时枚举 `Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex`。环境变量、当前 PATH、登录 shell、npm 与 Windows 发现分支保持原有逻辑。使用原生 CLI 而非额外探测 shell 包装入口，继续由 SDK 规范化去重、验证版本和处理超时。该辅助函数与测试限定于 macOS，未新增跨模块 API 或 re-export。

前端 `agentRuntime.ts::shouldShowRuntime` 只隐藏内置三个 Provider 中 status=unavailable、无路径/安装候选且诊断为 runtime-not-found 的条目。初始化、扫描失败、Provider 连接失败、手动路径和不兼容安装保留展示，自定义 Provider 不过滤。App.svelte 使用此投影，不修改底层 inventory；仍有全量检测入口，避免隐藏后无法发现新安装。此变更是列表显示规则，不停止用于探测的 Provider 插件进程。

回归验证：

- `CARGO_TARGET_DIR=src-tauri/target cargo test --manifest-path crates/providers/codepet-provider-codex/Cargo.toml --lib`：80 通过，1 个真实安装测试默认忽略。目录用例覆盖旧 Codex、旧 ChatGPT、新 CLI 共存、仅新路径、软链接去重及完全缺失。
- 同命令筛选 `native_chatgpt_discovery_with_minimal_path -- --ignored --nocapture`：通过。子进程删除 CODE_PET_CODEX_BIN，PATH 限定系统目录，真实 discover_codex_candidates 发现本机 ChatGPT 内 `0.166.0-alpha.5.1`。此测试只读取 inventory，不选择持久化配置、不启动会话或发送模型请求。
- `npm run build`：通过；`npx vitest run frontend/lib/agentRuntime.test.ts frontend/lib/providerRuntimes.test.ts frontend/connections.test.ts`：19 通过。
- 浏览器 QA `runtimeCase=chatgpt` 只显示 Codex，Claude/OpenCode 隐藏，保留检测其他运行时入口；`runtimeCase=missing` 显示空状态和重新检测按钮。该页面使用模拟 IPC，与上述真实原生发现验证分开。

上述回归阶段未替换安装版应用；后续安装验证见下节。未执行完整 App Server 会话或 Windows 实机验收。

## 主干发布与本机安装验收

2026-11-05，修复提交 `970578b` 已推送至 `origin/main`。执行以下命令完成原生 release app 构建（本机未安装 bun，通过 npm 临时提供）：

```sh
CARGO_NET_GIT_FETCH_WITH_CLI=true npm exec --yes --package=bun -- npm run tauri -- build --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

构建成功，产物为 `src-tauri/target/release/bundle/macos/Code Pet.app`，通过 `codesign --verify --deep --strict`。退出旧应用后，将旧版本备份到 `/Users/wangxin/Documents/CodePet-backups/runtime-fix-970578b/`，替换 `/Applications/Code Pet.app` 并重启。安装后 Host、Codex Provider 和 webcontent manifest 的 SHA-256 均与构建产物一致。构建工具自动修改的 Cargo feature 已恢复，未混入提交。

真实安装版「连接接入 → 本机运行时」验收结果：

- 自动显示 Codex，路径为 `/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex`，版本 `0.166.0-alpha.5.1`。
- Claude 和 OpenCode 未显示；Codex 显示 Provider 在线，Harness 已停止。
- 点击「重新检测」后路径和版本仍正确，未出现 runtime-not-found。
- 只读查询 Provider SQLite：替换前 runtime_selection 无记录，替换后自动记录上述新路径；验收未手动注入运行时配置。
- 原生界面截图保存在 `/Users/wangxin/Documents/CodePet-backups/runtime-fix-970578b/installed-runtime.png`。

本轮验证覆盖实际安装、自动发现与列表显示，不代表完整模型会话或 Windows 实机兼容性已验证。
