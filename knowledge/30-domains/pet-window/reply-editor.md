# 回复编辑器

## 当前行为

回复模式由 `frontend/PetApp.svelte` 中的 `replyingToId` 和 `replyText` 控制。

打开后，textarea 会获得焦点，光标会移动到末尾，编辑器高度根据 `scrollHeight` 调整。最大高度为五行，计算依据包括 computed line height、padding 和 border。

在 macOS 上，桌宠悬浮 panel 只有在需要时才允许成为 key window，使回复 textarea 可以接收键盘焦点。

回复模式可以通过取消按钮退出；提交成功后会清空；目标 activity 消失或失去回复能力时也会清空。

## 能力依赖

只有当 `activityCapabilities(activity).canReply` 为 true，且卡片尚未处于回复模式时，才显示回复按钮。

## 风险

- 条件渲染可能让 textarea 在点击处理之后才挂载；focus 必须发生在 DOM 更新之后。
- 列表中复用一个 `replyTextarea` binding 只有在同一时间只有一个卡片进入回复模式时才安全。
- 自动调整高度不能迫使整个任务卡片布局依赖固定高度。

## 验证

- 人工点击回复，确认光标出现在编辑器里。
- 输入超过五行，确认编辑器内部滚动。
- 取消回复，确认 footer 操作恢复。
