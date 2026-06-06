# 桌宠透明区域事件透传

## 规则

透明桌宠窗口的非命中区域必须使用窗口级鼠标事件透传，不能只依赖 CSS `pointer-events: none`、移除拖拽层或前端事件吞吐来表现为“不响应”。

## 适用场景

修改 `frontend/PetApp.svelte`、桌宠窗口尺寸、透明背景、拖拽命中区、宠物图片渲染、任务卡片交互或 Tauri window capability 时适用。

## 反例

让透明区域不触发拖拽，但窗口本身仍接收鼠标事件。用户看到透明区域，却无法点击或滚动其下方应用，尤其在固定大窗口高度下会形成大面积屏幕事件拦截。

## 推荐做法

使用 `cursorPosition()` 轮询全局鼠标位置，将其换算到桌宠窗口坐标；只在任务卡、操作按钮、宠物不透明像素等命中区域内恢复 `setIgnoreCursorEvents(false)`，其余区域设置 `setIgnoreCursorEvents(true)`。新增或调整 Tauri JS window API 时同步检查 `src-tauri/capabilities/default.json`。

## 来源

用户反馈：透明区域在 app 内不响应，但事件不会透给其他窗口；需要的不是禁止拖拽响应，而是真正透传。历史提交 `754581a fix(pet): 让透明区域鼠标事件穿透` 也说明该行为曾作为 Bug 修复出现过。

## 验证方式

运行 `npx vitest run frontend/PetApp.test.ts frontend/lib/petHitTest.test.ts`。人工验证时在 Windows 和 macOS 上检查：透明区域可点击下方窗口，任务卡/按钮可点击，宠物不透明像素可拖动，宠物透明像素尽量不拦截下方窗口。
