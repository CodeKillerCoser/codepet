# 设置持久化

## 模型

`src-tauri/src/app/settings.rs` 定义 `AppSettings`，顶层区域包括：

- `data`：应用数据目录覆盖项。
- `appearance`：主题和运行中气泡设置。
- `pet`：当前宠物、sprite/图片设置、透明度、置顶和抽打反应音。
- `petLibrary`：宠物列表和数据目录。
- `notifications`：声音、自定义声音路径、响铃开关、重复间隔和静音时段。
- `activityFilters`：标题和消息关键词过滤。
- `agents`：每个 Agent 的 hook 偏好。
- `agentRuntimes`：旧版保存的 executable 字段，保留读取兼容；当前 Provider 启动与状态展示不使用这些历史路径。
- `providerPlugins`：额外 Provider plugin manifest 目录；默认目录不需要写入设置。
- `updates`：忽略的更新版本。

## 存储

设置统一保存到默认系统数据目录的 `code-pet/config/settings.json`。Windows 为 `%LOCALAPPDATA%/code-pet/config/settings.json`，macOS 为 `~/Library/Application Support/code-pet/config/settings.json`。读取只认新路径，不兼容根目录旧 settings.json。

共享 `crates/codepet-paths` 解析该引导文件，`data.dataDirectory` 仅覆盖业务数据根，设置入口和 `~/.codepet` 执行工作区保持固定。默认根的计算不能从 config 文件父目录直接推导，否则会误把 config 当成数据根。

实际目录及各模块职责见 [Path Manager](path-manager.md)。来源偏好保存于实际数据根的 config/pet-sources.json；Provider 数据为 providers/<pluginId>/provider.sqlite；远程身份凭据为 connections/；任务和技能为 tasks/；日志集中到 logs/；前端更新为 versions/webcontent/。自定义宠物目录仍优先于数据根下 pets/。

切换数据目录只保存路径并创建目标，不执行复制、清空、迁移或旧路径回退。需要重启才能让启动期资源全部使用新路径。用户选择的外部项目、Harness 安装及原生 Agent 自有数据目录保持自身语义。

## 前端同步

`frontend/App.svelte` 加载设置后会先归一化，再通过 Tauri command 保存。`frontend/PetApp.svelte` 监听 `settings-updated`，用于更新主题、过滤器、声音和宠物透明度。

`agentRuntimes` 旧字段仍不能由通用 `update_app_settings` 改写。当前 `set_agent_runtime_executable` 将选择交给 Provider 校验，验证成功后由 Provider 写入其数据库的 runtime_selection 表，不写 App 设置，也不重启 Provider 来恢复选择。`clear_agent_runtime_executable` 重启 Provider 触发全新自动扫描，并清除旧配置项。Host 只消费 Provider 的本次 harnessList、selected 与扫描诊断，App 历史路径不参与启动。Provider 自己根据数据库 lastSelected 匹配扫描结果，缺失或不兼容时选择最高语义版本。

`providerPlugins.directories` 是 Plugin Catalog 的显式附加目录。相对路径按已解析应用数据目录解析，绝对路径保持不变。Host 始终同时检查该数据目录下的 `versions/providers/`；设备身份和实例注册分别保存在同一目录下的 `connections/device-identity.json` 与 `providers/provider-instances.json`。这些记录不进入 pet activity 数据。

`update_app_settings` 在后端接收完整 JSON 值并先检查字段是否出现，再反序列化为 `AppSettings`。`providerPlugins` 缺失表示旧 caller 没有发送该字段，后端保留当前目录；显式发送 `providerPlugins.directories: []` 才表示清空。前端 `AppSettings.providerPlugins` 是必填字段，保存完整设置时总会带上当前值。该区分避免旧前端或局部设置更新误清插件目录。

## 验证

- 运行 `cargo test --manifest-path src-tauri/Cargo.toml settings`。
- 运行 `cargo test --manifest-path src-tauri/Cargo.toml --lib settings_update_preserves_missing_provider_plugins_and_clears_explicit_empty`。
- 运行会构造设置默认值的前端测试，尤其是声音和气泡颜色相关测试。
