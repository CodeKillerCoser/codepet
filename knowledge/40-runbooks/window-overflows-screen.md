# 窗口超出屏幕

## 现象

桌宠窗口越过显示器边缘，或无法完整看到。

## 需要收集的证据

- 平台和显示器数量。
- 显示器坐标，包括负坐标。
- 窗口 outer position 和 outer size。
- 问题发生在停靠、拖动、resize 事件还是跨屏移动时。

## 排查步骤

1. 检查 `frontend/PetApp.svelte` 中的 `monitorForWindow()`。
2. 检查 `clampWindowPositionToMonitor()`。
3. 确认 move 和 resize 事件后会调度 `ensureWindowFrameAndBounds()`。
4. 确认选中的屏幕工作区正确。

## 修复后验证

- 人工拖动到每个屏幕边缘。
- 人工跨屏拖动。
- 确认边界限制后任务列表仍然可见。
