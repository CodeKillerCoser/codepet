# 外置 webcontent 与 Rust 可执行文件

## 背景与目标

此前 `frontendDist: ../dist` 把前端构建产物嵌入 Rust 程序，调整 UI 后重新打包会触发编译/链接。正式应用统一采用外置资源目录，允许独立构建、替换前端并复用 Rust 可执行文件。该结构不是测试开关，也不依赖本地 HTTP 服务器。

## 当前实现与证据

- `vite.config.ts` 输出 `webcontent/`；`scripts/webcontent.mjs` 生成资源清单，记录应用标识、包格式、后端 API 契约版本、UI 版本、构建时间戳 builtAt 和每个文件的 SHA-256。鞭子音效由 `frontend/lib/sound.ts` 的 Vite URL import 一起构建到资源包，源 WAV 仍存放在 `src-tauri/resources/sounds/`。
- `src-tauri/tauri.conf.json` 设置 `frontendDist: []`，并通过 `bundle.resources` 将 `../webcontent/` 映射为安装资源目录下的 `webcontent/`。两个窗口仍使用 `index.html` 和 `pet.html`。
- `src-tauri/src/lib.rs` 通过 `generate_context!(assets = ...)` 提供自己的资源容器，防止 macro 嵌入前端文件。正式构建替换成 `app::webcontent::WebContentAssets`；`tauri dev` 仍使用已有 Vite 开发地址。
- `src-tauri/src/app/webcontent.rs` 使用 Tauri `Assets` 接口。启动时验证整个资源包并读取到内存，两个窗口共享同代资源；后续请求只查询清单内已校验的数据，保留原 Tauri origin、初始化脚本、MIME 处理和 IPC/capability 行为。

## 决策、备选方案与取舍理由

采用包内与数据目录资源共同参与比较的正式分发结构。独立 UI 构建不调用 Cargo，更新写入新版本目录；应用启动后持有资源快照，因此需要重启切换 UI，并承担资源常驻内存的成本。

- 继续嵌入资源：单文件分发简单，但每次 UI 更新要重新编译/链接，无法满足独立替换要求。
- 直接替换包内资源：加载路径简单，但会修改签名覆盖的 App 内容，不符合签名保留要求。
- 开发服务器：支持即时刷新，但依赖运行额外进程，不能作为本次要求的正式离线资源结构。

## 加载优先级与分发布局

正式 App 通过 Path Manager 取得包内资源根与当前 data 根，收集包内 `webcontent/` 和 data 下全部 `versions/webcontent/vN/`。目录名 vN 仅代表安装槽位，不能用来判断 UI 新旧；未完成复制的 `.webcontent-stage-*` 不参与选择，不接受版本目录符号链接。

每个候选先验证应用标识、后端契约、资源入口、版本/时间戳和全文件哈希。完整且兼容的候选一起排序：先比较清单 `version` 的 SemVer 优先级，再比较 `builtAt`（Unix 毫秒）；版本与时间相同优先包内，两份数据资源完全相同则取更高槽位。SemVer 的 build metadata 不影响优先级，正式版高于同版本预发布版。

不兼容、损坏或缺少构建元数据的候选会记录诊断并跳过；其他位置仍有可用资源时正常加载。全部不可用才显示最小诊断页。包内目录无法解析、数据目录无法读取也记录原因，不阻止另一个位置的有效资源参与选择。两个窗口共享最终资源快照，重启才重新比较。

Windows 包内目录位于可执行文件旁；macOS 为 `.app/Contents/Resources/webcontent/`。正常 UI 更新只写数据目录，不修改已安装 App 包或签名。
## 独立构建与替换

- `npm run build:webcontent` / `npm run build`：只构建前端并写清单，不调用 Cargo。
- `npm run build:bin`：通过 `tauri.bin.conf.json` 关闭前端预构建与资源复制，只构建启用 `custom-protocol` 的正式 Rust 可执行文件，不生成安装包。首次运行前可以将 webcontent 安装到应用数据目录；完整安装包则自带基线资源。
- `npm run tauri build`：完整构建并分发。Tauri build script 会复制/跟踪 bundle resources，因此修改 UI 后不要用它代替独立 UI 命令。
- `npm run install:webcontent -- --data-dir <app-data-dir>`：验证源包，在数据目录 `versions/webcontent/` 下复制到临时目录、再次校验，最后改名为比现有版本更大的 `vN`。不覆盖任何旧版本，不写 App 包内目录；发布前失败只清理本次临时目录。拒绝重叠目录及 App bundle 目标。

安装后重启 App 才切换资源快照。刷新现有窗口不会读取更新后的磁盘文件。可以在 App 运行期间安装新目录，正在运行的窗口仍用旧快照。不要逐个替换散列 JS/CSS，也不要只复制 HTML。回退时移走当前选中的数据资源目录，重启后从剩余数据资源和包内资源重新比较。

## 版本与边界

`webcontent-contract.json` 是前后端共同的资源包契约，不是知识索引。UI 样式变更可以沿用 `backendApiVersion`；改变 IPC 方法、参数/返回结构或前端依赖的原生能力时需要审查并升级该版本，同时构建新 Rust 程序。UI `version` 取自手动维护的 `package.json`，构建不自动递增，启动不要求它等于原生版本。`builtAt` 由每次前端构建的清单生成步骤自动填入 `Date.now()`；安装复制保留该值，不使用文件修改时间、复制时间或目录编号。哈希验证发现损坏，不提供签名真实性保证。

macOS 签名覆盖 App 内资源，因此正常 UI 更新只能写数据目录版本，包内 webcontent 保持不变；无须为了更新数据目录的 UI 重签 App。完整原生版本发布仍沿用签名/公证流程。[Apple 代码签名说明](https://developer.apple.com/library/archive/technotes/tn2206/)

## 涉及模块与风险验证

- Rust 资源读取器：拒绝路径穿越及逃出根目录的符号链接；Rust 单元测试覆盖SemVer 排序、包内与数据共同比较、同版本构建时间比较、异常候选跳过、路径、完整性、契约、资源快照与错误页面转义。
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

## 资源比较规则更新（2026-09-12）

按用户修正规则，包内与数据目录共同参与选择，版本手动维护、构建时间自动记录。旧的“存在 vN 就不看包内”和“损坏最新槽位直接失败”规则已被替换。

本轮 `npm run test:webcontent` 10 项通过；`cargo test --manifest-path src-tauri/Cargo.toml --features custom-protocol app::webcontent::tests --lib --offline -j 2` 10 项通过。测试覆盖高版本包内胜出、数据 SemVer 数值排序、同版本构建时间、完全相同优先包内、预发布版排序，以及损坏/契约不符/缺少元数据的候选。测试编译期间修正 Provider 单元测试夹具中错误引用未声明 settings 的路径设置，使用其临时 provider-data 目录。

`npm run build:webcontent` 通过；11 个资源文件已安装至用户 data/versions/webcontent/v3，清单版本为 0.3.9-beta，builtAt 为 1789227718898，安装保留了构建时间。

正式原生构建：`npm run build:bin -- -- --offline --jobs 2` 成功，release 优化构建耗时 7m21s，产物为 `src-tauri/target/release/code-pet.exe`。未替换已安装程序，也未生成安装器；macOS 实机与原生窗口启动验收仍待执行。

2026-09-13 补充完整 Windows 安装包：`npm run tauri -- build --bundles nsis --config '{"bundle":{"createUpdaterArtifacts":false}}' -- --offline --jobs 2`（本地构建设置 CARGO_BUILD_JOBS=2、CARGO_NET_OFFLINE=true）。协议检查 22 项通过，三个 Provider、webcontent、Rust release 与 NSIS 构建成功。产物 `src-tauri/target/release/bundle/nsis/Code Pet_0.3.9-beta_x64-setup.exe`，53,031,189 字节。仅本次命令关闭 updater 签名附件生成，仓库发布配置不变；未执行安装器。包内 UI builtAt=1789228635451，高于数据目录 v3，同版本时应选包内资源。
