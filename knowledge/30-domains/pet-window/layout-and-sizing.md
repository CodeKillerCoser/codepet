# 布局与尺寸

## 当前尺寸模型

桌宠窗口在 `frontend/PetApp.svelte` 中使用预设逻辑宽高。用户手动缩放被禁用，但应用仍会调用 `setSize()`，确保窗口 frame 可预测。

任务卡片内容不应依赖固定卡片高度。回复编辑器和 footer 间距需要在卡片中自然撑开。

## 已知 UI 约束

- 只有一个任务卡片时，不应出现无意义列表滚动条。
- 操作按钮存在时，footer 间距仍需可见。
- 回复模式可以覆盖到桌宠窗口区域上方，但控件必须保持可触达。
- 文本不能覆盖卡片 footer 或按钮。

## 验证

- 人工检查单卡、多卡、回复模式、审批模式和收起任务列表。
- 布局变更后运行 `npx vitest run frontend/PetApp.test.ts frontend/lib/activity.test.ts`。
