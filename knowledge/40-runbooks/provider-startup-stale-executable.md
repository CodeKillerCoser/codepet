# Provider 启动被旧安装路径干扰

## 现象

Windows Code Pet 启动后 Codex 卡片显示“配置无效”，提示 Select an existing native executable，路径指向已移除的安装 ID。

## 证据

2026-09-09 本机 settings.json 的 agentRuntimes.byProvider.dev.codepet.codex 保存了 8e5b6932251c2c1c/codex.exe；Test-Path 返回 False。当前 fd4c151a749f3ab4/codex.exe 存在，--version 返回 codex-cli 0.153.4。SDK 接受合法 Windows verbatim 路径，不能仅凭 `\\?\` 前缀推断路径语法错误。

源码中 Host 启动经 CODEPET_RUNTIME_EXECUTABLE 注入保存路径，并在首轮 inventory 事件后调用 runtime.select 恢复它。Tauri runtime_views 又依据历史配置和 selected 是否存在判定“配置无效”。Provider 原有后台扫描仍执行，但历史路径污染诊断与展示。

## 根因

上次选择被当成跨 Provider 生命周期的约束，不符合每次启动由 Provider 自主扫描、返回 Host 的预期。安装 ID 更换后，历史绝对路径不再有效。

## 引入历史

首次引入提交未确认。旧行为记录在 windows-provider-runtime 的 2026-09-07 启动章节；2026-09-09 用户明确要求取代该行为。

## 修复方案与涉及模块

2026-09-09 最终规则由用户明确指定：Provider 自己保存 lastSelected；扫描后能匹配就选上次的，否则选最高版本。此前“仅本次选择、不持久化”的中间方案已被取代。

- Provider 数据库：codepet-provider-data 新增 runtime_selection 单行表，存规范化 executable 路径。每个 Provider 的数据库独立，既有数据库以 CREATE TABLE IF NOT EXISTS 迁移。未分配目录的协议 fixture 仍允许无持久存储运行。
- Provider SDK local_runtime：扫描时加载数据库记录并重新探测路径，记录指向已移除文件时不会产生阻断错误。匹配兼容路径优先，否则按 semver 选最高版本；预发布版本参与 semver 比较，build metadata 不影响优先级，同版本按规范化路径确定性排序。无法解析的版本低于可解析版本；仍遵守最低版本兼容性标记。成功选择写库后才发布结果，数据库错误以扫描诊断返回，禁止宣称选择已保存。
- 三个 Provider：初始化数据库后为 scanner 注入存储接口；移除额外的 selected_runtime 内存副本，默认实例启动直接使用 scanner.selected。显式实例 executable 仍是底层实例配置接口，通过独立 inspect 校验，不修改 Provider 默认选择或 lastSelected。
- Host manager：不注入、恢复 App 历史路径；新 generation 清除库存缓存，接收 Provider 扫描结果后启动 manifest 实例。
- 协议与 Tauri：runtime.getInstalled 和 runtime.inventoryChanged 始终返回 harnessList、selected；无可用选择时 selected=null，扫描进度与诊断仍保留。selected 必须属于同一份列表，未验证的手动候选不写库、不作为库存 selected 发布。Host 状态从该快照派生。
- 前端：说明上次选择优先、最高兼容版本回退；手动选择交给 Provider 持久化。“重启并重新检测”仍遵循数据库选择规则，已有 Harness 实例不会被一次手动选择直接替换。

该协议字段从 installed 改成 harnessList，Host 与 Provider 必须同步构建更新；生成 Rust、TypeScript SDK 与 canonical fixture 已同步。

## 验证结果

新增回归覆盖路径匹配优先、1.10.0 高于 1.9.0、预发布比较、失效路径回退、无兼容安装、数据库重开与 Provider 隔离、手动选择后重启恢复、删除旧安装后回退以及 selected 属于 harnessList。原生 Harness 验收测试也新增选中项属于返回列表的断言。

- `npm run protocol:check`：21 项通过，生成文件 freshness 通过，包含 harnessList 必填与 selected 必填可空的契约回归。
- `npx --no-install vitest run frontend/lib/providerRuntimes.test.ts frontend/lib/agentRuntime.test.ts`：9 项通过；`npx --no-install vite build` 通过。
- 进程内设置 `TAURI_CONFIG={"bundle":{"resources":[]}}` 后，`cargo check --manifest-path src-tauri/Cargo.toml --lib -j 2` 通过，覆盖应用依赖链下 SDK、ProviderData、Host 与 Tauri。临时覆盖用于缺失的打包资源，未修改生产配置。
- `cargo test --manifest-path sdk/rust/Cargo.toml -p codepet-provider-sdk --lib local_runtime -j 2` 因 pin_project_internal 缺失报 E0463；`cargo test --manifest-path crates/Cargo.toml -p codepet-provider-data --lib runtime_selection -j 2` 因 windows_interface 缺失报 E0463。新增 Rust 测试未执行，不能计入通过项。
- 三 Provider 的 `cargo check --manifest-path crates/Cargo.toml -p codepet-provider-codex -p codepet-provider-claude -p codepet-provider-opencode -j 2` 同样被依赖 tracing_attributes 的 E0463 阻塞。尝试通过应用 manifest 运行 SDK/Data 的测试被 Cargo 拒绝：它们不是该 workspace 的成员且需要 dev-dependencies。
- `git diff --check` 通过。未进行新安装包 GUI、焦点与响应式人工验收。

## 回归防线

遵守 provider-local-runtime-paths 规约。旧 agentRuntimes 字段允许留在用户配置中，但不得重新进入启动或状态判定链。平台发现规则继续由各 Provider 持有；Host 不自行猜测新的安装 ID。

## 未知项

尚未用新安装包完成真实 GUI 启动验收。当前本机安装版不会因工作区源码修改自动更新。macOS 原生发现需要该平台验收。
