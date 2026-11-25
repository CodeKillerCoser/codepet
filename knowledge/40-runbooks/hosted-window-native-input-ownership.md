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

- `platform/window_drawer/macos.m`、`watch.h/m`：移除自动拖出释放。已展示窗口的按下不再成为拖入来源，不增加外部松手计数，也不触发“其他窗口按下”失效通知。展示区未改变时，被动回读保留原 App 自己移动的位置；尺寸跟随改读窗口几何，不依赖正文鼠标按下对象。被动展开只 raise，显式选择才激活 App。
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
