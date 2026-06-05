# Qoder Remote Control 边界

## 当前状态

Qoder 具备 remote-control daemon 方向，但 Code Pet 当前没有已验证的本地 API 可以向现有本地 Qoder session 发送消息。

## 已实现边界

- 后端 `src-tauri/src/agent/actions.rs` 中的 `QoderDriver` 返回 `ReplyStrategy::Unsupported`。
- 前端 `frontend/lib/agentInteractions.ts` 中的 `qoderInteraction` 将 `canReply` 设为 false。
- 当任务等待审批时，Qoder 审批仍然使用 collector wait 路径。

## 证据

此前探测发现，Qoder remote-control 是通过云端 broker 把本机注册为远程环境。本地 `127.0.0.1:52345` 的 MCP-like 端点没有暴露 chat/session send API。

## 规则

在没有稳定官方路径或已验证 broker/API 可以发送消息并让消息出现在 Qoder UI 前，不要暴露 Qoder 现有会话回复。
