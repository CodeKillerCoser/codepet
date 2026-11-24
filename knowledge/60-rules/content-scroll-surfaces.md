# 内容区滚动、收缩与局部隐藏滚动条

## 规则

主窗口滚动条统一使用主题色和细宽度；页面隐藏滚动条时必须保留滚轮与键盘滚动。可滚动详情中的子元素不得通过负外边距或最小内容宽度撑出横向滚动，不能仅靠裁切掩盖越界内容。

## 适用场景

AppContentPane 的固定导航与滚动内容、任务列表/详情、运行卡片和全局侧栏。外层页面滚动条继承主窗口样式；任务详情是明确的隐藏例外。

## 证据与反例

2026-11-24 用户截图显示任务详情同时出现粗纵向与底部横向滚动条。长标题/路径样例在 1143×949 视口复现：详情 clientWidth=553、scrollWidth=556。`1ab3ecc9` 的 TaskRunDetail 运行导航使用 `margin:-3px` 扩展焦点留白；即使仅多 3px，macOS 仍显示整条横向滚动条。

## 推荐做法

- 框架内容导航之外的顶部留白由 `main-window.css` 管理，导航后的内容 padding-top 为 12px，不在业务页另加空白块。
- 主题滚动条使用 `theme/main-window.css` 的浅色 thumb/hover token，主窗口统一标准属性与 WebKit 伪元素。
- TaskRunDetail 导航不使用负外边距，运行卡片可换行；卡片标题/子项显式 min-width:0，焦点环内收。任务列表标题单行省略并保留完整 title。
- TaskRunDetail 的 .task-run-scroll 使用 overflow-y:auto、scrollbar-width:none 和 WebKit 隐藏规则，.task-detail 仅负责布局、overflow:hidden；overflow-x:hidden 是兜底，必须另验 scrollWidth 不超 clientWidth。隐藏不代表禁止滚动。

## 验证方式

`node scripts/task_ledger_ui_qa.mjs` 及 WebKit 模式覆盖长标题、长路径、变更文件、12 轮运行、1280/820/390、明暗主题、单行高度、无横向溢出与 PageDown；实际修复后详情 clientWidth=568、scrollWidth=568。`codex_workbench_ui_qa.mjs` 覆盖共享滚动条与同步自远端的侧栏卡片/拖拽行为。

本轮只改前端，不提升原生 API 29。资源替换按 [前端资源热加载](../10-architecture/webcontent-hot-reload.md) 构建完整包、安装新数据槽位，再从 App 关于页加载；不得逐个覆盖散列文件或改签名包。安装与当前页面构建时间必须分别核对。

## 后续前端替换的回归防线

2026-11-25 用户反馈多个滚动条均未通过。实际 App URL 的 builtAt=1795536216571（v19），原生截图同时显示粗深纵向条与底部横向条，搜索框和长标题也回到旧布局。v19 样式不包含本规则的隐藏/细浅样式；构建工作区仍停在 a7b05b6c，加上未提交的抽屉调宽改动，缺少主干 d61ff23b。v17 自测通过不能作为 v19 的验收证据。

替换前核对 origin/main、构建工作区 HEAD 与未提交差异，并记录源码组合；独立工作区的新功能不得覆盖已交付修复。本次用 d61ff23b 加只读导出的抽屉差异，在隔离目录构建，保留两边工作区原状。合并包需同时跑任务滚动和抽屉调宽回归。安装后核对实际 URL builtAt，并在运行中的 App 检查外层、详情、横向三处滚动；人工失败保留在同一次执行，后续自测不覆盖用户结论。

## 固定任务信息与局部分栏（2026-11-25）

用户截图要求任务标题、描述、基本信息不随执行正文滚动。TaskRunDetail 的 task-info 与 task-run-scroll 是并列区域，只有后者承担执行滚动；框架 content 在任务视图不再承担外层滚动，列表单独滚动。不能只给标题加 sticky，仍让背景和项目时间被卷走。长背景展开时限制头部高度并允许独立读取，不能挤掉正文；切换任务或执行后正文回顶，保留原有键盘滚动和隐藏滚动条。

左侧列表默认 280 px，可调范围 220～440 px，并根据内容区宽度进一步限制，为详情保留空间；小于 700 px 维持已有单栏返回行为。分隔条支持鼠标/触屏、键盘、双击复位、Escape/pointercancel/失焦/卸载取消。宽度只在当前 App 会话保留，临时变窄不覆盖偏好。

回归用 task_ledger_refinement_qa.mjs 的真实 pointer、Home/End/方向键、取消、窄屏来回及头部矩形不变检查；完整 App 短窗口还要检查页签、筛选、任务信息三处位置和 content.scrollTop=0，不再用“外层能滚动”作为通过条件。
