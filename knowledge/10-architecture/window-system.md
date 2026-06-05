# 窗口系统

## 当前行为

桌宠悬浮窗不允许用户手动缩放。`frontend/PetApp.svelte` 在挂载时调用 `getCurrentWindow().setResizable(false)`。

前端仍负责运行时尺寸控制。它会调用 `ensureWindowSize()`，并在停靠或限制边界前使用预设逻辑宽高。这样可以保持稳定的最大透明区域，同时避免用户手动缩放带来的不确定性。

## 屏幕边界

`frontend/PetApp.svelte` 使用 `availableMonitors()`、`primaryMonitor()`、`outerPosition()` 和 `outerSize()` 选择与窗口相交面积最大的屏幕。没有相交时，选择离窗口中心最近的屏幕；仍然失败时回退到主屏。

`clampWindowPositionToMonitor()` 会把桌宠窗口限制在选中屏幕的工作区内，避免窗口越过屏幕边缘。

## macOS 悬浮窗设置

`src-tauri/src/platform/macos_window.rs` 配置 macOS 专属悬浮行为。panel 只有在需要时才允许成为 key window，使回复输入框可以获得键盘焦点，同时保持桌宠作为悬浮层。非 macOS 构建使用 `src-tauri/src/lib.rs` 中的 no-op wrapper。

## 风险

- 跨屏拖动时，位置或尺寸数据可能短暂陈旧。
- 修改内容高度但不检查 `ensureWindowSize()`，可能重新引入任务列表被裁剪或无意义滚动条。
- 多屏工作区可能使用负坐标。

## 验证

- 运行 `npx vitest run`，间接覆盖前端布局逻辑。
- 修改 `PetApp.svelte` 窗口代码时，在 Windows 和 macOS 上人工验证跨屏拖动。
