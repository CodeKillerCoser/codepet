# 前后端边界

## 前端职责

- 渲染主窗口和桌宠悬浮窗。
- 读取 Pet Gateway 投影，供桌面与顶部两套视图共享任务、来源和关闭记录；主窗口独立展示近期事件。
- 管理 hover、焦点、展开和布局，通过 Surface IPC 请求原生窗口操作。
- 当前桌宠不提供回复、审批与打断入口；旧前端事件归并、能力判断和通知重复计时链已移除。
- 应用主题 class 和 CSS token。

相关模块：`frontend/App.svelte`、`frontend/PetApp.svelte`、`frontend/lib/pet/model.ts`、`frontend/lib/pet/presentation.ts`、`frontend/lib/theme/`。

## 后端职责

- 安装和移除托管 hook 项。
- 运行本地 collector 并归一化原始 hook payload。
- 持久化设置和宠物库数据。
- 处理审批等待、发送已验证回复，并激活受支持目标。
- 提供平台专属窗口设置和本地日志。

相关模块：`src-tauri/src/agent/`、`src-tauri/src/activity/`、`src-tauri/src/app/`、`src-tauri/src/platform/`。

## 规则

前端操作按钮必须由与后端能力兼容的逻辑驱动。不要因为原始 hook 事件包含展示数据，就暴露后端无法可靠执行的按钮。
