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

设置入口保存到系统 local data 目录下的 `code-pet/settings.json`，用于保证应用始终能找到自定义数据目录配置。未配置 `settings.data.dataDirectory` 时，Token 缓存、日志和默认宠物库路径保持原来的系统 local data 下 `code-pet/`。配置后，Token 缓存、下次启动的日志、默认宠物库和自定义离线 spool 跟随该目录；显式配置过的 `petLibrary.dataDirectory` 仍优先。

默认路径按平台展开为：

| 平台 | 默认 Code Pet 数据目录 | 固定设置入口 |
| --- | --- | --- |
| Windows | `%LOCALAPPDATA%/code-pet/` | `%LOCALAPPDATA%/code-pet/settings.json` |
| macOS | `~/Library/Application Support/code-pet/` | `~/Library/Application Support/code-pet/settings.json` |

默认数据目录由 `dirs::data_local_dir()`（失败时回退 `dirs::data_dir()`）和 `PathBuf::join("code-pet")` 得到，不允许业务代码自行拼接环境变量字符串。`settings.data.dataDirectory` 是 Code Pet 数据根的覆盖项；固定设置入口始终留在默认目录，用它在进程启动早期找到覆盖后的实际数据根。

实际数据根包含或承载以下内容：

```text
<Code Pet data>/
├── logs/code-pet.log
├── providers/<pluginId>/
│   ├── data/provider.sqlite
│   └── logs/provider.log
├── pets/
├── provider-plugins/
├── provider-host/
│   ├── device-identity.json
│   └── provider-instances.json
└── remote-access/
```

表中只列出由当前实现明确管理的主要内容，目录可以按功能延伸。配置自定义 Code Pet 数据根后，离线事件写入 `<Code Pet data>/spool/events.jsonl`；未配置覆盖项时为兼容旧 Hook，仍使用 `~/.code-pet/spool/events.jsonl`。`providerPlugins.directories` 的相对路径也以实际数据根为基准；绝对路径保持不变。安装包自带的 Provider adapter 仍位于 App resource 目录，不会复制到这里。

修改应用数据目录时，后端会把原数据目录内容复制到目标目录，但不会复制固定入口 `settings.json`。新旧数据目录不能互相包含。用户通过目录选择器指定的自定义目标必须为空；如果目标已有内容，前端必须先弹窗确认，后端只有收到确认标记后才会清空目标目录并继续复制。默认宠物库的图片路径会随应用数据目录改写；显式配置过宠物库目录时不改写。保存完成后需要重启应用，日志等启动期资源才会完全使用新目录。

Code Pet 数据目录覆盖不改变以下目录：Harness executable 安装位置、Provider 实例的 Runtime 数据目录，以及 `~/.codepet/remote_workspace/`。Runtime 数据目录遵循 [Provider 本地路径和数据目录](../60-rules/provider-local-runtime-paths.md)，会话 workspace 遵循 [Provider 新建会话的项目与工作目录](../60-rules/provider-conversation-workspaces.md)。

## 前端同步

`frontend/App.svelte` 加载设置后会先归一化，再通过 Tauri command 保存。`frontend/PetApp.svelte` 监听 `settings-updated`，用于更新主题、过滤器、声音和宠物透明度。

`agentRuntimes` 旧字段仍不能由通用 `update_app_settings` 改写。当前 `set_agent_runtime_executable` 将选择交给 Provider 校验，验证成功后由 Provider 写入其数据库的 runtime_selection 表，不写 App 设置，也不重启 Provider 来恢复选择。`clear_agent_runtime_executable` 重启 Provider 触发全新自动扫描，并清除旧配置项。Host 只消费 Provider 的本次 harnessList、selected 与扫描诊断，App 历史路径不参与启动。Provider 自己根据数据库 lastSelected 匹配扫描结果，缺失或不兼容时选择最高语义版本。

`providerPlugins.directories` 是 Plugin Catalog 的显式附加目录。相对路径按已解析应用数据目录解析，绝对路径保持不变。Host 始终同时检查该数据目录下的 `provider-plugins/`；设备身份和实例注册分别保存在同一目录下的 `provider-host/device-identity.json` 与 `provider-host/provider-instances.json`。这些记录不进入 pet activity 数据。

`update_app_settings` 在后端接收完整 JSON 值并先检查字段是否出现，再反序列化为 `AppSettings`。`providerPlugins` 缺失表示旧 caller 没有发送该字段，后端保留当前目录；显式发送 `providerPlugins.directories: []` 才表示清空。前端 `AppSettings.providerPlugins` 是必填字段，保存完整设置时总会带上当前值。该区分避免旧前端或局部设置更新误清插件目录。

## 验证

- 运行 `cargo test --manifest-path src-tauri/Cargo.toml settings`。
- 运行 `cargo test --manifest-path src-tauri/Cargo.toml --lib settings_update_preserves_missing_provider_plugins_and_clears_explicit_empty`。
- 运行会构造设置默认值的前端测试，尤其是声音和气泡颜色相关测试。
