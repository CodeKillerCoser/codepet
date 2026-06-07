# 设置持久化

## 模型

`src-tauri/src/app/settings.rs` 定义 `AppSettings`，顶层区域包括：

- `data`：应用数据目录覆盖项。
- `appearance`：主题和运行中气泡设置。
- `pet`：当前宠物、sprite/图片设置、透明度、置顶和抽打反应音。
- `petLibrary`：宠物列表和数据目录。
- `notifications`：声音、自定义声音路径、响铃开关、重复间隔和静音时段。
- `activityFilters`：标题和消息关键词过滤。

## 存储

设置入口保存到系统 local data 目录下的 `code-pet/settings.json`，用于保证应用始终能找到自定义数据目录配置。未配置 `settings.data.dataDirectory` 时，Token 缓存、日志和默认宠物库路径保持原来的系统 local data 下 `code-pet/`。配置后，Token 缓存、下次启动的日志、默认宠物库和自定义离线 spool 跟随该目录；显式配置过的 `petLibrary.dataDirectory` 仍优先。

修改应用数据目录时，后端会把原数据目录内容复制到目标目录，但不会复制固定入口 `settings.json`。新旧数据目录不能互相包含。用户通过目录选择器指定的自定义目标必须为空；如果目标已有内容，前端必须先弹窗确认，后端只有收到确认标记后才会清空目标目录并继续复制。默认宠物库的图片路径会随应用数据目录改写；显式配置过宠物库目录时不改写。保存完成后需要重启应用，日志等启动期资源才会完全使用新目录。

## 前端同步

`frontend/App.svelte` 加载设置后会先归一化，再通过 Tauri command 保存。`frontend/PetApp.svelte` 监听 `settings-updated`，用于更新主题、过滤器、声音和宠物透明度。

## 验证

- 运行 `cargo test --manifest-path src-tauri/Cargo.toml settings`。
- 运行会构造设置默认值的前端测试，尤其是声音和气泡颜色相关测试。
