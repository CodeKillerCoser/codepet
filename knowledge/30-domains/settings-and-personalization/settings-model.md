# 设置模型

## 当前模型

`src-tauri/src/app/settings.rs` 中的 `AppSettings` 有五个顶层区域：

- `appearance`
- `pet`
- `petLibrary`
- `notifications`
- `activityFilters`

所有区域都使用 serde defaults，确保新增字段后旧设置文件仍能加载。

## 前端归一化

`frontend/App.svelte` 会在使用加载到的设置前做归一化：

- 运行中气泡默认值和数值边界。
- 图片像素尺寸。
- 宠物透明度。
- 抽打反应音默认值。
- activity filter 关键词去空白和去重。

## 风险

新增字段如果没有 Rust 默认值或前端归一化，可能破坏旧用户设置，或产生 `undefined` UI 状态。

## 验证

- 运行 `cargo test --manifest-path src-tauri/Cargo.toml settings_tests`。
- 默认值影响 UI helper 时，新增或更新前端测试。
