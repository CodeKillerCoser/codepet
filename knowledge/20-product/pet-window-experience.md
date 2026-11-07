# 桌宠窗口体验

## 行为

桌宠窗口是用于日常活动监控的透明悬浮界面。它展示当前宠物形象、任务卡片、操作按钮、通知消息和抽打互动。

## 当前事实

- `frontend/PetApp.svelte` 由 `pet.html` 加载。
- 桌宠窗口加载设置、轮询 Pet Gateway 的 `pet.snapshot`，并监听 `settings-updated`。
- 新的实时活动可以展开已经收起的任务列表。
- 宠物透明度由 `settings.pet.opacity` 控制，并应用为 `--pet-window-opacity`。
- 窗口由程序控制尺寸，并限制在屏幕边界内。

## 产品约束

- 桌宠需要在用户阅读其他内容时仍然有用，因此透明度和任务列表密度都重要。
- 窗口高度贴合实际内容，任务区有滚动上限；非命中区域通过原生 `setIgnoreCursorEvents` 透传，角色按当前图集帧的 alpha 命中。
- 内置橘猫/熊猫；支持抚摸、悬停、拖动反应和任务状态动画，素材与模块边界见 [角色与动画模块](../30-domains/pet-window/character-animation.md)。
- 不受支持的任务阶段不能出现回复或审批 UI。

## 验证

- 修改布局或操作状态时，检查 `frontend/PetApp.svelte`。
- 修改卡片行为或声音行为后，运行 `npx vitest run frontend/lib/activity.test.ts frontend/lib/sound.test.ts`。
