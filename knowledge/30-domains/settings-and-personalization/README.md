# 设置与个性化

这个领域覆盖持久化设置、宠物库数据、通知设置和个性化 UI。

## 源码模块

- `src-tauri/src/app/settings.rs`：持久化设置结构、默认值、加载和保存。
- `src-tauri/src/pet/library.rs`：宠物库和图片导入。
- `src-tauri/src/pet/subject_cutout.rs`：主体抠图。
- `frontend/App.svelte`：设置 UI，以及保存前的归一化。
- `frontend/PetApp.svelte`：消费实时设置更新。

## 测试

- `src-tauri/tests/settings_tests.rs`
- `src-tauri/tests/pet_library_tests.rs`
- `src-tauri/tests/subject_cutout_tests.rs`
- 前端声音、气泡颜色和主题相关 helper 测试。
