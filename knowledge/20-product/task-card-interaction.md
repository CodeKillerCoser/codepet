# 任务卡片交互

## 当前数据与展示

桌宠通过标准 Gateway 的 `notice.list` 与 `notice.changed` 消费共享 `Notice`。Provider 解释来源 Hook/消息并发布 Notice，Host 负责持久化与路由；前端 `pet/model.ts` 负责 revision 合并、最新排序、关闭历史和连接错误。私有 Pet Gateway 和来源事件投影已删除。

桌面模式使用 `NoticeGroups.svelte`，由 `finished` 和已启用的回复/授权动作分成等待交互、进行中、已结束；界面不解释接入方的状态枚举。每组可展开，组状态存于宿主，模式切换保留。顶部模式使用 `pet/NotchPet.svelte` 的横向任务列表，固定顶栏，列表单独滚动。

标题、来源、实际状态与最近更新时间来自 `content.title`、`pluginId`、`status.label` 和 `updatedAt`；顶部列表用左侧 provider 图标标识 Agent，不重复显示右侧名称徽标；时间为 updatedAt，不是持续运行时长。Markdown 经过统一净化；桌面消息保持一行，顶部最多两行，链接仍可打开。顶部缺少正文时明确提示“暂无进度说明”。

`content.progress.text` 展示 Provider 提供的进度正文，`content.progress.percent` 可提供数值进度。来源回复、提示词或工具描述如何转换为正文由 Provider 决定，Host 和界面不推断来源私有字段。

## 操作与边界

支持隐藏任务、桌面清除已完成任务、模式切换、打开主窗口、角色互动。隐藏记录在该任务有更新前有效，两套 UI 共用。回复、授权入口仅根据 Notice 已启用的标准 action 展示，经 `notice.action.execute` 路由回 Provider。原始 Agent Hook 不自动获得这些动作。

2026-11-25 按用户要求移除通知卡片的“打开应用”按钮。该入口由 2026-11-16 的提交 `3a47f544` 随 Notice 动作支持引入；`NoticeActions.svelte` 是顶部面板与桌面气泡的共用动作组件，本次从其可展示动作中排除 `activate` 并删除直接执行分支。只有激活动作的通知不渲染操作行，混合动作的通知继续显示回复和授权；协议与 Provider 动作契约保持兼容。`NoticeActions.test.ts` 覆盖激活入口隐藏、操作行无空白占位，以及回复聚焦和授权执行，浏览器 QA 检查两套展示与窄屏布局。

本次通过 `npx --no-install vitest run frontend/lib/NoticeActions.test.ts`（3 项）、`npm run check:frontend`（0 错误、5 项既有警告）、`npm run check:source-size` 和 `git diff --check`。浏览器验证使用 `node artifacts/remove-open-app-qa.mjs` 临时 runner，在 `scripts/pet_notch_ui_qa.mjs` 的 Notice 更新中注入激活动作，确认两种模式均隐藏入口、保留回复，并通过原有焦点、动作可见性及响应式矩阵；截图位于 `artifacts/pet-notch-qa/`，临时 runner 验证后删除。未运行原生 WKWebView 人工验收。

顶部将 `finished: true` 的条目按完整 pluginId 归组，保留一个“已结束”标题。组收起时，垂直滚轮循环切换顶层任务，遍历整组而非仅三张可见堆叠层；不同组分别记忆选择，展开后恢复原排序和正常列表滚动。展开再收起保留所选任务；任务被隐藏、恢复运行或移出该组后回到组内第一项。单条任务、横向手势和捏合缩放不拦截。触控板微小增量累加到阈值才翻页，连续翻页间隔至少 160ms；减少动态效果偏好关闭堆叠位移动画。`NotchPet.svelte` 负责滚轮、展开与任务分组，`planeStackMotion.ts` 只负责 Plane Cards 运动，不改变共享任务顺序或协议。

收起的组使用纵向轨道：卡片平面始终与屏幕平行，位置沿上下和纵深方向循环，以平移、等比缩放、连续文字透明度和遮挡层级呈现新卡上浮、旧卡退后。缩放沿用原堆叠的 600px 透视与 18px 层间距，卡片宽度限制在原宽的约 94.3%–100%，不扩大前后层的宽度差或改变窗口基础宽度。弹簧驱动连续进度，反向输入保留当前位置与速度，停止后吸附到整张卡片。虚拟三槽轨道最多渲染三张卡片，后侧接缝淡出换入，两个任务使用对向槽位。静止时恢复原来的向下 11px/22px 紧凑堆叠，不保留上方露出或额外轨道留白；只有两个整数停靠点之间才连续混入环绕轨迹，停靠前后混合权重及其斜率归零，避免开始/停止时跳变。卡片高度与原堆叠占位固定，避免每帧触发原生窗口缩放；窄屏保留标题、状态与两行消息的空间。展开时取消动画、恢复原列表布局；偏好减少动态效果时直接切换。

## 风险与验证

新增动作需检查协议、键盘焦点、窄屏可见性和原生透传。隐藏堆叠卡不得拦截鼠标；模式切换不可产生第二条任务订阅。运行 `frontend/lib/pet/model.test.ts`、`activityGroups.test.ts` 及顶部/桌面分组浏览器 QA，原生拖动和透传另在目标系统验证。

`npm run test:pet:ui` 使用真实组件和模拟原生 IPC，覆盖整组轮换、正反向循环、触控板增量、分组独立性、展开后的原生滚动、隐藏/恢复任务、键盘切换及窄屏布局。`npm run test:pet` 检查共享模型和状态机。浏览器 QA 不能替代 macOS 原生 WKWebView 的真实触控板惯性手感验收。

`npm run test:pet:stack` 对真实组件使用可控浏览器时钟，按浏览器测试时钟的 16ms 帧间隔保存前进、反向及中途反向的截图和矩阵数据，逐帧检查平面朝向、位移/缩放/文字连续性、顶栏和容器稳定性，并检查展开后的原生滚动与减少动态效果。产物在 `artifacts/pet-plane-stack-qa/`；GitHub Pet UI quality 保留截图和报告。该验收使用模拟 IPC，尚未覆盖原生 WKWebView 的合成与触控板惯性。
