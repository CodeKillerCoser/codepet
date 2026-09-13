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


## 主窗口入口归属（2026-09-13）

主导航的“宠物制作”负责预览、抠图导入、像素化与宠物库管理。主题、任务气泡、窗口不透明度、互动音效归“设置 → 外观与桌宠”；任务通知声音、静音时段与机器人归“设置 → 通知”；资源目录归“设置 → 通用 → 数据与存储”。这些移动沿用原设置字段和保存 API。

完整分区与验证记录见 `settings-organization-proposal.md`。真实 App 的导航可通过浏览器 QA 入口 `frontend/qa/main-window.html` 在模拟 IPC 下复查；此入口不参与生产构建。
