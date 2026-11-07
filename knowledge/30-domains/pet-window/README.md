# 桌宠窗口领域

本领域覆盖桌面角色、顶部状态条和任务面板，以及内容尺寸、吸附、拖动、透明命中和自定义角色。

## 当前实现

- `frontend/PetApp.svelte`：宿主设置、定时器、共享状态与原生命令装配。
- `frontend/lib/pet/DesktopPet.svelte`、`NotchPet.svelte`：两套具体 UI，共用 `model.ts` 数据与 `presentation.ts` 状态机。
- `frontend/lib/pet/surface.ts`：自然内容测量与原生 Surface 通信；原生几何和拖动事务位于 `src-tauri/src/pet/surface/`，平台差异在 `platform/pet_surface.rs`。
- `frontend/lib/pet/PetAvatar.svelte`、`assets.ts`、`animation.ts`、`hitTest.ts`：统一角色素材、动画与 alpha 命中；自定义图片/图集导入继续使用 `src-tauri/src/pet/`。

任务来自 Pet Gateway，窗口不消费旧 `PetEvent` 归并或 companion 投影，也不提供旧回复/审批编辑器。布局、数据边界与验收见 [两套 UI 与共享状态](two-views-and-shared-state.md)。

## 验证

运行 `npx vitest run frontend`、原生 `pet::surface` 与宠物库集成测试，以及 `scripts/pet_notch_ui_qa.mjs`、`pet_window_frame_ui_qa.mjs`、`pet_module_ui_qa.mjs`。跨平台原生拖动仍需对应系统验收。
