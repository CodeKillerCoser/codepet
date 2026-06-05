# 领域知识

领域文档用于连接产品行为、源码模块和测试。

## 领域

- `agent-events/`：hook 接入、归一化、活动归并和过滤风险。
- `agent-control/`：provider capability、Codex app-server、Qoder remote-control 边界、回复和审批。
- `pet-window/`：悬浮窗布局、尺寸、屏幕边界和回复编辑器。
- `settings-and-personalization/`：设置模型、宠物个性化和通知。
- `theme-system/`：主题 token 库和迁移规则。

## 维护

当 Bug 反复触及某个领域时，更新该领域的 `known-risks.md`，并考虑在 `../60-rules/` 下新增可复用规约。
