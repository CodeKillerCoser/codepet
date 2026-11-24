# Codex 工作台项目点击后未选中

## 现象与复现

工作台搜索框展开项目列表，鼠标点击项目后列表消失，已选项目不变。已有项目草稿时更容易误认为保存失败。用生产工作台 fixture，在 macOS WebKit 点击 Other project 可稳定复现；Chromium 同样操作正常。

## 证据与根因

修复前事件记录在 `artifacts/codex-workbench-qa/project-picker/before-webkit.json`：选项收到 pointerdown，搜索框随即 focusout，relatedTarget 为外部 SECTION；没有选项 click，也没有 saveSelection 请求。Chromium 的 relatedTarget 是列表内 BUTTON，正常执行 click 和保存。

原实现直接在 focusout 目标离开 picker 时设置 pickerOpen=false。WebKit 点击按钮时焦点移到祖先 section，Svelte 在 click 到来前卸载选项。问题发生在前端事件顺序，尚未进入偏好保存或扫描。`git blame` 显示失焦关闭与选项逻辑引入于 `29c6efb18`，后续框架和背景调整仍保留了该逻辑。

## 修复方案与模块

`frontend/lib/CodexWorkbench.svelte` 在选项及清空按钮的主 pointerdown 阶段阻止默认焦点转移，让 click 完成后更新选择。统一选择更新函数在 tick 后将焦点交回搜索框，兼顾键盘选择、移除和清空。再次点击仍有焦点的输入框也可重新展开 Escape 关闭的列表。移到外部控件仍关闭列表，选择仍只保存草稿，确认才扫描。

`scripts/codex_project_picker_ui_qa.mjs` 使用生产构建、Chromium 和 WebKit 的实际鼠标、触摸及键盘事件。必须验证长按鼠标按下阶段列表不消失，不能只调用 DOM element.click()，否则会绕过此故障。

## 验证与回归防线

运行 `node scripts/codex_project_picker_ui_qa.mjs`：验证两个引擎的点击、触摸、键盘、移除/清空、焦点恢复、Escape 后再次展开、保存顺序、确认后扫描范围及结果项目筛选。报告在同目录 report.json；输入数据为隔离 fixture。

同时运行工作台完整浏览器 QA、前端类型检查、样式与筛选测试、源码尺寸检查及生产构建。该修复未改 Rust、实际项目来源或存储格式，也未改已安装 App 的数据。本轮未通过系统鼠标实际操控已安装的 Tauri 窗口；已在独立 macOS WebKit 上复现原事件顺序并回归修复。
