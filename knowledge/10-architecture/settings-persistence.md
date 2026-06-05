# 设置持久化

## 模型

`src-tauri/src/app/settings.rs` 定义 `AppSettings`，顶层区域包括：

- `appearance`：主题和运行中气泡设置。
- `pet`：当前宠物、sprite/图片设置、透明度、置顶和抽打反应音。
- `petLibrary`：宠物列表和数据目录。
- `notifications`：声音、自定义声音路径、响铃开关、重复间隔和静音时段。
- `activityFilters`：标题和消息关键词过滤。

## 存储

设置保存到系统 local data 目录下的 `code-pet/settings.json`。README 还记录了 Token 缓存、日志、宠物库和离线事件 spool 的位置。

## 前端同步

`frontend/App.svelte` 加载设置后会先归一化，再通过 Tauri command 保存。`frontend/PetApp.svelte` 监听 `settings-updated`，用于更新主题、过滤器、声音和宠物透明度。

## 验证

- 运行 `cargo test --manifest-path src-tauri/Cargo.toml settings`。
- 运行会构造设置默认值的前端测试，尤其是声音和气泡颜色相关测试。
