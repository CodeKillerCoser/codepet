# 前后端边界

## 前端职责

- 渲染主窗口和桌宠悬浮窗。
- 将近期事件归并为用户可见任务卡片。
- 根据 provider capability 决定卡片操作是否可见。
- 管理焦点、回复编辑器状态、通知重复计时器和布局。
- 应用主题 class 和 CSS token。

相关模块：`frontend/App.svelte`、`frontend/PetApp.svelte`、`frontend/lib/activity.ts`、`frontend/lib/agentInteractions.ts`、`frontend/lib/theme/`。

## 后端职责

- 安装和移除托管 hook 项。
- 运行本地 collector 并归一化原始 hook payload。
- 持久化设置和宠物库数据。
- 处理审批等待、发送已验证回复，并激活受支持目标。
- 提供平台专属窗口设置和本地日志。

相关模块：`src-tauri/src/agent/`、`src-tauri/src/activity/`、`src-tauri/src/app/`、`src-tauri/src/platform/`。

## 规则

前端操作按钮必须由与后端能力兼容的逻辑驱动。不要因为原始 hook 事件包含展示数据，就暴露后端无法可靠执行的按钮。
