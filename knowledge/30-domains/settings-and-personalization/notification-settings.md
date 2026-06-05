# 通知设置

## 当前设置

`NotificationSettings` 包含：

- 选中的声音。
- 自定义声音路径。
- 权限、失败和完成状态的响铃开关。
- 等待审批的重复秒数。
- 静音时段。

抽打反应音存放在 `PetSettings` 下，因为它属于宠物互动，而不是任务通知。

## 运行时行为

`frontend/PetApp.svelte` 在新事件到达时调用 `handleRing()`。它使用 `frontend/lib/sound.ts` 判断是否响铃、静音时段、声音播放和重复行为。

## 验证

- 运行 `npx vitest run frontend/lib/sound.test.ts`。
- 人工确认审批被处理或 activity 消失后，重复审批提醒会停止。
