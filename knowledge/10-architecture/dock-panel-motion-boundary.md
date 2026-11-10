# 顶部面板动效的组件边界

## 背景

2026-11-10，用户担心任务业务迭代破坏已完成的展开优化。改动前，`PetApp.svelte` 持有 hover 定时器和动画实例，`notchMotion.ts` 通过 `.latest-title`、状态点、头像等业务选择器测量、克隆和逐节点驱动动效。修改标题结构会同时改变动画依赖。原生画面交接已有独立 Surface 接口，不需要重做。

## 决策

采用一个具体的 `DockPanel.svelte`，不建立多种桌宠 UI 的统一抽象接口。

- `NotchPet.svelte` 保留角色、任务排序、完成/取消分组、消息和操作，通过具名插槽提供顶栏内容，通过默认插槽提供任务列表。它不创建动画，也不维护 hover 定时器。
- `DockPanel.svelte` 拥有稳定的顶栏、正文容器、200ms 展开/240ms 收起定时器、焦点与整条顶栏拖动。继续使用 `presentation.ts` 的同一纯状态机；最新输入立即生效。收起时正文保留挂载，通过 `inert`、可见性及裁剪退出交互，不重建任务节点。
- 私有 `dockPanelMotion.ts` 只修改组件持有的 frame 节点。宽、高、窗口偏移、安全区和单一展开进度由同一 rAF 驱动；CSS 根据进度布局状态区、操作区和正文。字号及角色图像不缩放，不查询业务后代，不克隆业务 DOM。
- `PetSurfaceController` 仍负责测量/规划/提交原生窗口和 begin/end 画面交接。macOS 的 WKSnapshot 覆盖窗口留在平台层；API 12 不变。角色锚点及原生拖动范围沿用已有测量协议。

测量目标尺寸时，同步把组件自身放到目标布局，测量后立即恢复当前帧，再进入异步原生交接。展开先扩容原生窗口并补偿屏幕偏移，收起先完成动效再缩窗。正文使用目标宽度排版，只裁剪可见区域，避免文字在中途反复换行。

原生交接期间合并到最后一次目标；动画期间反向从当前帧接续。内容观察器只观察组件容器，内容更新先标记重新测量，在当前交接结束后处理，避免逐帧写原生窗口。卸载、失败必须释放覆盖层并恢复自动测量。

## 备选方案

- 只把旧动画代码移到独立文件：文件边界更清楚，但业务选择器、克隆节点与逐元素同步依赖仍在。
- 建立通用动画引擎或多模式 UI 接口：当前只有一种顶部容器需要这套交接，会增加配置和实现分支。
- 对整块内容 scale：实现简单，但字体、图像会变形；也没有解决 AppKit 与 WebView 画面时钟不同的问题。

## 取舍理由

具体组件把固定结构和动效约束放在一起，插槽保留业务节点、列表展开状态及焦点。任务开发者修改内容时不用了解原生快照时序。代价是收起时仍维护列表 DOM，以及容器宽高的布局/绘制开销；列表本身的分组和数量限制沿用现有行为。

完整交互回归发现：收缩和安全区变化会在静止鼠标下合成 enter/leave。组件忽略动效几何变化引起的 hover 切换，真实 pointermove 仍立即进入状态机，防止 Escape 后自动重开。

## 影响范围

`PetApp.svelte` 移除顶部动效及定时器，继续拥有共享数据、Surface 和光标透传；`NotchPet.svelte` 专注任务；`DockPanel.svelte` 和其私有驱动承担交互与呈现。删除旧 `notchMotion.ts`。桌面悬浮 UI、自定义角色导入、Provider 协议和原生实现不变。QA 的关闭判断改为不可交互状态，不能再要求正文节点消失。

## 验证与后续观察

- `npm run test:pet`：41 项通过，包含新增内容变高、反向、交接期间合并请求、卸载和失败恢复测试。
- `npm run test:pet:ui`：任务堆叠、导入图集、焦点/Escape、后台 hover、拖动冻结、四档屏幕、刘海安全区与固定顶栏通过。
- `npm run test:pet:motion`：8 种位置/反向场景和真实 gateway 在展开期间更新内容通过；标题、头像和正文节点保持身份。`uv run --with pillow python scripts/pet_motion_images_qa.py` 检查 312 张浏览器帧的头像像素轨迹。
- `npm run build:webcontent`、`TMPDIR=/private/tmp npm run test:webcontent` 通过。原生 `pet_surface_qa --motion` 的展开/收起/反向共 132 帧通过几何检查，报告在 `artifacts/dock-panel-refactor/native-frames.json`。
- `.github/workflows/pet-ui-quality.yml` 在相关 PR/main 变更时执行单元、UI、连续帧及像素检查并保存产物；Playwright 固定为开发依赖。首轮 GitHub 运行发现键盘导航起点与动画终态等待问题，见 [UI CI 修复记录](../40-runbooks/pet-ui-ci-keyboard-timeout.md)。
- `npx tsc --noEmit` 未通过：仓库已有 Node/CSS 类型声明和其他测试 fixture 错误；对 HEAD 的临时副本执行同一命令也失败，本次新增动效文件没有报告错误。构建仅有既存 `ProviderRuntimeInfo.svelte` 未使用选择器提示。

原生坐标采样不等于桌面合成像素验收。完整桌面录屏、真实刘海/混合 DPI、原生鼠标拖动仍遵循[原生闪烁排查](../40-runbooks/pet-native-motion-flicker.md)的实机验证要求。本次构建前端包和隔离原生测试，未替换 Applications 中已安装的应用。
