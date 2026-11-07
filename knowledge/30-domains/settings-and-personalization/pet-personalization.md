# 宠物个性化

## 当前行为

用户可以选择内置橘猫/熊猫、导入动画包或清晰图片、预览六种任务动作、调整桌宠大小、选择宠物数据目录，并删除导入宠物。内置角色不可删除。未单独配置宠物数据目录时，宠物库默认跟随应用数据目录；已配置 `settings.petLibrary.dataDirectory` 时仍使用宠物库自己的目录。

当前 Path Manager 的目录变更只修改配置，不复制、删除或搬迁原文件。已记录的外部角色路径继续保留，新导入使用新目录。角色模型只保留 builtin、image、codex-atlas；废弃字段读取时忽略，未知类型回退内置角色，保存不再写旧字段。

`settings.pet.opacity` 控制桌宠窗口整体透明度，并在个性化 UI 中暴露。

## 实现

- `src-tauri/src/pet/library.rs` 处理列表、选择、删除、内置档案与选择规范化和导入持久化；`packages.rs` 负责 Codex 兼容包验证/复制/发现，`images.rs` 保留图片清晰度与 alpha。
- `src-tauri/src/pet/subject_cutout.rs` 处理抠图请求。
- `frontend/lib/pet/PetLibraryPanel.svelte` 持有宠物库与动作预览；`frontend/App.svelte` 连接设置和原生命令。
- `frontend/PetApp.svelte` 应用宠物透明度并渲染选中头像。

## 验证

- 运行 `cargo test --manifest-path src-tauri/Cargo.toml --test pet_library_tests --test settings_tests --test subject_cutout_tests`。macOS 环境参数见 [角色与动画模块](../pet-window/character-animation.md)。
- 人工验证导入图片、默认宠物和透明度修改在桌宠窗口中生效。
- 前端与浏览器验证、素材授权、数据迁移边界见 [角色与动画模块](../pet-window/character-animation.md)。
