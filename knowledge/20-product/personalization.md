# 个性化

## 支持范围

- 主题模式：跟随系统、亮色、暗色。
- 宠物外观：默认像素宠物、导入图片、Codex atlas 宠物、像素尺寸和宠物库选择。
- 桌宠窗口透明度：由个性化 UI 控制。
- 运行中气泡：背景呼吸、边框跑马灯、颜色、边框宽度和动画速度。
- 声音：通知音、自定义通知音、静音时段和抽打反应音。

## 实现

- 设置模型：`src-tauri/src/app/settings.rs`。
- 主界面：`frontend/App.svelte`。
- 主题 token：`frontend/lib/theme/`。
- 声音行为：`frontend/lib/sound.ts`。
- 宠物库和图片处理：`src-tauri/src/pet/library.rs` 与 `src-tauri/src/pet/subject_cutout.rs`。

## 验证

- 运行 `npx vitest run frontend/lib/bubbleColorSettings.test.ts frontend/lib/sound.test.ts`。
- 修改持久化默认值或宠物库行为时，运行 Rust 侧 pet/settings 测试。
