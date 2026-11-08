# 桌宠窗口体验

## 行为

桌宠提供独立桌面浮窗和顶部吸附面板两套界面，共享任务数据和展示状态机。桌面展示完整角色、任务气泡和抽打互动；顶部收起为状态条，hover 展开深色任务列表。

## 当前事实

- `frontend/PetApp.svelte` 由 `pet.html` 加载。
- 桌宠窗口加载设置、轮询 Pet Gateway 的 `pet.snapshot`，并监听 `settings-updated`。
- 新的实时活动可以展开已经收起的任务列表。
- 桌面透明度由 `settings.pet.opacity` 控制，顶部面板保持不透明。
- 原生 Surface 统一控制尺寸与拖动；前端测量实际内容，窗口限制在屏幕边界内。

## 产品约束

- 桌宠需要在用户阅读其他内容时仍然有用，因此透明度和任务列表密度都重要。
- 窗口高度贴合实际内容，任务区有滚动上限；非命中区域通过原生 `setIgnoreCursorEvents` 透传，角色按当前图集帧的 alpha 命中。
- 内置橘猫/熊猫；支持抚摸、悬停、拖动反应和任务状态动画，素材与模块边界见 [角色与动画模块](../30-domains/pet-window/character-animation.md)。
- 当前 Pet Gateway 仅提供任务展示，桌宠不包含回复或审批 UI。

## 验证

- 修改布局时检查 `frontend/lib/pet/DesktopPet.svelte`、`NotchPet.svelte` 与宿主 `frontend/PetApp.svelte`。
- 修改共享行为后运行 `npx vitest run frontend/lib/pet/model.test.ts frontend/lib/pet/presentation.test.ts frontend/lib/activityGroups.test.ts frontend/lib/sound.test.ts`。
- 布局、焦点、透传与拖动模拟分别由 `scripts/pet_notch_ui_qa.mjs`、`pet_window_frame_ui_qa.mjs`、`pet_module_ui_qa.mjs` 验证；原生多屏行为仍需实机验证。
