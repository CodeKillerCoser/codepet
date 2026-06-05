# 架构

Code Pet 是一个 Tauri 2 应用，前端使用 Svelte，后端使用 Rust。前端负责 UI 状态和交互体验；后端负责本地集成点、持久化、事件接入和平台 API。

## 主要区域

- `runtime-topology.md`：窗口、collector、IPC、托盘和本地运行流。
- `frontend-backend-boundary.md`：Svelte 与 Rust 的职责边界。
- `event-pipeline.md`：hook payload 到桌宠任务卡片的路径。
- `settings-persistence.md`：设置模型和存储流程。
- `window-system.md`：桌宠窗口尺寸、停靠和屏幕边界。
- `agent-control.md`：hook、激活、回复和审批边界。
- `cross-platform-boundaries.md`：macOS、Windows 和不支持路径的边界。

## 模块证据

- `frontend/` 包含 Svelte 入口和前端库。
- `src-tauri/src/activity/` 包含 collector、事件、标题解析和 Token 用量。
- `src-tauri/src/agent/` 包含 Agent 注册、hook 管理、provider 控制和 provider 专属辅助逻辑。
- `src-tauri/src/app/` 包含设置、共享状态、日志、自启动和 CLI 辅助逻辑。
- `src-tauri/src/pet/` 包含宠物库、图片处理、主体抠图和主题默认值。
- `src-tauri/src/platform/` 包含平台专属窗口代码。
