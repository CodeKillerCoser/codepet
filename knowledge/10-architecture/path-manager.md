# Path Manager：安装、数据与工作目录

## 背景

设置模块原先计算数据根，Hook 模块另算默认根，Provider 各自拼接 `.codepet/remote_workspace`，任务抽取又把执行目录放进应用数据。前端读取设置字段并补默认展示路径，无法一次确认实际生效目录。用户要求独立 Path Manager，明确 install、data、workspace。

## 决策

`crates/codepet-paths` 是不依赖 Tauri 的共享路径模块，`src-tauri/src/app/paths.rs` 是桌面接入入口。Provider 只依赖共享 crate，不依赖桌面业务。`PathManager` 返回 install、data、workspace，并提供 resources、settingsFile 和常用子目录。

| 类型 | Windows | macOS | 用途 |
| --- | --- | --- | --- |
| install | 可执行文件所在目录 | `.app` 根目录；未打包时为可执行文件目录 | 已安装程序，日常业务不写入 |
| resources | install | `.app/Contents/Resources` | 包内 webcontent、Provider 与 SDK |
| data | `%LOCALAPPDATA%/code-pet` | `~/Library/Application Support/code-pet` | 设置、日志、数据库、缓存、UI 更新与宠物 |
| workspace | `~/.codepet` | `~/.codepet` | Code Pet 管理的任务执行目录 |

`data.dataDirectory` 是当前路径配置；它仅覆盖 data，不改变 install 或 workspace。设置引导文件固定保存在默认 data 根的 `settings.json`，从而更改 data 后仍能找到配置。设置模块与 Path Manager 共用该 JSON 和同一个 `DataSettings` 类型；Path Manager 只读取路径字段，不重写其他设置。无配置时用默认值，损坏 JSON 返回错误，避免覆盖原设置。旧的路径读取接口及其默认回退已经删除。

## 备选方案与取舍理由

- 独立 `paths.json`：边界直观，但旧 settings 里的 dataDirectory 会产生第二份配置，需要双写同步。本次采用同一 JSON。
- install/data/workspace 都可随设置修改：迁移灵活，但容易把用户工作树随缓存一起搬走。本次 install 来自可执行文件，workspace 固定来自平台 home。
- 每个模块继续拼接系统路径：改动小，但不同进程长期容易出现路径分歧。共享 crate 固化目录规则，各业务只派生本模块文件。

## 影响范围

- 设置加载和保存：直接调用 Path Manager 获取引导文件。settings.rs 不再定义任何目录计算、迁移、复制或清空方法；自定义数据/宠物目录的写入入口也归 Path Manager。切换路径只保存配置并创建目标目录。
- 前端：`path_manager_get` 返回完整路径对象；`getAppPaths()` 调用它；写入统一使用 `path_manager_set_data`、`path_manager_set_pets`，系统设置展示真实 install/data/workspace，不在 UI 猜默认路径。
- webcontent：共同比较 data/webcontent/vN 和 resources/webcontent，选择兼容且完整的最高版本，同版本取最新构建时间。版本选择和完整性检查仍由资源加载器负责。
- Provider 与 Hook：包内 Provider 根由 Path Manager 提供，显式开发资源覆盖保留；Hook 脚本使用当前数据目录。日志、宠物、事件缓冲等直接调用 Path Manager，旧路径函数已删除；Hook 的 Node 适配器读取同一 settings.json，默认事件缓冲改为 data/spool，不保留 `~/.code-pet/spool` 回退。外部 Agent 的 CODEX_HOME、Claude 配置等仍属于其原生目录，不移动到 Code Pet。
- 三个 Provider：共同调用 remote_workspace，保留 `~/.codepet/remote_workspace/<provider>`；用户选择的项目目录和已有会话 cwd 保持原值。
- 任务抽取：记录和 skills 保留在 data，新执行目录位于 `~/.codepet/workspaces/task-extraction`、`workspaces/tasks`。旧执行目录保留，不删除或自动搬移；任务分配的边界校验改为对应工作目录。

## 验证与风险

共享 crate 测试覆盖 Windows 布局、macOS App bundle 资源布局、自定义 data 不影响 workspace/引导文件、共用 JSON 不写回、损坏配置和路径越界。任务管理测试验证记录与执行目录分离，并实际分配 task 文件。

已运行 `cargo test --manifest-path crates/Cargo.toml -p codepet-paths -p codepet-task-lineage --lib --offline`：路径 4 项及任务管理原有 23 项通过；新增隔离测试补齐 schemaVersion 后单独复跑通过。三个 Provider 的 `cargo check` 通过。`npx vitest run frontend` 195 项及 `npm run build:webcontent` 通过。Node 路径适配器测试和 webcontent 的 9 项测试通过。

桌面回归命令：`cargo test --manifest-path src-tauri/Cargo.toml --features custom-protocol --test settings_tests --test pet_library_tests --test spool_tests --test app_log_tests --test hook_script_tests --offline -j 2`，使用 `tauri.bin.conf.json` 配置，40 项全部通过。

## 后续观察

新增 Path Manager 原生命令，因此 webcontent 后端契约升级为 2。旧已安装 App 和 data/webcontent/v1 本轮未替换。发布时需要配套原生程序与契约 2 的完整 UI；已有契约 1 资源目录应先准备更新，否则新程序会按既有资源校验规则显示不兼容诊断。

实际安装包的 macOS 路径、首次启动、切换数据目录后的日志/Hook 和设置页原生显示需实机验收；浏览器环境不代表这些行为。Linux 安装资源布局仍由桌面 Path Manager 内的 Tauri 平台解析器处理，尚未在此轮实机验证。

## 旧接口清理

已删除 `configured_app_data_dir`、`current_app_data_dir`、`default_app_data_dir`、`pet_data_directory`、`spool_path_for_settings`、`app_support_dir`、`code_pet_data_dir`、`log_file_path` 及对应的旧 IPC。Provider 的 `default_remote_workspace_root` 包装也已删除，直接调用共享 Path Manager。设置里旧目标检查、清空确认、递归复制、路径重写和迁移测试随实现删除。

不执行已有目录迁移，不清空已有数据，不为了本次结构调整扫描旧路径。领域内文件名、用户选择的项目 cwd、外部 Agent 自有配置和缓存保持各领域语义；系统 home/数据根/包内资源解析由 Path Manager 提供。
