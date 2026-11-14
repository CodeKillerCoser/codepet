# Provider 信息展示与本地包热加载

## 背景与实现

2026-11-05，用户要求本机运行时展示 Provider 自有名称/图标和信息，仅在内置列表右下角保留一个检测按钮，并让自定义 Provider 导入后立即加载。此前入口只将目录保存到 settings，必须重启；图标被写死为 CPU，单项和全量检测重复。

- `frontend/lib/ProviderRuntimeInfo.svelte`：名称来自 Provider 握手，icon 来自 manifest，与 Gateway 的来源一致；展示 Provider 版本、默认工作目录、运行时路径及版本，保留候选选择和真实诊断。信息字段没有新增协议假设，缺省字段不伪造。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs`：扩展连接快照元信息；新命令导入后发连接快照，原有 runtime observer 读取新 inventory。`refresh_agent_runtimes` 仅扫描内置 Provider。
- `crates/codepet-host/src/providers/manager/registry.rs`：串行化导入，复制选中 manifest 的包目录到同盘 staging，校验 manifest 和包内 executable 后 rename 到 `<data>/versions/providers/<pluginId>`。只新增 ID，拒绝已有 ID/目录；不会停止或替换已有 Provider。
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


## Provider 卡片与 UI 热替换（2026-11-12）

按用户标注将本机运行时改为 `repeat(auto-fill, minmax(min(320px, 100%), 1fr))` 网格，分组标题维持小字号。卡片展示名称、类型、在线状态和插件版本；执行环境以用户文案替代 Harness。运行时选择保留原生 select，透明覆盖在无边框路径文本之上；目录前缀按可用宽度省略、保留文件名，title 可查看完整路径，键盘选择和焦点轮廓保留。通知观测、自管运行时不显示冗余运行时行。

`providerPresentation.ts` 只做界面分类，不新增协议字段。内置已知来源标为智能体，自定义通过已在线 ready 实例的 `provider.describe` 能力识别；读取并行，过期结果丢弃，不唤醒未启动实例。不将 not-required 推断成通知观测：Echo/Pi 仍是智能体。当前 Gateway 尚无统一 Provider kind 字段，无法确认时显示“扩展服务”；通知观测通过显式 kind 或宣告的 observation/notification 能力展示，现有协议不因此新增方法。

19 项前端测试与正式 Vite build 通过；模拟 IPC 在 1280 和 600 像素检查多列/单列、路径和通知卡，无横向溢出。实际运行的是用户从修复包打开的 App，原生 API 13，进程 PID 12592；通过“关于 → 加载最新”切换到 data/versions/webcontent/v6，builtAt=1794498338888。主窗口已回到本机运行时并实际展示新卡片，后端进程保持。最初按 /Applications 的旧版本生成的 API 12 资源未加载，已移到 CodePet-builds/provider-ui-preview-20261112 下归档。未修改原生程序或协议契约，不自动重启 App。

网格规则补充：使用 auto-fill 保留空列，两个分区必须仅由容器宽度决定列数；auto-fit 会按各分区卡片数量折叠空列，导致内置两张与自定义四张的卡片宽度不一致。已构建并热加载 v7（builtAt=1794499079999），原生窗口确认上下两区列宽、列起点一致，内置区剩余列留空。

2026-11-13，卡片统一为 border-box 280px 高度，跨内置/自定义及不同内容量保持一致。主网格列显式 minmax(0, 1fr)，避免长路径的最小内容尺寸产生横向溢出；过长内容仅纵向滚动。正式构建通过，热加载 v9（builtAt=1794499419972），原生窗口确认卡片等高且无横向滚动条。


## 完整卸载与统一 Registry（2026-11-13）

插件级安装、活动快照查询和卸载统一放在 manager/registry.rs；PluginManager 保持现有对外 API，并持有唯一活动插件表。ProviderInstanceRegistry 负责稳定身份和历史关联，不作为第二份活动清单；UI、Gateway provider.list 和 protocol.ping 读取 Registry 的 snapshots。

卸载由 Registry 串行协调：验证受管包归属，获取该插件生命周期锁，等待优雅退出或确认强制终止，关闭该插件全部本地订阅，删除安装包目录，注销活动实例和插件，最后发布状态与运行时清单变更。包删除在 blocking worker 中进行，不持有全局插件表锁；其他 Provider 请求继续并行。此处是安装包物理删除，不再新建 removed-providers 归档。保留用户历史与数据库；此前已有归档不属于本次安装包。

订阅操作复用每个插件的生命周期锁，替代跨 Provider 的全局订阅锁，防止卸载时迟到的订阅写回。旧 RPC 由进程 shutdown 关闭，旧 generation 的事件被丢弃；在未确认进程终止时不删除安装包或伪报成功。文件删除/注册表清理失败会报告错误并保留可重试的停止项，文件已经缺失时可继续完成注销。

卸载完成后，桌面成功响应移除对应卡片并刷新；下一次 protocol.ping 从同一活动表生成 providers，自然不再包含已卸载实例，不断开其他手机/桌面客户端。
