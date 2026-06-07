# 宠物个性化

## 当前行为

用户可以选择默认 palette 宠物、导入图片宠物、调整图片像素尺寸、选择宠物数据目录，并删除非默认宠物。未单独配置宠物数据目录时，宠物库默认跟随应用数据目录；已配置 `settings.petLibrary.dataDirectory` 时仍使用宠物库自己的目录。

应用数据目录变更时，默认宠物库会随数据目录复制到新位置，托管图片宠物的 `imagePath` 和 `sourcePath` 会改写到新目录；显式配置过宠物库目录时不参与应用数据目录迁移。

`settings.pet.opacity` 控制桌宠窗口整体透明度，并在个性化 UI 中暴露。

## 实现

- `src-tauri/src/pet/library.rs` 处理宠物库列表、导入、选择、删除、数据目录修改、图片像素化和内置 Codex atlas 宠物发现。
- `src-tauri/src/pet/subject_cutout.rs` 处理抠图请求。
- `frontend/App.svelte` 持有个性化控件。
- `frontend/PetApp.svelte` 应用宠物透明度并渲染选中头像。

## 验证

- 运行 `cargo test --manifest-path src-tauri/Cargo.toml pet_library_tests subject_cutout_tests`。
- 人工验证导入图片、默认宠物和透明度修改在桌宠窗口中生效。
