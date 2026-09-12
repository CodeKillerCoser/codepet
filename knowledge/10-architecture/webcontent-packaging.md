# 外置 webcontent 与 Rust 可执行文件

## 背景与目标

此前 `frontendDist: ../dist` 把前端构建产物嵌入 Rust 程序，调整 UI 后重新打包会触发编译/链接。正式应用统一采用外置资源目录，允许独立构建、替换前端并复用 Rust 可执行文件。该结构不是测试开关，也不依赖本地 HTTP 服务器。

## 当前实现与证据

- `vite.config.ts` 输出 `webcontent/`；`scripts/webcontent.mjs` 生成资源清单，记录应用标识、包格式、后端 API 契约版本、UI 版本和每个文件的 SHA-256。鞭子音效由 `frontend/lib/sound.ts` 的 Vite URL import 一起构建到资源包，源 WAV 仍存放在 `src-tauri/resources/sounds/`。
- `src-tauri/tauri.conf.json` 设置 `frontendDist: []`，并通过 `bundle.resources` 将 `../webcontent/` 映射为安装资源目录下的 `webcontent/`。两个窗口仍使用 `index.html` 和 `pet.html`。
- `src-tauri/src/lib.rs` 通过 `generate_context!(assets = ...)` 提供自己的资源容器，防止 macro 嵌入前端文件。正式构建替换成 `app::webcontent::WebContentAssets`；`tauri dev` 仍使用已有 Vite 开发地址。
- `src-tauri/src/app/webcontent.rs` 使用 Tauri `Assets` 接口。启动时验证整个资源包并读取到内存，两个窗口共享同代资源；后续请求只查询清单内已校验的数据，保留原 Tauri origin、初始化脚本、MIME 处理和 IPC/capability 行为。

## 决策、备选方案与取舍理由

采用数据目录版本优先、包内资源兜底的正式分发结构。独立 UI 构建不调用 Cargo，更新写入新版本目录；应用启动后持有资源快照，因此需要重启切换 UI，并承担资源常驻内存的成本。

- 继续嵌入资源：单文件分发简单，但每次 UI 更新要重新编译/链接，无法满足独立替换要求。
- 直接替换包内资源：加载路径简单，但会修改签名覆盖的 App 内容，不符合签名保留要求。
- 开发服务器：支持即时刷新，但依赖运行额外进程，不能作为本次要求的正式离线资源结构。

## 加载优先级与分发布局

正式 App 先通过 `settings::current_app_data_dir()` 取得当前数据目录（含用户自定义目录），枚举其中 `webcontent/v1/`、`v2/` 等版本。目录名为 `v` 加不带前导零的正整数，按 u64 数值选择最大值，不能用字典序或文件时间排序。未完成复制的 `.webcontent-stage-*` 不参与选择；不接受版本目录符号链接。

数据目录下没有版本目录时，才通过 Tauri `resource_dir` 读取包内 `webcontent/`。Windows 包内目录位于可执行文件旁；macOS 为 `.app/Contents/Resources/webcontent/`。数据版本存在时不依赖包内资源可用性。读取数据目录失败或最高版本校验失败时直接诊断，不静默跳过或回退。更换应用数据目录后，需要重启才能选择新目录里的版本。

正式安装包包含 Rust 程序和资源目录；不保留正常 UI 的嵌入副本。资源缺失、哈希不符或契约不兼容时记录日志并显示最小诊断页面，避免空白窗口或悄悄运行旧 UI。

## 独立构建与替换

- `npm run build:webcontent` / `npm run build`：只构建前端并写清单，不调用 Cargo。
- `npm run build:bin`：通过 `tauri.bin.conf.json` 关闭前端预构建与资源复制，只构建启用 `custom-protocol` 的正式 Rust 可执行文件，不生成安装包。首次运行前可以将 webcontent 安装到应用数据目录；完整安装包则自带基线资源。
- `npm run tauri build`：完整构建并分发。Tauri build script 会复制/跟踪 bundle resources，因此修改 UI 后不要用它代替独立 UI 命令。
- `npm run install:webcontent -- --data-dir <app-data-dir>`：验证源包，在数据目录 `webcontent/` 下复制到临时目录、再次校验，最后改名为比现有版本更大的 `vN`。不覆盖任何旧版本，不写 App 包内目录；发布前失败只清理本次临时目录。拒绝重叠目录及 App bundle 目标。

安装后重启 App 才切换资源快照。刷新现有窗口不会读取更新后的磁盘文件。可以在 App 运行期间安装新目录，正在运行的窗口仍用旧快照。不要逐个替换散列 JS/CSS，也不要只复制 HTML。回退时移走最高版本目录，重启后选上一版本；所有版本都不存在时才恢复包内基线。

## 版本与边界

`webcontent-contract.json` 是前后端共同的资源包契约，不是知识索引。UI 样式变更可以沿用 `backendApiVersion`；改变 IPC 方法、参数/返回结构或前端依赖的原生能力时需要审查并升级该版本，同时构建新 Rust 程序。UI `version` 取自 `package.json`，启动不要求它等于原生版本。哈希验证发现损坏，不提供签名真实性保证。

macOS 签名覆盖 App 内资源，因此正常 UI 更新只能写数据目录版本，包内 webcontent 保持不变；无须为了更新数据目录的 UI 重签 App。完整原生版本发布仍沿用签名/公证流程。[Apple 代码签名说明](https://developer.apple.com/library/archive/technotes/tn2206/)

## 涉及模块与风险验证

- Rust 资源读取器：拒绝路径穿越及逃出根目录的符号链接；Rust 单元测试覆盖数值版本排序、数据优先/包内回退、路径、完整性、契约、资源快照与错误页面转义。
- 构建/安装脚本：不覆盖旧版本，拒绝源目标重叠及链接别名；Node 测试覆盖自增版本、损坏拒绝、安装新版本绕过损坏旧版本、排除临时目录及保留原生程序。
- Vite、音效与 Tauri 配置：检查产物包含两个入口及音效，运行前端测试与正式 `custom-protocol` 编译检查；安装后验证两个窗口、IPC、字体/样式与音效加载。
- 升级 Tauri 时重新检查 `generate_context` 自定义 assets 参数及 `Assets` API。当前依据锁定的 Tauri 2.11.2 源码实现，不能假设 macro 内部契约永远不变。

## 验证与未知项

验证命令：`npm run test:webcontent`、`npm run build:webcontent`、`npx vitest run`、`cargo check --manifest-path src-tauri/Cargo.toml --features custom-protocol --locked`、`cargo test --manifest-path src-tauri/Cargo.toml --features custom-protocol app::webcontent::tests --lib`。

2026-09-12 Windows 验证：Node 安装测试 9 项、Rust 资源加载测试 8 项、前端测试 160 项通过；前端资源构建、版本一致性检查与 `custom-protocol` 编译检查通过。安装测试覆盖指向 `.app` 的链接别名，验证拒绝操作时不创建包内目录。

`npm run build:bin -- --debug` 成功生成 Windows EXE，未执行前端构建；此检查使用未优化编译验证独立原生构建入口，仍启用正式资源协议。默认 `npm run build:bin` 使用 release 构建，本轮未执行 release 优化构建和安装包验收。

macOS 的签名、公证、安装包资源路径与原生多窗口行为需要在对应系统验收；当前 Windows 环境无法代替这些检查。

## 主工作区合并验证（2026-09-12）

合并到 `v0` 时原生版本为 `0.3.9-beta`。保留 `build:bundle` 中协议检查与 Provider staging，以及包内 `provider-plugins/`、`provider-sdk/` 资源映射；独立 UI 构建仍只生成 webcontent。桌宠已使用 Pet Gateway，因此分组适配 `waiting-input`、`completed`、`interrupted`、`unknown`，保留主工作区的来源状态和本地隐藏行为。

合并验证：`npm run build:webcontent` 通过，产物含 11 个清单文件；`npx vitest run frontend` 的 196 项、`npm run test:webcontent` 的 9 项、`npm run providers:test` 的 3 项通过。使用 `TAURI_CONFIG` 指向原生独立配置后，`cargo check --manifest-path src-tauri/Cargo.toml --features custom-protocol --locked` 通过，仍有既有平台代码 unused 警告。

全仓 `npx vitest run` 会误收 Node/Bun 测试及原型测试，出现 5 个套件错误；前端应使用上述限定目录命令，其他脚本使用各自测试运行器。本轮没有运行主工作区 release 构建、原生窗口交互或 macOS 签名验收。
