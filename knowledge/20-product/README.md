# 产品区域

这个目录记录面向用户的产品行为。它应该描述产品当前真实能力，而不是路线图愿望。

## 文档

- `pet-window-experience.md`：桌宠悬浮窗行为。
- `task-card-interaction.md`：任务卡片内容、操作、移除、回复和审批。
- `personalization.md`：宠物、气泡、声音、主题和透明度设置。
- `notification-and-approval.md`：需要注意的状态和审批流程。

## 证据来源

- `frontend/App.svelte`：设置 UI。
- `frontend/PetApp.svelte`：桌宠窗口行为。
- `frontend/lib/activity.ts` 及测试：任务卡片规则。
- `frontend/lib/sound.ts` 及测试：通知行为。
