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

`data.dataDirectory` 是当前路径配置；它仅覆盖 data，不改变 install 或 workspace。设置引导文件固定保存在默认 data 根的 `config/settings.json`，从而更改 data 后仍能找到配置。设置模块与 Path Manager 共用该 JSON 和同一个 `DataSettings` 类型；Path Manager 只读取路径字段，不重写其他设置。无配置时用默认值，损坏 JSON 返回错误，避免覆盖原设置。旧的路径读取接口及其默认回退已经删除。

## 备选方案与取舍理由

- 独立 `paths.json`：边界直观，但旧 settings 里的 dataDirectory 会产生第二份配置，需要双写同步。本次采用同一 JSON。
- install/data/workspace 都可随设置修改：迁移灵活，但容易把用户工作树随缓存一起搬走。本次 install 来自可执行文件，workspace 固定来自平台 home。
- 每个模块继续拼接系统路径：改动小，但不同进程长期容易出现路径分歧。共享 crate 固化目录规则，各业务只派生本模块文件。

## 影响范围

- 设置加载和保存：直接调用 Path Manager 获取引导文件。settings.rs 不再定义任何目录计算、迁移、复制或清空方法；自定义数据/宠物目录的写入入口也归 Path Manager。切换路径只保存配置并创建目标目录。
- 前端：`path_manager_get` 返回完整路径对象；`getAppPaths()` 调用它；写入统一使用 `path_manager_set_data`、`path_manager_set_pets`，系统设置展示真实 install/data/workspace，不在 UI 猜默认路径。
- webcontent：共同比较 data/versions/webcontent/vN 和 resources/webcontent，选择兼容且完整的最高版本，同版本取最新构建时间。版本选择和完整性检查仍由资源加载器负责。
- Provider 与 Hook：包内 Provider 根由 Path Manager 提供，显式开发资源覆盖保留；Hook 脚本使用当前数据目录。日志、宠物、事件缓冲等直接调用 Path Manager，旧路径函数已删除；Hook 的 Node 适配器读取同一 settings.json，默认事件缓冲为 data/connections/spool，不保留 `~/.code-pet/spool` 回退。外部 Agent 的 CODEX_HOME、Claude 配置等仍属于其原生目录，不移动到 Code Pet。
- 三个 Provider：共同调用 remote_workspace，保留 `~/.codepet/remote_workspace/<provider>`；用户选择的项目目录和已有会话 cwd 保持原值。
- 任务抽取：记录和 skills 保存在 data/tasks，新执行目录位于 `~/.codepet/workspaces/task-extraction`、`workspaces/tasks`。旧执行目录保留，不删除或自动搬移；任务分配的边界校验改为对应工作目录。

## Path Manager 初次引入时的验证记录

共享 crate 测试覆盖 Windows 布局、macOS App bundle 资源布局、自定义 data 不影响 workspace/引导文件、共用 JSON 不写回、损坏配置和路径越界。任务管理测试验证记录与执行目录分离，并实际分配 task 文件。

已运行 `cargo test --manifest-path crates/Cargo.toml -p codepet-paths -p codepet-task-lineage --lib --offline`：路径 4 项及任务管理原有 23 项通过；新增隔离测试补齐 schemaVersion 后单独复跑通过。三个 Provider 的 `cargo check` 通过。`npx vitest run frontend` 195 项及 `npm run build:webcontent` 通过。Node 路径适配器测试和 webcontent 的 9 项测试通过。

桌面回归命令：`cargo test --manifest-path src-tauri/Cargo.toml --features custom-protocol --test settings_tests --test pet_library_tests --test spool_tests --test app_log_tests --test hook_script_tests --offline -j 2`，使用 `tauri.bin.conf.json` 配置，40 项全部通过。

## 后续观察

新增 Path Manager 原生命令，因此 webcontent 后端契约升级为 2。旧已安装 App 和 data/versions/webcontent/v1 本轮未替换。发布时需要配套原生程序与契约 2 的完整 UI；已有契约 1 资源目录应先准备更新，否则新程序会按既有资源校验规则显示不兼容诊断。

实际安装包的 macOS 路径、首次启动、切换数据目录后的日志/Hook 和设置页原生显示需实机验收；浏览器环境不代表这些行为。Linux 安装资源布局仍由桌面 Path Manager 内的 Tauri 平台解析器处理，尚未在此轮实机验证。

## 旧接口清理

已删除 `configured_app_data_dir`、`current_app_data_dir`、`default_app_data_dir`、`pet_data_directory`、`spool_path_for_settings`、`app_support_dir`、`code_pet_data_dir`、`log_file_path` 及对应的旧 IPC。Provider 的 `default_remote_workspace_root` 包装也已删除，直接调用共享 Path Manager。设置里旧目标检查、清空确认、递归复制、路径重写和迁移测试随实现删除。

不执行已有目录迁移，不清空已有数据，不为了本次结构调整扫描旧路径。领域内文件名、用户选择的项目 cwd、外部 Agent 自有配置和缓存保持各领域语义；系统 home/数据根/包内资源解析由 Path Manager 提供。

## 按功能组织的数据目录（2026-09-13）

用户明确要求直接调整读写路径，不迁移、不回退读取旧路径。采用以下结构，按需创建目录，不预先创建空 cache：

```text
code-pet/
  config/settings.json                  # 固定在默认系统目录的引导设置
  config/pet-sources.json                # 实际数据根下的来源偏好
  config/hooks/                         # 托管脚本与 Node 路径适配器
  connections/device-identity.json       # 本机稳定身份
  connections/lan-tls-identity.json
  connections/remote-credentials.json
  connections/rtc-cloud.json
  connections/spool/events.jsonl         # 尚未消费的 Hook 事件
  providers/provider-instances.json
  providers/conversation-state.sqlite    # Host 共享/回退业务状态
  providers/<pluginId>/provider.sqlite   # Provider 私有业务状态
  tasks/skills/<name>/SKILL.md
  tasks/v1/<sourceHash>/                 # 任务记录、抽取设置及锁
  pets/
  logs/code-pet.log
  logs/events*.jsonl[.idx]
  logs/providers/<pluginId>/provider.log
  versions/webcontent/vN/                # 完整前端资源包
  versions/providers/                   # 数据根中的附加 Provider 包
  cache/                                # 预留可重建内容，当前不主动创建
```

`PathManager.data` 是 API 中的“应用数据根”，不是磁盘上新增的 data 子目录。包内 webcontent 和 provider-plugins 仍从 resources 读取。执行目录继续使用 `~/.codepet/workspaces`。

设置读取只认默认根的 config/settings.json；不存在时使用默认设置，即使旧根目录有 settings.json 也不读取。Rust 和 Node 使用相同路径规则。切换自定义数据根后固定引导设置仍在默认 config，其他业务内容随实际数据根解析。任务页不再采用旧 task-lineage 绑定文件接管历史实例目录。

Provider Host 分别指定 provider_data_root 和 provider_logs_root，将业务数据库与日志分配给插件；数据根启用时必须同时提供绝对日志根。独立调用者不配置数据根时仍可使用原来的无磁盘配置。Provider 程序使用 Host 下发的目录，不自行拼接旧目录。

风险与验证：默认根误变成 config、Node/Rust 分叉通过路径及 Hook 测试覆盖；旧配置误读通过放置无效旧文件验证；Provider 数据/日志路由通过真实 fixture 子进程回传目录并检查 SQLite 文件与日志；资源安装和加载分别验证新 versions 路径；任务执行/持久化分离沿用 Layout 测试。旧文件不自动删除，安装版不会因仅修改源码而改变运行路径。实际 Windows/macOS 安装验收不等于测试通过。

## 本次目录调整验证

- `node --test crates/codepet-paths/node_test.mjs scripts/webcontent_test.mjs`：12 项通过，含旧路径忽略、版本目录与符号链接边界。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-paths --lib --offline -j 2`：5 项通过。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-task-lineage --lib --offline -j 2`：与路径包一同执行，24 项通过。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test manager_gateway codepet_usage_routes_to_provider_and_host_assigns_private_storage_and_logs --offline -j 2`：1 项通过。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test device_catalog --offline -j 2`：4 项通过。
- 桌面 Rust 检查首次发现两处旧接管函数调用残留，清除后复跑通过。命令：`cargo test --manifest-path src-tauri/Cargo.toml --features custom-protocol --lib --test settings_tests --test pet_library_tests --test spool_tests --test app_log_tests --test hook_script_tests --offline -j 2`，使用 `TAURI_CONFIG={"bundle":{"resources":[]}}`：77 项单元测试、40 项集成测试全部通过。保留既有 unused/dead_code 编译警告。
- `git diff --check` 通过；构建产生的 schema 文件已恢复，未纳入改动。

本次沿用 living-dev-doc-writer 技能更新架构及领域文档。没有安装、启动新 App，也没有迁移或清理用户目录。Mac 实机行为仍需发布时验证。
