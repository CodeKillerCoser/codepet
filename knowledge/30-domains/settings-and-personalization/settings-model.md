# 设置模型

## 当前模型

`src-tauri/src/app/settings.rs` 中的 `AppSettings` 有六个顶层区域：

- `appearance`
- `pet`
- `petLibrary`
- `notifications`
- `activityFilters`
- `agents`

所有区域都使用 serde defaults，确保新增字段后旧设置文件仍能加载。

`activityFilters` 现在以 `byAgent` 保存每个 Agent 的标题和内容关键词过滤。旧的顶层 `titleKeywords` 和 `messageKeywords` 字段保留为兼容入口；前端归一化会把旧全局过滤迁移成每个 Agent 各自的过滤配置。

`agents.byAgent.<agent>.hookEvents` 保存每个 Agent 勾选的 hook 事件。缺失或空列表表示默认使用该 Agent 注册表中的全部支持事件。

## 前端归一化

`frontend/App.svelte` 会在使用加载到的设置前做归一化：

- 运行中气泡默认值和数值边界。
- 图片像素尺寸。
- 宠物透明度。
- 抽打反应音默认值。
- activity filter 关键词按 Agent 去空白和去重。
- agent hook 事件按注册表顺序过滤非法值，默认全选。

## 风险

新增字段如果没有 Rust 默认值或前端归一化，可能破坏旧用户设置，或产生 `undefined` UI 状态。

按 Agent 过滤时，不能再把顶层 `activityFilters.titleKeywords/messageKeywords` 当成新配置写回；否则会重新变成全局过滤。

## 验证

- 运行 `cargo test --manifest-path src-tauri/Cargo.toml settings_tests`。
- 默认值影响 UI helper 时，新增或更新前端测试。
