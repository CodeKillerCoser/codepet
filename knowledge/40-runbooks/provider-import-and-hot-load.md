# Provider 信息展示与本地包热加载

## 背景与实现

2026-11-05，用户要求本机运行时展示 Provider 自有名称/图标和信息，仅在内置列表右下角保留一个检测按钮，并让自定义 Provider 导入后立即加载。此前入口只将目录保存到 settings，必须重启；图标被写死为 CPU，单项和全量检测重复。

- `frontend/lib/ProviderRuntimeInfo.svelte`：名称来自 Provider 握手，icon 来自 manifest，与 Gateway 的来源一致；展示 Provider 版本、默认工作目录、运行时路径及版本，保留候选选择和真实诊断。信息字段没有新增协议假设，缺省字段不伪造。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs`：扩展连接快照元信息；新命令导入后发连接快照，原有 runtime observer 读取新 inventory。`refresh_agent_runtimes` 仅扫描内置 Provider。
- `crates/codepet-host/src/providers/manager/import.rs`：串行化导入，复制选中 manifest 的包目录到同盘 staging，校验 manifest 和包内 executable 后 rename 到 `<data>/versions/providers/<pluginId>`。只新增 ID，拒绝已有 ID/目录；不会停止或替换已有 Provider。
- `instance_registry.rs`：导入使用追加注册，保留旧实例 ID、当前 settings 和记录；启动时的全量 catalog 同步仍可 prune。新进程沿用 initialize、实例 create、Host update 和 heartbeat 链路，已连接客户端无需重新配对。
- `webcontent-contract.json`：后端 API 契约升级为 3。此界面依赖新的原生导入命令和连接元信息，必须与新原生程序一起发布，避免旧安装版热加载新 UI 后缺少导入接口。

## 包约束与失败语义

包需要包含 executable 及所需资源；manifest 中 executable 可以是包内相对路径或包内绝对路径，导入统一重写为相对路径。外部 executable、符号链接、特殊文件和包含目标安装目录的源目录明确拒绝。pluginId 用字母、数字、点、下划线和横线，禁止 `.`/`..`；这只是本地安装目录名约束，不改变协议 ID 定义。

复制失败自动移除 staging；实例身份冲突导致注册失败时移除本次安装目录。插件启动失败保留已安装包及错误状态，返回 loaded=false 和 diagnostic，不能显示为成功加载。manifest 的 enabled=false 被保留并提示禁用。包导入不是在线覆盖升级；相同 ID 不会悄悄覆盖正在执行会话的进程。

## 验证与风险

- Host 新增 4 项真实 fixture 子进程测试：运行中导入、原客户端心跳驱动实例启动、Gateway 即时列表与事件、旧 Provider generation/settings 保持、重复 ID 拒绝、重启后 catalog 发现；另覆盖坏 manifest、目录穿越 ID、外部 executable、符号链接、启动失败和 shutdown 后拒绝。
- `npx vitest run frontend/connections.test.ts frontend/lib/providerRuntimes.test.ts frontend/lib/agentRuntime.test.ts frontend/lib/api.test.ts`：37 项通过；`npm run build` 通过。
- `TMPDIR=/private/tmp npm run test:webcontent`：11 项通过。默认 macOS 临时目录测试曾因 `/var` 与 canonical `/private/var` 字符串比较失败；使用真实临时目录后通过，未修改无关测试或资源加载逻辑。
- `CARGO_TARGET_DIR=src-tauri/target cargo test --manifest-path crates/Cargo.toml -p codepet-host --test manager_gateway`：35 项通过。
- 同 manifest `-p codepet-host --lib -- --test-threads=1`：74 项通过。首次并发 lib 测试中进程 FD 数断言失败；串行全套通过，该断言读取进程级 FD 数，不能与其他打开 FD 的用例并发评估。
- Tauri 检查使用 `TAURI_CONFIG='{"app":{"macOSPrivateApi":false}}' cargo check --manifest-path src-tauri/Cargo.toml --lib`，避免仓库既有 macOS 配置与 Cargo feature 的检查冲突，不改源配置。
- 浏览器使用 QA 模拟 IPC，在 1280×720 和 600×800 检查布局、图标、单一检测按钮、键盘焦点和长路径，无横向溢出。截图 `/tmp/codepet-provider-info-desktop.png`、`/tmp/codepet-provider-info-narrow.png`。检测后下拉禁用状态曾暴露 Svelte 间接函数依赖问题，已改为 busyRuntime 的显式表达式并重验。

风险验证：重复导入不能影响活跃进程（generation 对比）；注册不能重置旧配置（更新设置后导入再 restart 对比）；无手机时不提前启动 Harness、已有手机时热加载可启动（现有 SDK 心跳机制及 fixture marker）；失败包不能伪报在线（loaded/diagnostic 断言）。

未进行 Windows 实机导入或用户自定义包验收；浏览器模拟不等于安装版验收，后续原生安装结果见下节。之前 Worktree 权限问题不在本次修复范围内。

## 主干提交与 Release 安装验收

2026-11-05，功能提交 `b13c9e5` 已推送 `origin/main`。使用 `CARGO_NET_GIT_FETCH_WITH_CLI=true npm exec --yes --package=bun -- npm run tauri -- build --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'` 构建 macOS ARM64 Release App，版本 `0.3.9-beta`，webcontent 后端契约为 3。构建成功并通过 `codesign --verify --deep --strict`；使用本机 ad-hoc 签名，未做公证。Tauri CLI 自动加入的 Cargo feature 已恢复，未混入提交。

旧 App 正常退出后，备份到 `/Users/wangxin/Documents/CodePet-backups/provider-hotload-b13c9e5/`，替换 `/Applications/Code Pet.app` 并重新启动。安装后的 Host、Codex Provider 与 webcontent manifest SHA-256 分别与构建产物相同。新进程从 `/Applications/Code Pet.app` 启动。

原生界面验证：Codex 显示 manifest 图标和握手名称、Provider 版本 `0.1.0`、ChatGPT 内置 CLI 路径、运行时版本 `0.166.0-alpha.5.1` 与默认工作目录；Provider 在线、Harness 可用。仅一个列表级重新检测按钮，点击后结果正常，未安装的 Claude/OpenCode 未显示。截图保存为上述备份目录中的 `installed-runtime.png`。真实用户自定义包导入及手机新建会话未在此次安装验收中执行。
