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
- taskLedger.css 的详情使用 overflow-y:auto、scrollbar-width:none 和 WebKit 隐藏规则；overflow-x:hidden 是兜底，必须另验 scrollWidth 不超 clientWidth。隐藏不代表禁止滚动。

## 验证方式

`node scripts/task_ledger_ui_qa.mjs` 及 WebKit 模式覆盖长标题、长路径、变更文件、12 轮运行、1280/820/390、明暗主题、单行高度、无横向溢出与 PageDown；实际修复后详情 clientWidth=568、scrollWidth=568。`codex_workbench_ui_qa.mjs` 覆盖共享滚动条与同步自远端的侧栏卡片/拖拽行为。

本轮只改前端，不提升原生 API 29。资源替换按 [前端资源热加载](../10-architecture/webcontent-hot-reload.md) 构建完整包、安装新数据槽位，再从 App 关于页加载；不得逐个覆盖散列文件或改签名包。安装与当前页面构建时间必须分别核对。
