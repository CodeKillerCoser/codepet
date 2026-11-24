# App 框架右侧第三栏与抽屉

## 背景与目标

2026-11-24 用户明确要求框架级第三栏或抽屉。先前 `2031dfc2` 的工作树详情虽然使用共享 Dialog，但容器和定位仍属于工作台页面，不能算作 App 框架能力。现在将容器、布局和生命周期提升到主窗口框架，业务页只登记内容。

## 实现与边界

每个 `App.svelte` 实例调用 `provideSidePanel()`，通过 Svelte context 提供独立 controller；同一个 App 只有一个 `AppSidePanelHost`，作为内容区的同级节点。controller 不是进程单例，也不接受或扫描 Codex 数据。

业务组件使用 `shell/AppSidePanel.svelte` 提供 open、title、description、关闭回调、返回焦点的元素和 children snippet。该组件不渲染容器，只按自身 owner 登记或撤销内容。新的 owner 替换旧内容时通知旧页关闭；旧页晚到的卸载不能清掉新页的面板。不得在业务页添加备用 host，否则会重新形成页面私有抽屉。

框架在窗口宽度至少 1180 px 时，将原来的导航与内容网格扩成第三列，宽 400 px；无面板时不保留宽度。第三栏不加遮罩或限制背景焦点，可以直接选择列表另一行或使用全局导航。导航收起时，内容继续使用释放的宽度。

较窄窗口由同一个 host 使用共享 Dialog 显示右侧抽屉，统一提供遮罩、焦点约束、标题和关闭按钮。显式关闭恢复原触发元素焦点；路由切换清理面板且不抢走导航焦点。新内容替换已经排队的焦点恢复时，旧恢复动作失效。详情自身的关键词、归档等业务状态仍留在页面。

打开预览等前景弹窗时，框架暂缓第三栏与抽屉之间的切换；关闭前景弹窗后应用最新窗口尺寸，避免重建底层面板改变弹窗栈。MutationObserver 只在这一待切换阶段启用，完成后断开。所有容器使用 App 主题 token，业务页不参与定位和断点判断。

## 内容区域的统一样式

用户进一步要求主内容区背景和样式一致。工作台原先将 `--wb-bg` 接到窗口外壳 `--app-bg`，并以 92% 宽度自行留白；现在它指向内容画布 `--app-surface`，工作台和任务账本填满框架内容区。主窗口的 `.content`、标题栏及侧栏左右内边距统一使用 `--app-content-inset`：常规 24 px、840 px 以下 20 px、600 px 以下 12 px。侧栏正文的内边距由 host 管理，页面不重复加边距；宽屏第三栏与主内容容器共用画布色、轮廓色和 16 px 圆角。

浏览器验收比较工作台、设置页和右侧第三栏的计算样式，检查浅暗主题下背景、边框、圆角相等，以及页签与框架内边距对齐。任务账本额外运行 `scripts/task_ledger_ui_qa.mjs`，覆盖两个主题及 390–1280 px。

## 受影响模块

- `frontend/lib/shell/sidePanel.ts`：按 App 隔离的登记、替换、撤销、关闭及焦点恢复。
- `shell/AppSidePanel.svelte` 与 `AppSidePanelHost.svelte`：分别定义页面内容入口和框架唯一展示容器。
- `App.svelte` 与 `main-window.css`：创建框架上下文、挂载 host、切页清理及三列网格。
- `CodexWorkbench.svelte`：移除局部详情 Dialog、标题栏与几何样式，只提供工作树内容；扫描与 IPC 不变。
- `frontend/qa/WorkbenchFrameQa.svelte`：独立页面验收也挂载生产框架 controller/host，不用页面内 fallback。浏览器和原生验收入口相应更新。

## 风险与验证

旧页卸载误清新内容、跨窗口串状态、关闭后抢焦点：`shell/sidePanel.test.ts` 覆盖 owner、App 实例隔离、导航关闭和延迟焦点取消。完整 App 的结构、列宽与切页清理交给工作台浏览器 QA，不能只验收独立组件截图。

模式切换破坏预览、窄屏焦点逃逸、背景误遮挡：浏览器覆盖桌面直接换行选择、背景搜索聚焦、预览打开时缩放、窄屏嵌套预览先关闭、关闭抽屉后返回原行、收起导航和浅暗/系统主题；390–1440 px 检查溢出与筛选控件可读宽度。

本轮验证命令：`npx vitest run frontend`（224 项通过）、`npm run check:frontend`（0 错误，6 条既有警告）、`npm run check:source-size`、`node scripts/codex_workbench_ui_qa.mjs`、`npm run test:ui`、`npm run build`。原生生产组件与 IPC 验收找到当前工作树和当前对话，验收时 9 项未提交与独立 Git 基线一致，重检与真实文件预览通过。报告和截图在 `artifacts/codex-workbench-qa/`；浏览器多条列表为样本数据。

## 未知项

本轮未实测 Windows 原生窗口，也未实际触发 Finder、终端或 Codex 深链。未将旧抽取历史等非当前入口批量迁移到新框架。后端接口与资源 API 26 保持不变。
