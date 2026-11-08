# 桌宠透明区域事件透传

## 规则

透明桌宠窗口的非命中区域必须使用窗口级鼠标事件透传，不能只依赖 CSS `pointer-events: none`、移除拖拽层或前端事件吞吐来表现为“不响应”。

## 适用场景

修改 `frontend/PetApp.svelte`、桌宠窗口尺寸、透明背景、拖拽命中区、宠物图片渲染、任务卡片交互或 Tauri window capability 时适用。

## 反例

让透明区域不触发拖拽，但窗口本身仍接收鼠标事件。用户看到透明区域，却无法点击或滚动其下方应用，尤其在固定大窗口高度下会形成大面积屏幕事件拦截。

## 推荐做法

通过统一 Surface 平台接口读取窗口内逻辑鼠标坐标；macOS 的 AppKit 点坐标不能与 Tauri 物理像素混算。只在任务卡、操作按钮、宠物不透明像素和顶部可见面板等命中区域内恢复 `setIgnoreCursorEvents(false)`，其余区域设置 `setIgnoreCursorEvents(true)`。新增或调整 Tauri JS window API 时同步检查 `src-tauri/capabilities/default.json`。

图集命中必须读取渲染组件公布的当前行、列和帧边界，不能将整张图或第一帧当命中区域。静态图片 `object-fit: contain` 产生的留白必须扣除；装饰性情绪叠层不应扩大鼠标拦截区。实现统一放在 `frontend/lib/pet/hitTest.ts`，不再从旧像素 DOM 或 CSS 变换猜测位置。

透明事件透传与拖动边界必须分别验证。原生窗口应贴合实际布局并保留有限装饰边距；不能用透明区域可透传来证明大窗口没有问题，因为整窗工作区限位仍会让透明留白提前触顶。调整内容高度应保持桌宠角色锚点（停靠时保持停靠区下缘），并在低高度工作区缩小任务滚动区。

## 来源

用户反馈：透明区域在 app 内不响应，但事件不会透给其他窗口；需要的不是禁止拖拽响应，而是真正透传。历史提交 `754581a fix(pet): 让透明区域鼠标事件穿透` 也说明该行为曾作为 Bug 修复出现过。

## 验证方式

运行 `npx vitest run frontend/PetApp.test.ts frontend/lib/petHitTest.test.ts`。人工验证时在 Windows 和 macOS 上检查：透明区域可点击下方窗口，任务卡/按钮可点击，宠物不透明像素可拖动，宠物透明像素尽量不拦截下方窗口。

图集和图片留白另运行 `npx vitest run frontend/lib/pet/hitTest.test.ts`；`scripts/pet_module_ui_qa.mjs` 使用真实素材验证 alpha 和原生调用边界，但不能替代操作系统实测。

内容高度和拖动边界另运行 `scripts/pet_window_frame_ui_qa.mjs`，覆盖单卡/无任务、展开收起、顶部边界和不同 DPI；原因与证据见 [透明留白阻止上移](../40-runbooks/pet-window-transparent-frame-blocks-drag.md)。


## 刘海与拖动补充

原生拖动事务期间禁止工作区强制限位，也不能切回鼠标透传，否则无法进入吸附带或丢失松手。前端 CSS 布局改变后必须测量角色锚点，原生根据 revision 接受新布局；不得再由前端和原生分别 setPosition/setSize。拖动结束或取消后恢复透传检查。

新增回归：Surface 几何/状态测试、前端旧布局回复测试，以及顶部→悬浮→顶部的原生拖动验收。实现与证据见[接入实施记录](../30-domains/pet-window/notch-docking-design.md)。

顶部状态条与下拉内容是一个完整命中区域，不能只命中里面的按钮，否则 hover 会穿过列表空隙。头像裁切后 alpha 检测必须同时受头像外框限制，不能让裁掉的身体扩大命中区。`scripts/pet_notch_ui_qa.mjs` 覆盖两套 UI 的展开、焦点与尺寸。

展开宽度与收起宽度可以不同，但展开后的当前活动栏必须与列表共用容器；只允许任务区滚动。外侧圆角需要有计入内容测量的有限边距，不能超出原生 frame 被裁掉。头像视口不要小于渲染帧宽度，以免左右耳朵被切掉；固定上部裁切仍不等同于任意图片的头部识别。


屏幕比例布局必须从原生当前显示器的逻辑工作区计算；禁止用桌宠自身 viewport 的百分比推导下一次原生窗口尺寸，否则 resize 会反馈到自身尺寸并反复缩放。尺寸预算是上限，实际高度必须按内容收缩，角色和操作栏空间先于滚动区保留。验证顶部 QA 的多屏尺寸矩阵、窗口 QA 的混合 DPI 与稳定 resize 次数；依据见 [布局与尺寸](../30-domains/pet-window/layout-and-sizing.md)。
