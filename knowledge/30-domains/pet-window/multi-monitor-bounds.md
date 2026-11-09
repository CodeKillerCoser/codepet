# 多屏边界

## 当前实现

桌宠几何统一由 `src-tauri/src/pet/surface/` 协调，平台 API 封装在 `platform/pet_surface.rs`。前端只发送自然布局和角色锚点，不再自行选择显示器、调整窗口位置。

悬浮时按角色锚点选择包含它的显示器，找不到时选最近显示器；工作区钳制只在非拖动阶段执行。拖动期间可进入屏幕顶部吸附带，松手后才停靠或恢复工作区约束。当前显示器移除时使用可用显示器及归一化锚点恢复。macOS 所有屏幕统一使用顶部吸附，物理刘海只影响相交面板的内容安全区；横向位置按屏幕比例持久化；其他平台维持普通悬浮。

macOS 计算使用 AppKit 点坐标，y 统一为负的 AppKit y，不混用物理像素。其他平台适配器使用物理坐标，并携带每屏 CSS→原生单位倍率。前端 alpha 命中通过原生 pet_surface_pointer 获取窗口内逻辑坐标。

## 刘海检测诊断

`platform/pet_surface.rs` 当前按每块 `NSScreen.safeAreaInsets.top > 0` 设置 `physical_notch`；辅助左右区域用于确定横向位置和宽度，辅助区域不可用时宽度回退到 180pt。无刘海屏仍创建顶部吸附目标，因此 `notch.is_some()`/`canDock` 不能解释为物理刘海。当前 Surface 不再输出菜单模式；统一输出 `topInset`、`notchWidth`，仅在窗口横向范围与物理刘海相交时增加顶部黑色安全区。窗口保持贴顶，详见[统一顶部吸附](top-edge-docking.md)。后文菜单分流修复是被统一方案替代前的历史记录。

运行 `swift scripts/diagnose_notch_macos.swift --json`，或双击同目录下的 `diagnose_notch_macos.command`。脚本只读，列出所有屏幕的 ID、名称、内外置属性、缩放、frame/visibleFrame、安全区、左右辅助区、当前 Code Pet 判断及回退吸附目标；保存的 displayId 仅供核对。自定义数据目录用 `--surface-state <路径>`，其他安装副本用 `--app <路径>`。

2026-11-10 编译并实测：本机 LG ULTRAFINE（ID 2）为外接屏，`safeAreaInsets.top=0`、左右辅助区域为空，物理刘海 false，但吸附能力 true；visibleFrame 顶部预留 30pt 是独立数据，不能据此认定刘海。本机结果未复现检测错误，尚不能代表有刘海的设备、多屏或兼容模式下正确。

独立脚本与 App 并非同一进程，可能受不同的安全区兼容行为影响；脚本读取 App 的 `NSPrefersDisplaySafeAreaCompatibilityMode` 配置，但该键缺失不能直接推断兼容模式开关状态，也不能从保存的 displayId 推断实时菜单所在屏幕。需在发生问题的设备上对照原始值和实际菜单屏幕，不能用主屏结果替代目标屏幕。新增脚本已运行 Swift 编译/JSON 输出检查与 `zsh -n`；未改变生产检测逻辑。

## 风险与验证

用户的 `notch-report.json` 给出内置屏 1512×982pt、顶部安全区 32pt、辅助区间隙 185pt，检测结果 `physicalNotch=true`，保存的显示器 ID 同为 1。源码原先却以全局 `menu_ready` 优先选择菜单位置、控制隐藏和展开，因此检测成功并未触发刘海布局。修复不改变检测 API，而是区分平台能力与当前屏的呈现路径；对应报告的测试要求面板中心 x=755.5、顶边 y=-950（Code Pet 坐标）。具体引入提交未确认。

受影响模块：`pet/surface/mod.rs` 统一分流定位、尺寸预算、窗口可见性和拖回目标，并清理跨屏残留的菜单固定/按下状态；`presentation.ts` 在路径变化时重置 hover/focus/展开；UI QA 使用一致的 `physicalNotch/menuBar` 组合验证跨屏圆角及 hover。检测适配器与共享任务数据不变。此修复包含原生逻辑，必须使用新编译的 App，单独替换前端包不足以生效。

- 负坐标/不同缩放：geometry 单测与浏览器 125% DPI 场景；混合 Retina 外屏仍需实机验收。
- 拖动/resize/显示器事件交错：原生单写入、revision 拒绝旧测量；测试拖动时任务增减、屏幕移除与取消。
- 可见区变化或拔屏：原生低频重查屏幕几何；手动验证 Dock/菜单栏自动隐藏、Space、睡眠与恢复。
- 菜单栏遮挡：停靠内容位于刘海/顶部区域下方，frame 贴合内容；真实菜单点击和透明区滚动需原生验收。

## 2026-11-10 分流修复验证

- `TAURI_CONFIG='{"app":{"macOSPrivateApi":false}}' cargo test --manifest-path src-tauri/Cargo.toml --lib pet::surface`：20 项通过，包含报告几何下收起/展开定位、菜单高度与边界约束、残留菜单拖回目标隔离、跨屏拖动及取消。
- `npx --no-install vitest run frontend/lib/pet frontend/PetApp.test.ts`：41 项通过，新增同一停靠模式内 menu→notch→menu 的呈现状态切换。
- `CODEPET_QA_NODE_MODULES=/tmp/codepet-pet-qa/node_modules node scripts/pet_notch_ui_qa.mjs`：通过两条路径的圆角、hover/键盘、焦点、拖出/拖回模拟、完成组及 1024/1440/2560/3840 工作区尺寸回归。跨屏样例以新对象模拟 IPC 快照，避免测试端原地修改旧状态掩盖路径变化。
- `npm run build:webcontent`：成功，API 8、16 个文件。
- `TAURI_CONFIG='{"app":{"macOSPrivateApi":false}}' cargo run --manifest-path src-tauri/Cargo.toml --example pet_surface_qa --features custom-protocol -- --smoke`：编译生产原生模块，独立临时数据目录的真实 WebView 检查通过。本机无刘海外屏上的菜单底边与实际窗口顶边均为 -1050pt；打开、关闭、悬浮、重新停靠和再次打开符合预期，输出 `Native smoke: Ok(())`。
- 仍未完成物理刘海设备、真实跨屏鼠标手势的实机验收；报告参数单测与浏览器模拟不能替代此项。未替换 `/Applications/Code Pet.app`，未发布新版。
