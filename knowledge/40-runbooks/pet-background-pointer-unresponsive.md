# 顶部桌宠未激活时鼠标无响应

## 现象与证据

2026-11-10 用户反馈顶部鼠标动作必须先激活窗口才响应。检查发现：`tauri.conf.json` 的 pet 窗口没有 `acceptFirstMouse`，本地 Tauri/Wry 默认值为 false；`NotchPet.svelte` 仅依赖 DOM enter/leave 驱动展开；虽然 `PetApp.svelte` 已每 60ms 通过 Surface 读取原生光标，但只用于透明区域透传。

## 根因与引入历史

未激活窗口首次点击未向 WebView 放行；悬停状态依赖 WebView 事件，不能保证非 key 的 NSPanel 在后台发送 enter/leave。两条路径都缺少后台交互支持。具体引入提交未确认，不将本次顶部改造直接认定为唯一来源。

## 修复与涉及模块

- `src-tauri/tauri.conf.json`：仅为 pet 开启首次点击接收，主窗口策略不变。
- `platform/macos_window.rs`：明确打开鼠标移动接收，保留 NonactivatingPanel，悬停不调用 activate 或 setFocus。
- `frontend/PetApp.svelte`：复用原生光标轮询，在页面失焦且处于非拖动 docked 模式时同步进入/离开；只在命中状态改变时派发事件，避免持续轮询重新打开 Escape 已收起的面板或不断重置离开计时器。跨平台读取继续走 Surface 接口。
- 隔离原生 QA 使用与生产一致的首次点击配置，并直接查询实际 WKWebView 的 `acceptsFirstMouse:` 和 NSPanel 移动接收/非激活标志。

## 验证与回归防线

运行 `npx --no-install vitest run frontend/lib/pet/presentation.test.ts frontend/lib/pet/surface.test.ts`（8 项），以及 `CODEPET_QA_NODE_MODULES=/tmp/codepet-pet-qa/node_modules node scripts/pet_notch_ui_qa.mjs`。新增后台场景禁止页面焦点，仅改变模拟原生光标，验证展开、离开收起、不抢键盘焦点、Escape 后停留不重复展开。

`TAURI_CONFIG='{"app":{"macOSPrivateApi":false}}' cargo run --manifest-path src-tauri/Cargo.toml --example pet_surface_qa --features custom-protocol -- --smoke` 验证实际 WebView 首次点击与 panel 标志、停靠/悬浮可见性和屏幕顶边。前端构建和原生 smoke 已通过。

上述 8 项定向测试、完整顶部 UI QA（含后台光标回归）均通过。修复版 `0.3.10-beta+topedge.20261110.2` 完整构建并通过签名完整性检查；退出旧进程，替换 Applications 中的 App 并启动，顶部窗口正常显示。测试日志为 `/tmp/codepet-inactive-{tests,ui,native,app-build}.log`。

人工复查：先激活浏览器，再悬停、点击和拖动桌宠；顶部响应应不需要预先点击激活，离开仍延迟收起，透明区域仍可穿透。当前自动操作工具无法对该非激活浮层发送坐标拖动，因此真实鼠标事件链仍需用户实机确认。规约补充于 `knowledge/60-rules/pet-window-click-through.md`。
