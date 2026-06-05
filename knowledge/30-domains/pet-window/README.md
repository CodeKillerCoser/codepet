# 桌宠窗口领域

这个领域覆盖悬浮窗口、任务卡片布局、窗口尺寸、屏幕边界和回复编辑器。

## 源码模块

- `frontend/PetApp.svelte`：窗口 UI、尺寸、停靠、拖动、回复、审批和声音重复提醒。
- `frontend/lib/petHitTest.ts`：hit-test 矩形支持。
- `src-tauri/src/platform/macos_window.rs`：macOS 悬浮窗配置。

## 测试

- `frontend/PetApp.test.ts`
- `frontend/lib/petHitTest.test.ts`
- `src-tauri/tests/macos_window_tests.rs`
