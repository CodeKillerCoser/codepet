# Codex App Server 作为主要回复路径

## 背景

Code Pet 需要一种可靠方式，把任务卡片中的回复发送到 Codex 对话里。

## 决策

对符合条件的 Codex 事件，使用 Codex app-server 作为主要主动回复路径。

## 备选方案

- 终端粘贴：脆弱，依赖焦点、shell 状态和终端应用。
- 辅助功能粘贴：权限面过大，并且对焦点敏感。
- 只依赖 hook 回复：hook 能描述活动，但不是可靠的主动聊天传输通道。

## 取舍理由

本机探测已验证 Codex app-server 可以把消息发送到 Codex app thread，并在 UI 上显示。当前后端通过 `src-tauri/src/agent/codex_app_server.rs` 路由 Codex 回复。

## 影响范围

回复可见性必须要求 Codex、终态状态和 session id。激活仍然走独立路径，因为 app-server 的窗口/打开 thread 方法没有被验证。

## 后续观察

如果 Codex app-server 协议变化，需要重新验证。
