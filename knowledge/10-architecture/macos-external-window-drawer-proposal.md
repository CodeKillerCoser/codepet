# macOS 顶部外部窗口抽屉调研与候选方案

## 背景

2026-11-17，用户希望把其他 App 的窗口收纳到桌宠顶部下拉框：鼠标悬停展开原窗口，继续操作，移开后自动收起。本次只研究 macOS。

2026-11-18，用户用截图蓝框明确视觉位置：选中窗口必须位于黑色下拉容器的正文内部，形成嵌入外观。此前仅在小顶栏下方摆放独立窗口的 Demo 不满足此要求；下述实现路径已按该澄清修订。

**结论：用辅助功能 API 管理原窗口，有实现该工作流的路径；不应承诺把任意跨进程窗口真正嵌入 WebView，也不能先承诺无缝上滑隐藏。** 用户已明确排除最小化收起。当前 0.3 Demo 的内置示例直接隐藏，外部窗口仅在可验证完全屏外停放时接受收纳，失败恢复原位；不得以最小化兜底。本文记录候选设计及独立 Demo，尚未接入正式桌宠，也不是已通过外部 App 兼容性验收的架构决策。

## 目标

- 绑定具体窗口，顶部入口可管理多个窗口，首版同一时刻展开一个。
- 窗口按相对顶栏的位置布局；沿顶部移动面板时，同步更新当前展开的窗口位置，收起的窗口只更新下次展开位置。
- 展开后直接使用原 App 的真实窗口，保留正常的输入法、复制粘贴、滚动和拖拽路径。
- 黑色容器扩展到整个正文，包住窗口并保留四周留白；实际窗口 frame 必须位于正文区域内，不能只有顶栏而没有正文外框。
- 收起后只剩顶部条，原窗口保持存活且不进入最小化状态；展开继续使用同一窗口及内容。
- 鼠标离开入口及窗口的联合范围后延迟收起，允许固定展开和解除收纳。
- 不影响同一 App 中未被收纳的窗口，并提供恢复路径。

## 非目标

- Windows 实现、将外部窗口变为本进程子视图、保证兼容所有 App。
- 首版跨 Space 搬运、原生全屏/Split View 窗口收纳、强制改变外部窗口层级。
- 首版镜像画面上的远程输入代理，以及要求用户降低系统保护的窗口接管方案。

## 现状理解

### 仓库证据

- [顶部面板边界](dock-panel-motion-boundary.md)及 `frontend/lib/pet/DockPanel.svelte`：组件负责现有容器、200ms 悬停展开、240ms 离开收起、拖动和动效。`presentation.ts` 的收起条件同时检查 hover、焦点和按键保持状态，目前没有外部窗口的概念。
- `frontend/PetApp.svelte`：命中区域包含 `.notch-backdrop`，通过 `pet_surface_pointer` 和整窗鼠标透传维持后台 hover。原窗口不属于 DOM，鼠标进入它会离开现有面板；仅增加一个前端插槽不足以支持此功能。
- `src-tauri/src/platform/macos_window.rs`：桌宠是透明、非激活的 `NSPanel`，可以加入所有 Spaces、作为全屏辅助窗口；这些配置只作用于桌宠自身。
- `src-tauri/src/platform/pet_surface.rs`：已有屏幕、安全区、全局鼠标位置及鼠标按钮读取。内部坐标为 `y = -AppKitY`，不能直接当成以主屏左上角为原点的 AX 坐标。
- `src-tauri/src/platform/permissions.rs`：目前提供完全磁盘访问设置入口，没有外部窗口所需的 AX 信任检查和授权流程。

本机 `sw_vers` 为 macOS 27.0（26A428）。读取 `/Applications/Code Pet.app` 的签名 entitlement 未输出 App Sandbox entitlement；只说明本机安装产物的观察结果，后续发布构建仍需核对。本次没有授权、移动、隐藏或输入到用户窗口。

### 公开接口与一手实现证据

| 路径 | 查到的能力 | 判断 |
| --- | --- | --- |
| AppKit 子窗口 | `addChildWindow` 接受 `NSWindow`，协调位置与层级；`window(withWindowNumber:)` 仅能取得本 App 拥有的对应对象 | 不能据此把外部窗口 ID 变成本 App 的可裁剪内容。未发现通用公开的跨进程嵌入接口，属于根据接口范围作出的判断。[Apple 子窗口](https://developer.apple.com/documentation/appkit/nswindow/addchildwindow(_:ordered:))、[窗口归属](https://developer.apple.com/documentation/appkit/nsapplication/window(withwindownumber:).md) |
| Accessibility / AX | 按 PID 获取对象、检查属性可写性、修改属性及执行窗口动作 | 推荐管理原窗口；每个窗口都要探测能力，处理不支持、无效对象和消息失败。[Apple AX](https://developer.apple.com/documentation/applicationservices/axuielement_h)、[设置属性](https://developer.apple.com/documentation/applicationservices/1460434-axuielementsetattributevalue) |
| ScreenCaptureKit | 将窗口画面采集为媒体帧 | 可以做缩略图或过渡画面，不会同时获得原窗口的交互能力；交互代理还需解决焦点、菜单、输入法和拖放。[Apple 采集示例](https://developer.apple.com/documentation/screencapturekit/capturing-screen-content-in-macos) |

[StickyWindows](https://github.com/aaronfisher/StickyWindows) 已提供边缘入口、hover 召回原窗口和离开收起，是接近需求的一手研究样本。其 [PeekMonitor](https://github.com/aaronfisher/StickyWindows/blob/main/StickyWindows/Parking/PeekMonitor.swift) 使用窗口/入口联合命中、10pt 容差、500ms 离开延迟，并在鼠标按住时暂停收起。它列出的停靠边缘是左、右、下，不能替我们证明顶部收纳已解决；本项目没有运行或复制其实现。

## 实现路径

### 1. 顶部作为入口，真实窗口显示在下方

用户通过选择器或快捷键收纳当前窗口，记录原 frame 和最小化状态，并检查位置、尺寸能力。最小化能力不再作为收起路径。建议首版选择器只展示当前可见桌面的普通窗口；快捷键收纳要在打开 Code Pet UI 前记录原前台窗口，避免绑定到自己。

顶部保留桌宠和任务入口，增加窗口入口。悬停约 200ms 后，恢复指定窗口、定位到顶栏下方并执行 raise；原窗口保持自己的标题栏和尺寸约束。较大的窗口按工作区尽量适配，尺寸不可写时只移动，放不下则拒绝收纳或作为普通窗口打开。不要强塞进现有任务列表的宽高。

容器使用原生 NSPanel 绘制完整黑色顶栏及正文外框，在实际窗口 frame 对应位置挖透明开孔；原窗口置于开孔内。面板宽度包含窗口及两侧留白，高度包含顶栏、正文和底部留白。鼠标进入窗口区域时切换外框的 `ignoresMouseEvents`，直接由原窗口接收系统输入；返回顶栏后恢复外框交互。原窗口仍保留正常标题栏和控件，不通过 WebView 或镜像重建原 App 内容。透明开孔只是视觉与输入边界，不是跨进程窗口 reparent。

悬停时先尝试只 raise、不激活；点击后按 macOS 正常机制把操作交给原 App。raise 与键盘焦点是两件事，部分 App 的前置效果、首次点击和激活副作用需要原型验证，不能承诺始终不切换焦点。[Hammerspoon raise](https://www.hammerspoon.org/docs/hs.window.html#raise)

### 2. 用相对布局实现窗口跟随

每个绑定窗口保存 `原始 frame`、`相对顶栏偏移` 和 `展开尺寸`。协调器根据顶栏实际屏幕位置计算目标 frame，通过 AX 更新原窗口；不要求窗口成为 AppKit 子窗口。沿顶部拖动时只更新位置，不逐帧修改尺寸；任务内容变化造成面板宽度改变时，以稳定的顶栏锚点计算位置，避免窗口随动画的临时 WebView frame 跳动。

外部窗口与桌宠分属不同进程，位置写入不能保证同一帧原子提交。实现时合并中间目标，按窗口串行发送最新位置，松手后回读实际 frame 校正；通过桌面录屏验证跟随延迟，而不预设能够完全贴合现有前端动画。原窗口被用户独立拖动或调整大小时，暂停跟随，操作结束后更新相对布局；提供明确的解除收纳动作，避免立即把用户刚移动的窗口拉回。

可收纳多个窗口。原型先验证单窗口；后续若同时展开多个，应保存各自相对位置并联合计算可用工作区、鼠标保持区域和恢复状态。展开/收起使用同一事务意图，但各窗口操作可能先后完成，需要处理其中一个窗口无响应的情况。

### 3. 收起策略必须单独选型

| 方式 | 收益与代价 | 建议 |
| --- | --- | --- |
| AX 最小化指定窗口 | 会进入系统最小化状态和 Dock，不能满足仅保留顶部条的收纳语义 | 用户已排除；0.1/0.2 的原型路径已由 0.3 移除，不得作为兜底 |
| 隐藏整个 App | 整个 App 一起隐藏 | 仅在用户明确选择整 App 收纳时考虑，不自动替代单窗口操作 |
| 移到屏幕外 | 保留原生窗口，存在位置钳制、边缘残留、露到另一块屏幕和恢复风险 | 0.3 外部窗口采用严格探测：实际 frame 与全部显示器无交集才算成功，否则恢复并提示不支持；尚未通过外部兼容性验收 |

[Hammerspoon](https://www.hammerspoon.org/docs/hs.window.html#minimize) 明确最小化动画遵循系统设置；[Apple hide](https://developer.apple.com/documentation/appkit/nsapplication/hide(_:)) 明确隐藏所有窗口。[AeroSpace 指南](https://nikitabobko.github.io/AeroSpace/guide#emulation-of-virtual-workspaces) 也记录了角落停放的边缘残留与多屏排列约束，说明屏外停放需要专门验证。

### 4. 将外部窗口纳入交互状态

抽屉状态建议为：`收起 → 展开中 → 展开 → 待收起 → 收起中`，另有固定展开、恢复和失联状态。异步 AX 操作按目标窗口串行，快速反向或切换窗口只执行最新意图；设置消息超时，避免同步 AX 请求堵塞 UI。

保持区域包括：顶栏入口、原窗口 frame、两者之间的通道，以及能够确认归属的菜单/sheet/弹出窗口。鼠标离开约 500–700ms 才收起；按住鼠标、拖动、调整大小时暂停。外部窗口仍有键盘焦点，不能单凭这一点永久阻止收起，否则无法满足用户移开鼠标即收起的需求；长时间输入提供“固定”按钮或快捷键。

菜单、文件选择器和输入法候选可能超出主窗口甚至属于其他进程。已识别的关联弹窗应暂缓收起；无法确认时应保守保持并提供手动收起，不能先承诺通用识别所有候选框。该逻辑由原生协调器拥有，前端只消费状态，避免与现有 240ms 定时器相互竞争。

### 5. 权限、桌面与生命周期边界

- 基础窗口控制需要辅助功能授权，用 `AXIsProcessTrustedWithOptions` 核验；现有完全磁盘访问不能替代。只有引入画面预览时再考虑屏幕录制权限。[Apple 信任检查](https://developer.apple.com/documentation/applicationservices/1459186-axisprocesstrustedwithoptions)
- Apple 将辅助类 Accessibility API 列为 App Sandbox 不兼容功能；当前独立 App 路径可继续评估，沙盒分发需另做设计。[Apple Sandbox](https://developer.apple.com/documentation/security/protecting-user-data-with-app-sandbox)
- 桌宠能跨 Space 显示，不等于外部窗口能跟随。首版不主动跨 Space 搬运；外部窗口不在当前桌面时退回普通唤起流程，接受可能切换桌面。[Hammerspoon Spaces](https://www.hammerspoon.org/docs/hs.spaces.html) 将其控制标为依赖私有 API 的实验功能。
- 解除收纳和正常退出时尽力恢复原位置及状态。0.3 不再依赖 Dock 最小化恢复；外部屏外停放必须保留恢复信息，异常退出后重新启动 Demo 确认恢复候选。不能依赖退出回调处理强制终止。
- 持久化只保存必要的恢复信息；PID、AX 对象或窗口 ID 不能作为跨进程重启后的稳定身份。重新启动时只恢复能够可靠匹配的存活窗口，歧义时让用户重新选择，不能仅凭相同标题操作窗口。

## 涉及模块

以下为生产应用接入建议，尚未修改这些生产模块。独立 Demo 的模块见下方交付记录。

| 模块 | 为什么涉及 |
| --- | --- |
| 新增 `src-tauri/src/window_drawer/` | 管理绑定、状态机、互斥展开、恢复信息及失败降级，独立于任务业务 |
| 新增平台窗口适配层，例如 `platform/external_window.rs` | 封装 AX、AppKit、坐标转换和能力探测，业务层不散落底层窗口调用 |
| `platform/permissions.rs`、`frontend/lib/permissions.ts` | 补齐辅助功能状态检查与授权引导 |
| `DockPanel.svelte`、`presentation.ts`、`PetApp.svelte` | 增加入口、与原生保持区域协调、调整覆盖范围和收起决策 |
| `pet/surface`、`platform/pet_surface.rs` | 读取顶栏屏幕位置、安全区与显示器变化；继续只管理桌宠自身 frame，不把外部窗口硬塞进现有 Surface 状态 |

## 风险

| 风险 | 验证或缓解 |
| --- | --- |
| AX 写入被拒绝、窗口最小尺寸超限 | 属性探测后写入并回读实际 frame；失败保留普通窗口，显示明确原因 |
| 鼠标进入原窗口后立即误收起 | 测试联合区域、连接通道、边缘容差、静止鼠标及快速反向 |
| 面板拖动时窗口滞后，或与用户的原窗口拖动争抢位置 | 合并位置更新、松手后回读；独立拖动时暂停跟随；录屏验证不同 App 的实际延迟 |
| 菜单、中文候选或文件面板操作被中断 | 实测原生/跨进程弹窗；识别不确定时暂停自动收起，提供固定模式 |
| 激活带出同 App 其他窗口、收起后抢回焦点 | 对比 raise 与 activate；仅在目标仍为前台且用户未切到其他 App 时考虑交还之前焦点 |
| Spaces、全屏、Stage Manager 改变窗口归属与前台组 | 首版限制普通桌面；分别测试后再扩大支持范围，不能从 NSPanel 配置推导外部窗口能力 |
| 双屏坐标、刘海或断开显示器导致错位 | 独立 AX/AppKit 坐标适配；覆盖负坐标、上下屏、主屏切换、不同缩放；丢失显示器时回退可见工作区 |
| 崩溃、权限撤销、睡眠后失联 | 保存恢复信息；模拟终止/撤权/唤醒；AX 超时只表示暂时失败，不当成窗口已经关闭 |

## 测试计划

1. **隔离原型**：先验证 TextEdit、Terminal、Safari，再测试 Chrome、VS Code、微信等不同窗口体系。它们是测试候选，不是已确认支持列表。每个 App 检查多窗口、中文输入、复制粘贴、滚动、拖放、右键菜单、文件选择及关闭后的行为。
2. **决策门槛**：能控制单窗口而不误隐藏同 App 其他窗口；鼠标进入原窗口后保持；离开延迟收起；正常退出可恢复。收起后必须验证不可见且没有进入最小化状态，同时检查恢复闪动和焦点变化。
3. **定向自动化**：实现后测试状态机的延迟/反向/拖动冻结/权限失败/窗口销毁，以及多屏坐标换算；沿用 `npm run test:pet`、`npm run test:pet:ui` 验证任务面板回归。只测试新逻辑和已有契约，不用源码字符串断言替代交互验证。
4. **真实桌面验收**：用真实 AppKit/AX 窗口验证层级与输入链，并录制展开/收起过程；浏览器模拟和 frame 数值不能替代桌面合成像素验收。按现有[原生动效排查要求](../40-runbooks/pet-native-motion-flicker.md)执行。

本次仅运行源码检索、系统版本/签名 entitlement 读取和公开资料核验；未运行功能原型或兼容性测试，也未修改业务代码。

## 知识沉淀

本文件保存调查证据和候选方案。原型通过后更新支持范围及实测结果；确定长期方案后再写架构决策，并同步顶部功能文档。若出现可重复的恢复问题，再沉淀 runbook，不提前把假设写成规约。

## 未知项

- 本机及计划支持的 macOS 版本中，逐 App 的 AX 支持、非最小化隐藏能力及恢复实际耗时。
- 顶部抽屉的期望是否必须包含连续上滑收纳；当前公开能力不能保证这一视觉。
- 不激活 App 的 hover 展示能覆盖多少窗口体系，以及首次点击是否只完成激活。
- 菜单、输入法候选、Stage Manager、跨 Space 唤起及混合缩放的实测行为。
- 屏外停放能否在目标系统与显示器排列下达到可接受的残留和恢复表现。

## 独立 Demo（2026-11-17）

本节记录 0.1 的历史交付；最小化行为已由下方 0.3 修正替代。

用户要求先提供演示版后，新增 [macOS 窗口抽屉原型](../../prototypes/macos-window-drawer/README.md)，使用原生 Swift/AppKit 独立运行。产物为 `artifacts/window-drawer-demo/Code Pet Drawer.app`，bundle ID 为 `com.codepet.window-drawer-demo`，不加载正式桌宠的任务或设置。

- `main.swift`：顶部入口、单窗口选择、AX 适配、真实 NSWindow 示例、固定/释放/退出、窗口跟随和异常恢复候选确认；解决独立运行及真实输入路径验证。
- `DrawerPolicy.swift`：纯悬停策略和 AppKit/AX 坐标换算，便于定向验证时序与多屏坐标。
- `LogicTests.swift`：覆盖短暂停留、重入取消收起、拖动/固定保护、手动关闭抑制及坐标/边界。
- `build.command`、`open.command`：本机编译、ad-hoc 签名、逻辑测试与重复启动入口，避免每次启动重新签名。

当前已通过 Swift 编译、纯逻辑测试及真实 AppKit 示例的 native smoke；后者覆盖收纳、展开、固定、跟随位置更新、最小化、再次展开、状态保留和释放恢复。CUA 界面验证了中文文本编辑、按钮计数、固定展开和选择器布局；文本编辑使用设置文字操作，不代表中文输入法候选链验收。高位 NSPanel 的 CUA 坐标拖动返回 `windowNotFoundAtPosition`，因此不声明真实鼠标拖动链已通过；位置跟随由 native smoke 调用实际协调器验证。

本机 Demo 显示辅助功能尚未授权，未自动修改该授权。外部 AX 控制代码已实现，但其他 App 的完整流程尚待用户授权后验证。示例是本进程窗口，其成功不能替代跨进程兼容性测试。Demo 收起采用系统最小化；菜单/文件面板/输入法候选的通用自动归属保护未实现，界面与说明明确要求这些场景先使用「固定」。

### 0.2 正文容纳区修正（2026-11-18）

证据为用户标蓝框截图及旧 Demo `desiredFrame`：旧版只有 520pt 宽顶栏，目标窗口独立出现在其下方，缺少包住正文的容器。

- `DrawerPolicy.swift` 增加正文布局与输入范围计算，`LogicTests.swift` 覆盖左右边缘、负坐标屏、窗口包含、顶栏分离及输入区域；解决只定位窗口而未定义容器内部区域的问题。
- `main.swift` 将原生容器扩展为完整外框，正文按真实窗口位置绘制透明开孔；控件锚定顶栏，原窗口显示在内部。鼠标位置以 25ms 周期更新输入透传，AX frame 回读仍低频执行。解除绑定或收起时恢复小顶栏；拒绝容纳失败的窗口并尝试恢复；切换无关 App 时收起，避免开孔展示其他窗口。
- 顶部精确定位通过 `TopPanel.constrainFrameRect` 和实际 AppKit frame 检查保证。原生验证中的 shell 为 `(70, 510, 780, 570)`，目标窗口为 `(90, 530, 740, 462)`，顶栏为 `(70, 1004, 780, 76)`；窗口完全位于黑色外框内且不与顶栏相交。

编译、逻辑测试、native smoke 通过。新增 smoke 检查实际包含关系、透明开孔 alpha、跟随及收起/释放后正文外框清理；CUA 验证了容器内示例的按钮动作。`embedded-preview.png` 由本应用真实 NSView 渲染结果合成并逐图检查，展示正文内的原生示例窗口；它不是系统桌面截图。单窗口截图不包含开孔后方的另一个原生窗口，不能据此把空白开孔当成窗口未显示。外部 App 授权与兼容性、真实鼠标透传延迟、物理刘海/多屏仍待实机验证。

风险验证：透明开孔必须既有像素透明也有输入透传，分别用像素检查和真实鼠标点击验证；窗口最小尺寸超限应恢复而非溢出；无关 App 激活时必须收起，避免正文内容错配；快速拖动的跨进程窗口同步仍需桌面录屏检查，原生几何测试不能代替合成帧验收。

本轮执行：`prototypes/macos-window-drawer/build.command`、带 `CODEPET_DRAWER_PREVIEW` 的原生 `--smoke`、两个启动脚本的 `zsh -n`、产物的 `plutil -lint` 和 `codesign --verify --deep --strict`、`git diff --check`；全部通过。CUA 在新版完整外框上尝试坐标拖动仍返回 `windowNotFoundAtPosition`，未声明真实鼠标拖动通过。没有用户辅助功能授权，未运行外部 App 窗口兼容性测试。

### 0.3 收起只留顶部条（2026-11-18）

用户再次用截图明确：收起应是同一收纳组件只剩顶部条，不能将原窗口最小化到 Dock。证据为旧版 `bind` 与 `tuck` 调用 `minimize(true)`，以及顶栏「收起使用系统最小化」文案。该实现与用户要求不符，已移除。

- `WindowTarget.conceal`：内置示例用 `orderOut` 隐藏同一 NSWindow；外部窗口通过 AX 尝试完全屏外停放并回读确认，与任一显示器仍有交集时恢复原位且提示不支持。没有最小化或整 App 隐藏的兜底。展开与释放仍使用原窗口，保留内容和原生操作。
- `DrawerGeometry` 与 `LogicTests`：屏外候选使用全部显示器的外包矩形，拒绝停在副屏、保留 1px 边缘、无显示器或无效 frame；防止将窗口仍可见误报为收起。
- 原生 smoke：绑定与收起后均断言 `!isVisible && !isMiniaturized`，同时检查顶部条尺寸、正文开孔清理、重新展开状态保留和释放恢复。导出 `collapsed-preview.png`，已视觉核对为仅顶部条。

构建、逻辑测试与 native smoke 通过；执行命令沿用 `build.command` 和 `--smoke`，增加 `CODEPET_DRAWER_COLLAPSED_PREVIEW` 输出。CUA 已观察到新版顶部条，并在展开正文中通过坐标点击让原生编辑窗口获得焦点，验证了内置示例的真实鼠标透传。外部 App 仍未授权，不能将内置 `orderOut` 的成功外推为任意 App 均支持；严格屏外停放对会钳制窗口位置的 App 可能不可用，必须保持错误可见，禁止静默回退最小化。

## 正式桌宠技术栈对照与接入建议（2026-11-18）

用户认可 Demo 的收纳效果，要求先对标技术栈再讨论正式接入。本节为源码核对和候选接入方案，没有修改正式实现。

### 技术栈与已验证边界

| 层次 | Swift Demo 0.3 | 正式桌宠当前源码 | 接入判断 |
| --- | --- | --- | --- |
| 界面 | Swift + AppKit 控件，非 SwiftUI | Svelte 5、TypeScript、CSS；macOS 由 WKWebView 呈现 | 保留现有前端，只增加收纳入口和正文承载区 |
| 原生外窗 | 自建 `TopPanel: NSPanel` | Tauri 2（lock 为 2.11.2）+ vendored `tauri-nspanel`；`PetOverlayPanel` 已是透明、非激活 NSPanel | 同一类系统窗口，无须另起一个 Swift App |
| 系统调用 | AppKit、ApplicationServices/AX | `objc2` 0.6.4、`objc2-app-kit`/`objc2-web-kit` 0.3.2，已有主线程 AppKit 调用 | 可以在 Rust 平台层调用同组 API |
| 外部窗口 | `WindowTarget` 串行 AX 队列，保存原 frame，移动/隐藏/恢复 | 目前没有外部窗口会话模块 | 迁移行为与契约，不直接复制 Demo 的单文件控制器 |
| 动效 | 原生 frame 即时切换 | `DockPanel` + `dockPanelMotion.ts` 单进度 rAF；原生 WKSnapshot 覆盖层交接 | 必须接入现有动效生命周期，不能再并行跑一套 |
| 鼠标 | 25ms 原生位置检测，进入开孔时透传 | `PetApp.svelte` 60ms 检测并调用 `setIgnoreCursorEvents`；Surface 原生拖动循环为 16ms | 收纳模式必须明确透传写入方，避免前后端定时器争抢 |
| 构建 | `swiftc` 编译独立 .app | Vite + Cargo/Tauri；当前 `build.rs` 仅调用 `tauri_build::build()` | 首选沿用 Rust 构建链；没有必须引入 Swift 的证据 |

Rust 侧可用 [objc2-application-services](https://docs.rs/objc2-application-services/latest/objc2_application_services/) 的 AX 绑定，和现有 objc2 依赖体系一致。本轮在仓库之外创建临时 Cargo 项目，固定 0.3.2，仅启用 AX 所需功能，实际编译并运行 `AXUIElement::new_application`、`AXIsProcessTrusted`，完成 CF 对象释放。输出见 `artifacts/window-drawer-demo/rust-native-api-probe.log`。该探针没有弹授权提示、枚举或改动用户窗口；它证明 Rust 调用链成立，不证明外部窗口可收起，也不代表正式 Code Pet 的授权状态。正式依赖及 feature 组合仍需在主项目中编译验收。

尤其注意：Demo 内置窗口由 Demo 自己拥有，可以 `orderOut`；正式桌宠对第三方 App 没有这种所有权。Swift/Rust 都面对同一 macOS 边界。外部窗口的无最小化收起应作为独立验收门槛，不能从语言迁移可行推导出兼容性已经解决。

### 推荐边界

维持 `Svelte UI → Tauri 命令/事件 → Rust 收纳会话 → macOS 平台适配`。外部窗口仍由原 App 绘制并处理输入。推荐在同一 Code Pet 进程中实现，不把独立 Demo 当作第二个常驻进程。

| 模块（含建议新增路径） | 责任及修改原因 |
| --- | --- |
| `frontend/lib/pet/NotchPet.svelte` | 增加窗口入口/正文内容切换；保留任务业务、角色及既有任务 DOM 身份 |
| `DockPanel.svelte`、`dockPanelMotion.ts` | 继续拥有顶栏、外框和呈现动效；增加正文开孔与转场准备/完成通知，不在组件里操作 AX |
| `frontend/lib/pet/surface.ts`、`pet/surface/geometry.rs` | 扩展测量协议，报告相对窗口的正文 `hostRegion`，校验有限数值、边界和 revision；不在 Rust 猜测 CSS 顶栏高度 |
| `pet/surface/mod.rs` | 继续是桌宠自身 frame 的唯一写入者；原生拖动应用 frame 后，将实际位置变化交给收纳会话跟随，不逐帧绕前端 IPC |
| 建议新增 `src-tauri/src/window_drawer/` | 管理选择、原状态、会话 generation、串行操作、恢复及外部能力状态；不接管桌宠的窗口布局 |
| 建议新增 `platform/external_window.rs` 及 macOS 实现 | 封装 AX 对象、权限、坐标转换、目标窗口 frame 和隐藏策略。业务只用统一接口，其他平台返回能力不支持 |
| `PetApp.svelte`、平台鼠标策略 | 收纳期间将透传写入权交给原生一方，前端报告控件区域和消费 pointer 状态；不得继续与原生 25ms 循环同时写忽略鼠标状态 |
| `platform/permissions.rs`、本地 Tauri 命令注册 | 补辅助功能检查与引导，延续现有权限入口边界；完全磁盘访问不能替代 AX 授权 |
| `webcontent-contract.json` | 当前后端 API 为 13，实施新命令/布局契约时递增并同步前端兼容检查；不直接修改生成的 pet SDK 或扩展 Provider/Remote 协议 |

AX 目标引用应留在串行原生执行器内，前端仅拿会话句柄、能力、尺寸和状态；AppKit/WKWebView 操作仍在主线程，不把阻塞 AX 消息放进 UI 高频循环。AppKit、Surface 和 AX 坐标分别转换：Surface 的 `y=-AppKitY` 与 AX 的 `y=primaryTop-AppKitMaxY` 原点不同，不能只翻转一次 y 后混用。

### 三处必须先接好的差异

1. **透明与输入是两个条件。** 当前 `.notch-backdrop` 盖满正文，`PetApp.collectPetHitRects` 又把整块背景当成命中区。只放一个透明占位 div 会得到黑色遮挡，或能看见但无法点击。正文开孔必须贯穿所有背景层，鼠标策略在 hostRegion 内优先透传，顶栏按钮仍由桌宠接收。停用收纳模式后恢复原有任务模式策略。
2. **真实窗口不参与 CSS 裁剪、淡入和缩放。** 展开先完成外框布局，再确认目标实际 frame 位于正文中，最后开放透明区域及输入；收起先遮住正文、隐藏目标并回读确认，再缩外框。隐藏失败则恢复开孔并保持展开。初版不让真实窗口跟随 260ms CSS 动画逐帧缩放；`WKSnapshot` 只包含 WebView，不包含第三方窗口，覆盖层也必须保持开孔语义。
3. **尺寸和保持区域不能照搬任务列表。** `petSizing` 当前任务展开宽上限 640、高上限 440，许多 App 放不下。收纳模式应基于显示器工作区和目标最小尺寸单独计算，并保留原收起尺寸。鼠标进入真实窗口后 WebView 会收到 leave/blur，必须用原生联合区域补充保持信号；`DockPanel` 仍决定呈现，不新增另一套独立开关计时器。

### 接入顺序与验收

1. **先做 Rust 原生能力验证。** 用隔离入口验证真实外部窗口的选择、移动、正文定位、无最小化隐藏和恢复；先选一个窗口、一个屏幕。不能完全藏起时返回不支持，不能退回最小化。也用 Rust 自建窗口覆盖本进程 UI 测试，但两种测试分开记录。
2. **再接透明正文和输入。** 沿用真实 DockPanel/Surface，验证 WKWebView 的透明开孔、顶栏可点、正文原生点击、任务模式回退。已有 `pet_surface_qa` 的隔离测试模式可扩展，避免改真实任务数据。
3. **最后接动效与生命周期。** 增加异步准备/完成握手和会话失效处理；覆盖反向、拖动、脱离顶部、切换任务正文、窗口关闭、WebView 重载、撤权、屏幕拔插和退出恢复。

风险与验证对应：双写透传用顶栏↔正文来回点击验证；跨进程同步滞后用桌面录屏验证；快照只含 WebView 用透明像素与真实桌面合成联合验证；隐藏失败用会钳制位置的外部 App 验证保持展开；身份/迟到请求用释放后不再移动旧窗口的状态测试验证。权限需由正式 Code Pet 单独获得，Demo 的授权不能当作正式产物授权。

实施后的定向检查沿用 `npm run test:pet`、`npm run test:pet:ui`、`npm run test:pet:motion`、`build:webcontent` 和 `test:webcontent`，补 Rust 收纳会话、坐标及能力失败测试，再进行真实外部 App 验收。本轮只执行源码/测试检索、临时 Rust AX 编译运行和文档检查；未修改正式代码，未运行正式窗口控制或无关全量回归。确定方案并验证外部隐藏能力后，再将长期边界整理为架构决策。
