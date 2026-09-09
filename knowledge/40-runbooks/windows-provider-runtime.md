# Windows Provider 发现与数据目录验证

> 2026-09-09 启动语义更新：每次 Provider 启动自主扫描，由 Provider 数据库保存 lastSelected，匹配优先、最高兼容版本回退；Host 不再保存或恢复 executable 选择。下文 2026-09-07 的 CODEPET_RUNTIME_EXECUTABLE 注入与选择恢复是历史实现；当前行为与验证见 [Provider 启动被旧安装路径干扰](provider-startup-stale-executable.md)。

## 现象

Windows 已能在终端运行 Harness，但 Provider 显示未安装，或历史数据为空。审阅基线是 v0 / ca78541；修复涉及 Provider SDK 的 local_runtime、三个 Provider 的配置/启动链，以及前端路径 API。

## 证据

2026-09-06 本机 `Get-Command -All`、`where.exe` 和 npm package.json 显示：

| Harness | PATH 入口 | 实际可执行文件 | 实测版本 |
| --- | --- | --- | --- |
| Claude Code | npm 的 claude.ps1 / claude.cmd / Unix shim | npm/node_modules/@anthropic-ai/claude-code/bin/claude.exe | 2.1.209 |
| OpenCode | npm 的 opencode.ps1 / opencode.cmd / Unix shim | npm/node_modules/opencode-ai/bin/opencode.exe | 1.18.29 |
| Codex | 原生 codex.exe | LOCALAPPDATA/OpenAI/Codex/bin/<installation-id>/codex.exe | 0.153.4 |

表中的斜线用于展示结构；实现用 PathBuf 构造。安装 ID 是目录名，不是版本排序或支持范围。

三个 provider 的 native_runtime 首轮测试均完成自动发现、显式选择、重新读取选择、隔离数据目录启动、列会话、停止与 shutdown。OpenCode `debug paths` 另确认自定义根下的 config/opencode、data/opencode、cache/opencode、state/opencode 生效。Codex 在临时目录中拒绝创建 PATH helper aliases，但 App Server 可正常 ready/list；隔离目录无登录信息，因此账号额度探测返回未认证。

## 根因

旧发现只检查无扩展名文件，并在 Windows 尝试 Unix 登录 shell；Claude/Codex 数据目录回退只识别 HOME；Claude 自定义目录没有传入发送进程；OpenCode 存在精确版本限制。

## 引入历史

未确认。

## 修复与配置

运行时选择只保存真实可执行文件位置。自动发现包括 PATH、Unix 登录 shell、Windows npm bin 元数据、Provider 自有安装布局；可通过 CODE_PET_CODEX_BIN、CODE_PET_CLAUDE_BIN、CODE_PET_OPENCODE_BIN 指定候选。Windows 不执行登录 shell；Codex 支持 LOCALAPPDATA 的独立安装和 Store 包布局。

数据目录通过 Provider manifest 的实例 settings 配置，与 executable 字段独立。例如：

```json
{
  "settings": {
    "serverArgs": ["serve"],
    "dataDirectory": "D:\\CodePetData\\opencode"
  }
}
```

以上是 OpenCode 实例 settings 片段。Codex 使用 appServerArgs，Claude 不需要 serverArgs。Claude 的既有 claudeConfigDir 仍受支持，它与 dataDirectory 是同一个字段的别名，不能同时提供。Codex 也接受 codexHome 别名。

- Codex：dataDirectory 是 CODEX_HOME，App Server 与项目归属读取都使用该目录。
- Claude：dataDirectory 是 CLAUDE_CONFIG_DIR，账号探测、历史扫描和发送都使用该目录。
- OpenCode：dataDirectory 是隔离存储根，子进程使用其 config/data/cache/state 作为 XDG roots；模型与账号 CLI 探测保持相同上下文。显式存储根会移除继承的 OPENCODE_CONFIG、OPENCODE_CONFIG_DIR、OPENCODE_CONFIG_CONTENT，以免混入其他配置。
- 未配置 dataDirectory 时保持原生默认行为：CODEX_HOME / CLAUDE_CONFIG_DIR / XDG_* 环境变量和平台 home 回退。不要把安装目录设为数据目录。

Provider 的全局 hook observation 观察外部 Harness，独立于远程实例存储。若外部 Harness 使用自定义配置，应在启动它和 Provider 的 manifest env 中设置同样的 CODEX_HOME、CLAUDE_CONFIG_DIR 或 XDG_CONFIG_HOME；不能只改变某个远程实例就假设外部终端进程也被重新配置。

## 验证命令

```powershell
Get-Command codex,claude,opencode -All
where.exe codex
where.exe claude
where.exe opencode
cargo test --manifest-path sdk/rust/Cargo.toml -p codepet-provider-sdk --lib local_runtime
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex -p codepet-provider-claude -p codepet-provider-opencode --lib
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex -p codepet-provider-claude -p codepet-provider-opencode --test native_runtime -- --ignored --nocapture --test-threads=1
```

native_runtime 不发送模型请求；使用临时数据目录，不修改现有配置。Codex 另有空 PATH 的 Windows 安装布局测试。npm shim 的显式路径可以通过 CODE_PET_CLAUDE_BIN / CODE_PET_OPENCODE_BIN 验证；子进程变量只影响当前测试。

## 2026-09-06 验证结果

- 三个 Provider 单元测试：77 项通过；SDK local_runtime：4 项通过。
- native_runtime：三个真实 Harness 的 PATH 发现与启动通过；再次通过环境变量选择（Claude/OpenCode 指向 `.cmd`）验证通过；Codex 空 PATH 的 Windows 安装布局测试通过，共 4 个实机用例。
- Claude provider_vertical：9 项通过；随后增加“发送子进程必须收到 CLAUDE_CONFIG_DIR”的 fixture 断言，定向复跑通过。OpenCode provider_vertical：2 项通过、1 项忽略。
- Codex provider_vertical：40 项通过、1 项忽略、3 项超时。三项是 `provider_binary_forwards_turn_limit_and_truncates_only_tool_text_with_item_metadata`、`provider_binary_pages_large_turns_without_transport_budgeting`、`provider_binary_read_errors_preserve_shared_server_and_other_conversations`。串行复跑仍超时；临时检出未修改的 `ca78541`，执行同样三项，均在 provider_vertical.rs:4033 的 recv_timeout 失败，因此属于基线已存在的问题，未扩大本次修改去调整传输时序。
- `npm run build` 通过；`npx vitest run frontend/lib/activity.test.ts frontend/styles.test.ts frontend/PetApp.test.ts`：79 项通过。
- Tauri 首次测试被 macOS Git 缓存阻塞；依赖隔离后进入构建阶段，随后发现缺少 SDK 打包资源。测试进程使用空资源覆盖时定向路径用例通过；完成 Bun 安装和 staging 后，撤掉覆盖执行 `cargo test --offline --locked --manifest-path src-tauri/Cargo.toml --test event_normalizer_tests`，正常配置下 14 项全部通过。未完成 GUI 验证。
- `npm install --global bun` 安装 Bun 1.4.2，原生二进制 `--version` 验证通过。npm 全局安装只在 PATH 提供 Bun shim，`spawnSync("bun")` 仍报 ENOENT；构建脚本现优先解析 PATH 原生 bun.exe 或 npm 包内的 bin/bun.exe，保留 BUN 覆盖。无需 shell。`npm run providers:stage:dev` 已在不设置 BUN 时通过，三个 Provider 和 SDK generator/协议资源已准备完成。
- `npm run providers:test`：3 项通过，包含 npm Bun 原生定位以及 native/Windows/Linux staging；修复了既有测试将 native target 一律视为 Unix、在 Windows 硬断言 POSIX 执行权限的问题。
- `git diff --check` 通过；临时基线 worktree 已移除。既有用户 Harness 配置未修改。

## 本地 Windows release 构建

先运行 `npm run version:check`。本机 `npm run tauri -- build --bundles nsis` 会依次执行协议 freshness、Provider release staging、Bun SDK generator、Vite 和 Tauri/NSIS。

本地不生成 updater 签名产物时，将 `{"bundle":{"createUpdaterArtifacts":false}}` 写入临时 JSON，并用 `--config <临时 JSON 的绝对路径>` 传入；不要改动仓库的生产 updater 配置，也不要把此安装包当作带 `.sig` 的自动更新发布。设置 `CODEPET_PROVIDER_TARGET=x86_64-pc-windows-msvc`，确保内置 Provider 与主程序架构一致。

如果 Windows 的协议检查误报 stale，先检查头部展示路径和 CRLF。2026-09-06 的修复统一生成头部的斜杠并在 freshness 比较中规范化 CRLF；18 项协议测试通过，包含“换行差异通过、真实内容变化拒绝”。不要靠跳过 `protocol:check` 完成打包。

2026-09-07 本机构建成功：0.1.5 Windows x64 release，NSIS 安装包 `src-tauri/target/release/bundle/nsis/Code Pet_0.1.5_x64-setup.exe` 为 42,242,007 字节，SHA-256 为 `f5fef31a38126ae22db7b57fb7769a1c13a1e654bd87b33c2b571c5c40e3e53d`。主程序 PE 架构和版本通过；NSIS 脚本包含三个 Provider 与 SDK generator，staging 二进制与对应 release 输出哈希一致。完整验证记录位于构建目录 `build-verification.json`。本次没有执行安装或 GUI 冒烟测试。

## 2026-09-07 启动、扫描和进程所有权

### 证据与根因

已安装 App 日志中 instance.create 和 runtime.getInstalled 曾出现 10 秒超时。原逻辑同步执行扫描/版本进程，并在探测后去重。将工作移到 spawn_blocking、给予整轮 8 秒预算仍不够：本机 OpenCode 原生 exe 的独立 --version 曾耗时约 77.8 秒，进程创建阶段本身也可能慢。没有证据把它完全归因于 E 盘。

本轮真实 Host 测试在 PATH 重复 10 次时，Claude/Codex/OpenCode 的快照查询分别约 5/4/6 毫秒返回 scanning=true；随后收到各自 runtime.inventoryChanged，扫描分别约 25/5/32 秒。这些是本轮耗时，不是性能承诺。

### 最终实现

- Provider initialize 启动后台 RuntimeScanner，不等待安装扫描；runtime.getInstalled 默认只读缓存；refresh=true 仅触发后台重扫并立即返回扫描状态，不重启 Provider 或中断现有 Harness。先规范化、去重，再最多并发 4 个候选探测；每个探测的执行预算为 120 秒，无法强行中断尚未返回的系统进程创建调用。
- 扫描完成通知携带 installed、selected、scanning、scanError。手动选择未扫描过的原生文件时先记录配置（version 为空表示待探测），后台验证完成后补充版本或错误。Host 缓存结果，应用已保存的选择，并在同代 Provider 上恢复实例；锁和 generation 检查防止重启期间旧结果恢复新实例。已保存的路径经 CODEPET_RUNTIME_EXECUTABLE 传入子进程，离开 PATH 后仍可参与扫描。
- Tauri 转发 provider-runtime-changed，前端收到后刷新快照；连接状态不变也能退出“检测中”。真实未检测到与仍在扫描分别显示。
- SDK process 提供 Command、Child、ProcessControl、spawn_async。平台实现在独立文件：Windows 隐藏控制台，挂起创建后分配 Job 再恢复；进程级非继承 Job handle 设置 KILL_ON_JOB_CLOSE，主进程崩溃也清理子树。Unix 为每个托管子树建立进程组，通过统一控制接口发信号。正常退出/超时/Drop 都清理所属子树。
- Claude 的读流、等待线程共享 ProcessControl，取消不再运行 taskkill 或按可复用 PID 查找进程。无控制台 Windows 不提供 Unix SIGINT，接口明确返回 Unsupported；终止可用 Job 完成。

### 验证与边界

SDK 测试覆盖 Windows 无控制台、根退出后的后代和继承管道清理、主进程被强杀后的子树清理、等待线程与取消句柄并发；scanner 测试用屏障证明探测并发且单线程 Tokio 可响应，并断言完成通知。前端扫描完成事件回归测试通过。三 Provider lib 测试共 77 项通过；协议生成测试 18 项通过。

真实 CLI 启动/数据目录验证使用各 Provider 的 ignored native_runtime；真实 Host 通知验证使用 codepet-host 的 ignored native_runtime。不要同时重新链接正在该测试中运行的 Provider exe，Windows 会拒绝覆盖运行中文件。

本机为 Windows，macOS 进程组分支与完整 App 交互仍须 macOS 实机验证。Unix 进程组不是 Windows Job 的嵌套替代品；不可宣称任意祖先进程被 SIGKILL 时也能保证跨组子树全部清理。原生工具主动脱离组的情况同样需要单独验证。


补充回归结果：SDK lib 24 项通过（2 项为被父测试实际调用的 ignored 子进程 fixture）；Claude vertical 9 项通过；Host lib 32 项、process_rpc 8 项通过。manager_gateway 首轮 23 项通过、1 项在等待启动标记的 2 秒期限超时，该项独立复跑通过；未修改超时阈值，也未将首轮描述为全绿。前端 API/Provider observer 25 项与 Vite 生产构建通过，Tauri Windows cargo check 通过。新 release 安装包尚未重新生成。


最终补充：加入 refresh=true 与手动新路径后台探测后，local_runtime 8 项通过（另 1 个由父测试调用的 console fixture），三个真实 Provider 的 4 项 native_runtime 再次通过。真实 Host 在同一个 Provider 进程中完成首次扫描与重扫，三者均收到完成通知，整组约 21.5 秒；暖缓存下快照约 2.6–4.3 毫秒。前端 25 项、生产构建、Tauri check 与更新后的协议一致性/生成文件新鲜度检查通过。
