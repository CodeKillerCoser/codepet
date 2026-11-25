# 收纳窗口反复出现激活提示、点击或轻拖后被释放

## 现象

2026-11-25 用户反馈 macOS 收纳窗口频繁显示“激活某窗口”，正常点击或轻微拖动后会退出收纳。用户进一步明确：收纳正文的鼠标事件归原 App，拖动、移入移出、选区和缩放都应遵循原窗口行为；Code Pet 只处理自己的顶栏与 Dock 控件。

## 证据

- 修改前 `platform/window_drawer/macos.m` 的 `detachMovedWindow` 只要求光标和原窗口分别移动 8 点，没有要求离开展示区，也可能在松手前删除窗口身份。`show`、`park`、`drop` 等请求都可能执行这一判断。
- 修改前 `WindowDock.svelte` 的同步异常分支调用 `restoreWindow`，原生适配失败也会调用 `restore`。因此读取或移动暂时失败会进入恢复原窗口、移除 Dock 条目的路径，普通点击本身不必触发拖出判断。
- 原生 `show` 的首次展开曾调用 `activateEntry`，包含 App 激活、AXFrontmost、AXMain 和 AXRaise；前端仅为已确认可见的小矩形设置透传，焦点通知撤回这些矩形后，正文会重新接收鼠标。
- 旧日志 `code-pet.log.1` 有 2026-11-23 的 `visible/covered` 交替，以及 `fit failed ... measured {{0, 0}, {0, 0}}`。这是历史运行证据，不能证明本次用户手势具体命中了哪条分支。

## 根因

原窗口输入与 Code Pet 的收纳控制混在同一条手势分类链中：原窗口的普通移动被解释为解除收纳，短暂 AX 失败也被解释为恢复并删除条目。透传还依赖当前视觉遮挡结果，焦点或窗口顺序变化能让宿主重新拦截正文点击。被动展开主动激活 App，则进一步增加了焦点变化。

## 引入历史

`9eba9d3e` 引入了按 8 点位移自动解除收纳。适配失败恢复与被动展开激活的完整引入历史未确认。

## 修复方案与涉及模块

- `platform/window_drawer/macos.m`、`watch.h/m`：移除自动拖出释放。已展示窗口的按下不再成为拖入来源，不增加外部松手计数，也不触发“其他窗口按下”失效通知。按住期间只读原 App 几何，松手后对齐展示区；尺寸跟随改读窗口几何，不依赖正文鼠标按下对象。被动展开只 raise，显式选择才激活 App。
- `WindowDock.svelte`、`windowDrawer.ts`：正文排除投放和选择手势；删除 `detached` 返回分支。同步失败保留窗口身份和原始恢复位置，封闭视觉开孔并重试；相同错误连续出现时不重复提示。原生 `fitPending` 保证失败后真正重试适配，避免停在部分移动的中间状态。
- `PetApp.svelte`、`NotchPet.svelte`：稳定展开时整个原生正文区透传，焦点、遮挡或读取失败不再改变该输入归属。移除正文里的激活/重试按钮，恢复与释放放在 Dock。收起、切页和显式释放仍先设置 `data-native-paused`，等原生命中更新完成再移动窗口。布局交接期间继续禁止透传。
- `webcontent-contract.json`：后端接口升至 34，避免新 UI 的事件归属规则与仍会自动解除收纳的旧宿主混用。本轮需要同步更新宿主与 WebContent。

位置采样用于决定宿主自身是否透传，不用于解释或代理原窗口内部事件。当前仍通过整窗鼠标透传切换实现正文和自有控件的分工；真实快速跨边界点击的时序需亮屏验收。

## 验证结果

- `npx --no-install vitest run frontend/lib/pet frontend/PetApp.test.ts`：57 项通过。
- `npm run test:pet:ui`：通过。新增正文按下并移到外部松手不发投放/激活/释放、暂时遮挡不自动激活、同步失败保持条目和透传、恢复后继续展示。保留显式释放等待原生命中确认和失败可重试的回归。
- `npm run test:pet:motion`：连续帧回归通过；`npm run check:frontend`：0 errors、5 个既有 warning；`npm run check:source-size`：无超限。
- `TMPDIR=/private/tmp npm run test:webcontent`：11 项通过。直接运行时一项被 `/var` 与 `/private/var` 的临时目录路径别名断言阻断，使用规范路径后通过，未修改无关测试。
- 原生测试以 `clang -fobjc-arc -framework AppKit -framework ApplicationServices -framework ImageIO src-tauri/tests/macos_drawer_parking.m src-tauri/src/platform/window_drawer/watch.m -o /tmp/codepet-drawer-parking-focus-test` 编译。`--notifications` 通过，验证已展示窗口的按下/松手不会产生拖入来源、外部松手计数或失效通知，并保留移动与 Space 通知回归。
- 完整原生测试在读取本进程样本 AX frame 时失败，发生在新增真实几何用例之前；`CGSessionCopyCurrentDictionary` 返回 `screenLocked=1`。新增的真实窗口移动、适配失败重试和释放恢复用例尚待亮屏执行，不能声明已通过跨进程实机验收。

## 回归防线与规约

约束同步到[桌宠鼠标透传规约](../60-rules/pet-window-click-through.md)。正文手势不能触发解除收纳，视觉可见矩形也不能控制稳定正文的输入归属。明确释放才恢复原始 frame；退出恢复继续保留。

## 未知项与下一步验证

- 在已解锁的 Mac 上运行完整原生测试，检查原生移动/缩放过程中没有位置争抢，释放仍恢复原始几何。
- 用用户实际 App 验收单击、双击、轻拖、选区、拖出再拖回、右键菜单和输入法候选，确认完整事件序列由原 App 接收；特别检查 60 ms 透传采样间隔附近的快速跨边界点击。
- 检查其他 App 遮挡与激活后的真实合成画面，以及恢复 Dock 控件的键盘焦点、动作可见性和小屏布局。浏览器 IPC 模拟不能证明跨进程输入或桌面合成像素正确。

## 松手吸附与缩放位置修正（2026-11-25）

用户在安装 `0.3.12-beta+129c647` 后提供截图：原窗口移到展示区右下方，大片黑色区域与半露出的窗口形成错位；用户要求松手后自动吸附回去，随后补充调整尺寸时两个窗口位置异常。

源码证据有两处：`followPresentedWindow` 在展示区未变化时直接接受实测新原点，没有区分是否松手，导致被动同步永久跳过 `fitWindow`；前端每收到 `resizing` 就更新期望宽高，外层 Surface 按顶部中心重新定位，而系统仍在按用户拖边更新原窗口原点。前者引入于 `129c647b` 的输入归属修正，后者的完整引入历史未确认。

本次将输入处理与几何收尾分开：原生 `tracking` 表示按住期间只读，`WindowDock.svelte` 在此阶段只更新视觉交集并保持展开，固定宿主几何。松手时全局按键状态变化只唤醒同步，不发送投放、激活或释放命令；未对齐的原点必须进入 AX 适配。缩放时先采用应用最终尺寸、约束到屏幕预算，再统一对齐。`windowDrawer.ts` 的 `tracking` 字段与 QA mock 同步扩展，WebContent 后端接口升至 35，防止旧 UI 忽略原生跟踪状态后继续逐像素改动宿主。

`macos_drawer_parking.m` 更新真实窗口回归：移位 12/80 点和返回原处期间不写回，松手后归位且条目保留；左上角变化、放大、缩小、超过预算时按住不移动宿主，松手完成尺寸协商与释放恢复。按钮状态作为原生命令输入显式传入测试，避免测试读取用户正在操作的实体鼠标而偶发跳过吸附。通知测试结束清除 fixture 的宿主身份，窗口还原后等待 compositor 更新再识别来源，修复测试间污染。

验证：`/tmp/codepet-drawer-snapback-test --geometry` 与 `--notifications` 通过；前者执行本进程真实 AX 窗口的失败重试、尺寸适配、缩放、松手吸附和恢复，后者执行通知与原生事件排除。普通完整模式曾被当前桌面的其他窗口遮挡，停在 `onscreen.count > 0`，因此新增 `--geometry` 独立入口，不要求把样本抬到用户面板之前。`npm run test:pet:ui` 新增按住缩放时宿主 frame 完全不变、松手才采用新尺寸的断言，已通过。相关 57 项单测、前端检查（0 errors、5 个既有 warning）和源码长度检查通过。第三方 App 的实体边角手势、快速连续拖放和真实合成画面仍需目标安装包验收。

## 黑色外框实时跟随原窗口（2026-11-25）

用户进一步明确：需要黑色外框跟随原窗口的位置和尺寸。上节“按住时固定宿主”是临时稳定方案，由本节替代；松手回顶部的要求保留。

原先同时调整两层时，Surface 按顶部中心放置外框，AX 跟随又试图将原窗口放回正文，形成双向约束。本次只允许单向跟随：按住时原窗口由其 App 控制，`macos.m` 返回实测 frame；`platform/window_drawer.rs` 与 `platform/pet_surface.rs` 在平台边界将 AX 主屏坐标转换为 Surface 坐标。`WindowDock.svelte` 提供窗口原点和正文局部偏移，`NotchPet.svelte` 仅转交统一控制器，`surface.ts` 把临时 `following` 合并进布局请求。

宿主的实际位置仍仅由 Rust `pet/surface` 写入：`x = 原窗口 x - 正文偏移 x × unit`，y 同理；宽高采用新的内容测量结果。临时跟随不修改保存的顶部锚点；规划和提交使用同一个位置计算。拖动期间不向原窗口回写 AX 位置。松手先清除临时布局，再在顶部采用最终尺寸与原点对齐；切页、收起、模式切换和卸载清理跟随。临时 AX 失败保留上一份有效跟随位置，避免按住时无故跳回顶部；全局松手仍清理。UI 同时检查当前按键状态，迟到的原生回包不能在松手后重新跟随。接口升至 37，避免旧宿主忽略临时布局字段；主线已将 36 用于处理历史接口，需保留该功能并使用新的配套版本。

新增回归：Rust Surface 覆盖负坐标、0.5/1/2 单位比例、移动与尺寸变化、松手返回顶部及非法坐标；前端控制器覆盖临时布局更新和清理。UI QA 检查外框屏幕位置加正文偏移等于原窗口原点、放大再缩小、松手和挂起旧回包后的清理。`pet_surface_qa --follow` 增加实际 AppKit 面板 frame 的跟随/复位检查，隔离数据与正式 App。

`cargo test --lib pet::surface`（17 项）、相关 Vitest（58 项）、`npm run test:pet:ui` 和 `npm run test:pet:motion` 已通过；`npm run check:frontend` 为 0 errors、5 个既有 warning。真实 AppKit 示例 `pet_surface_qa --follow` 输出 `Native following: Ok(())`，确认平台实际应用移动与尺寸变化，并在清除跟随后返回屏幕顶部。源码长度与空白检查通过。本地 Cargo 使用已有编译缓存，测试环境通过 `TAURI_CONFIG` 清空 bundle resources，避免未生成 WebContent 的资源路径阻断编译；不修改产品打包配置。

限制：两个跨进程窗口不能原子更新，当前跟随由 AX/WindowServer 通知、IPC 合并及按住时 50 ms 补查驱动；几何通过不表示连续桌面合成像素已验收。真实第三方 App 的快速缩放、跨屏和中文输入期间仍需连续录屏与实体手势验证，不能承诺所有 App 都完全无延迟。
