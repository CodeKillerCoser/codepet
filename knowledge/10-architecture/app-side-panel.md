# App 框架右侧第三栏与抽屉

## 背景与目标

2026-11-24 用户明确要求框架级第三栏或抽屉。先前 `2031dfc2` 的工作树详情虽然使用共享 Dialog，但容器和定位仍属于工作台页面，不能算作 App 框架能力。现在将容器、布局和生命周期提升到主窗口框架，业务页只登记内容。

## 实现与边界

每个 `App.svelte` 实例调用 `provideSidePanel()`，通过 Svelte context 提供独立 controller；同一个 App 只有一个 `AppSidePanelHost`，作为内容区的同级节点。controller 不是进程单例，也不接受或扫描 Codex 数据。

业务组件使用 `shell/AppSidePanel.svelte` 提供 open、title、description、关闭回调、返回焦点的元素和 children snippet。该组件不渲染容器，只按自身 owner 登记或撤销内容。新的 owner 替换旧内容时通知旧页关闭；旧页晚到的卸载不能清掉新页的面板。不得在业务页添加备用 host，否则会重新形成页面私有抽屉。

框架在窗口宽度至少 1180 px 时，将原来的导航与内容网格扩成第三列，默认宽 400 px，支持拖动调宽；无面板时不保留宽度。第三栏不加遮罩或限制背景焦点，可以直接选择列表另一行或使用全局导航。导航收起时，内容继续使用释放的宽度。

较窄窗口由同一个 host 使用共享 Dialog 显示右侧抽屉，统一提供遮罩、焦点约束、标题和关闭按钮。显式关闭恢复原触发元素焦点；路由切换清理面板且不抢走导航焦点。新内容替换已经排队的焦点恢复时，旧恢复动作失效。详情自身的关键词、归档等业务状态仍留在页面。

打开预览等前景弹窗时，框架暂缓第三栏与抽屉之间的切换；关闭前景弹窗后应用最新窗口尺寸，避免重建底层面板改变弹窗栈。MutationObserver 只在这一待切换阶段启用，完成后断开。所有容器使用 App 主题 token，业务页不参与定位和断点判断。

## 内容区域的统一样式

用户进一步要求主内容区背景和样式一致。工作台原先将 `--wb-bg` 接到窗口外壳 `--app-bg`，并以 92% 宽度自行留白；现在它指向内容画布 `--app-surface`，工作台和任务账本填满框架内容区。主窗口的 `.content`、标题栏及侧栏左右内边距统一使用 `--app-content-inset`：常规 24 px、840 px 以下 20 px、600 px 以下 12 px。侧栏正文的内边距由 host 管理，页面不重复加边距。

2026-11-24 用户截图验收否决了标题在圆角卡片外的实现。当前用同一个 `.panel-card` 包裹 `.panel-header` 与 `.panel-body`，整张卡片与主内容区共用画布色、轮廓色和 16 px 圆角。标题与正文仍为独立区域，但标题必须在圆角背景内；左侧使用业务方必填的 title，右侧仅放关闭按钮。窄屏抽屉同样包裹完整标题与正文，卡片外围使用 App 壳层背景。

浏览器验收比较工作台、设置页和右侧第三栏的计算样式，检查浅暗主题下背景、边框、圆角相等，以及页签与框架内边距对齐。任务账本额外运行 `scripts/task_ledger_ui_qa.mjs`，覆盖两个主题及 390–1280 px。

## 进退动画与全局窗口层级（2026-11-24）

证据：原共享 Dialog 使用固定的遮罩 1000、内容 1001，且默认关闭 portal。工作台预览与框架抽屉由不同 Root 渲染，后挂载的抽屉可能覆盖预览；页面祖先的布局、裁切或堆叠上下文也可能限制弹窗。此前只检查预览 DOM 和编辑器内容，没有检测实际点击命中，未能拦住此类遮挡。具体引入提交未确认。

现在 `ui/dialog/windowStack.ts` 统一管理当前 WebView 的模态窗口层级，每次打开分配高于此前窗口的一对遮罩/内容层级。共享 `Dialog.Content` 默认移到最近 `.main-theme` 内唯一的 `.app-window-host`，避开业务页及侧栏容器，同时保留主题继承。关闭动画仍占用旧层级，新窗口始终更高。通过共享入口记录实际点击的触发元素，补足 WebKit 不聚焦鼠标点击按钮时的返回焦点；调用方显式处理关闭焦点时仍优先使用其回调。

`Dialog.Modal` 提供 showModal / close / isOpen 的轻量入口，用同一套 Dialog、portal、窗口栈和焦点能力替换工作台来源/错误提示及任务页四个原生 dialog，避免浏览器 top layer 与应用 z-index 混排。任务表单保留保存期间禁止 Escape 和关闭按钮的行为；正文样式仍由各业务组件管理，弹窗定位和层级统一由共享组件负责。

桌面侧栏使用 220 ms 进退过渡，App 网格固定保留零宽第三列以便连续展开到用户选定宽度；退出期间保留内容但设为 inert。窄屏使用共享 Dialog 的右侧滑入/滑出及遮罩渐变，动画结束释放内容引用；减少动态效果时停用动效。验收截图等待动画结束，同时单独逐帧检查位移与主区宽度变化，避免把过渡帧误当最终布局。

新增受影响模块：`ui/dialog` 负责全局挂载、层级和焦点；`ui/ui.css` 负责共享遮罩动效；`TaskLedger.svelte` 与 `CodexWorkbench.svelte` 替换原生弹窗，其中任务弹窗移出容器后改用 viewport 媒体查询保持窄屏页脚排版。工作台“更多”菜单复用项目选择器的 pointerdown 焦点保持，防止 WebKit 在按钮/复选框 click 之前触发 focusout 收起菜单。

验证与回归防线：`windowStack.test.ts` 覆盖打开顺序、乱序清理、嵌套退出与共享挂载位置；工作台浏览器 QA 在 Chromium 和 WebKit 检查预览的实际点击命中、焦点、动态层级、主题切换、逐层 Escape、抽屉及第三栏的进退过程、减少动态效果、390–1440 px 和真实 App 导航。任务布局 QA 使用此前原生验收快照作浏览器回放，覆盖四种表单、忙碌时 Escape 不关闭及返回焦点；不代表本轮重新跑过原生任务验收。共享窗口回归另运行 `npm run test:ui` 和 `CODEPET_QA_ENGINE=webkit npm run test:ui`。层级约束见 [全局窗口层级与焦点](../60-rules/global-window-layers.md)。

本轮上述浏览器检查均通过，并覆盖侧栏退出途中快速重新打开。`npx --no-install vitest run frontend` 共 231 项通过；`npm run check:frontend` 无错误（6 条既有警告），`npm run check:source-size`、`npm run build`、`git diff --check` 通过。最新截图在 `artifacts/codex-workbench-qa/`，WebKit 在 `webkit/` 子目录，窄屏叠层示例为 `preview-390.png`。本次未修改 Rust 或重新打包原生应用。

## 用户验收修正：完整卡片与可调分栏（2026-11-24）

证据：用户提供的实际 App 截图中，标题 main 位于灰色背景，白色圆角只从正文开始；源码 `5c9ade3b` 的 `.panel-body` 承担圆角背景，而分栏轨道固定为 400 px。这与“圆角背景包裹标题和内容，分栏可拖动调宽”的验收要求不符；上一轮自动化通过不能替代本次用户验收。

修复将背景、边框和圆角提升到标题与正文共同的 `.panel-card`；标题保持可见，正文独立滚动。新增 `shell/SidePanelResizeHandle.svelte`，统一管理分隔条的 pointer capture、键盘操作、容器测量和取消清理。分栏轨道由 `sidePanel.ts` 的 width/bounds/resizing 状态驱动，App 和隔离 QA 框架只绑定 `--app-side-panel-width`，业务内容不管理分栏尺寸。

默认轨道宽 400 px，最小 320 px、最大 760 px，同时按导航和窗口可用宽度限制上限，为主内容保留 480 px。拖动期间暂停网格过渡，避免指针移动与分栏宽度脱节；展开和收起仍保留动画。双击分隔条恢复默认；左右键按 16 px 调整，Shift 按 50 px，Home/End 到边界；Escape 或 pointercancel 取消拖动且不关闭面板。关闭、切页再返回会保留当前 App 会话选择；窗口临时缩小时限宽，重新变宽后恢复偏好。窄屏抽屉不显示分隔条，也不改变桌面宽度偏好。未将宽度写入磁盘。

回归防线：`sidePanel.test.ts` 覆盖宽度上下限、无效值、关闭后保留、临时限宽恢复及 App 实例隔离；工作台 Chromium/WebKit QA 使用真实鼠标拖动，检查主内容与侧栏同步、拖动期间无过渡、键盘/双击/取消、窗口尺寸边界和抽屉往返。卡片验收同时检查标题与正文都是 `.panel-card` 的直接子元素、标题矩形位于卡片内，以及浅暗主题的卡片计算样式，避免再次只检查正文圆角。桌面新示例截图为 `artifacts/codex-workbench-qa/app-shell-resized-column.png`，WebKit 在同目录 `webkit/` 下；均为固定数据样例。

风险与验证：拖动中取消或页面卸载可能遗留 pointer capture/调整状态，组件在取消、失焦和卸载时清理，并用真实拖动取消验证；过窄窗口可能挤压主区，用 1180 px 限宽与 390 px 抽屉往返验证；标题背景再次遗漏，用结构、几何和截图三种检查覆盖。此前全局预览点击命中、层级、焦点、主题及进退动画回归同时执行。本轮未替换已安装 App 或重跑原生安装包验收。

本轮结果：`node scripts/codex_workbench_ui_qa.mjs` 与 `CODEPET_QA_ENGINE=webkit node scripts/codex_workbench_ui_qa.mjs` 通过；`npx --no-install vitest run frontend` 共 233 项通过；`npm run check:frontend` 无错误（6 条既有警告）、`npm run check:source-size`、`npm run build` 和 `git diff --check` 通过。上述为开发验证，尚待用户重新验收。

## 受影响模块

- `frontend/lib/shell/sidePanel.ts`：按 App 隔离的登记、替换、撤销、关闭、焦点恢复与分栏宽度/约束状态。
- `shell/AppSidePanel.svelte` 与 `AppSidePanelHost.svelte`：分别定义页面内容入口和框架唯一展示容器。
- `App.svelte` 与 `main-window.css`：创建框架上下文、挂载 host、切页清理及三列网格。
- `CodexWorkbench.svelte`：移除局部详情 Dialog、标题栏与几何样式，只提供工作树内容；扫描与 IPC 不变。
- `frontend/qa/WorkbenchFrameQa.svelte`：独立页面验收也挂载生产框架 controller/host，不用页面内 fallback。浏览器和原生验收入口相应更新。

## 风险与验证

旧页卸载误清新内容、跨窗口串状态、关闭后抢焦点：`shell/sidePanel.test.ts` 覆盖 owner、App 实例隔离、导航关闭和延迟焦点取消。完整 App 的结构、列宽与切页清理交给工作台浏览器 QA，不能只验收独立组件截图。

模式切换破坏预览、窄屏焦点逃逸、背景误遮挡：浏览器覆盖桌面直接换行选择、背景搜索聚焦、预览打开时缩放、窄屏嵌套预览先关闭、关闭抽屉后返回原行、收起导航和浅暗/系统主题；390–1440 px 检查溢出与筛选控件可读宽度。

本轮验证命令：`npx vitest run frontend`（224 项通过）、`npm run check:frontend`（0 错误，6 条既有警告）、`npm run check:source-size`、`node scripts/codex_workbench_ui_qa.mjs`、`npm run test:ui`、`npm run build`。原生生产组件与 IPC 验收找到当前工作树和当前对话，验收时 9 项未提交与独立 Git 基线一致，重检与真实文件预览通过。报告和截图在 `artifacts/codex-workbench-qa/`；浏览器多条列表为样本数据。

## 未知项

本轮未实测 Windows 原生窗口，也未实际触发 Finder、终端或 Codex 深链。未将旧抽取历史等非当前入口批量迁移到新框架。本轮不修改后端接口或资源契约。
