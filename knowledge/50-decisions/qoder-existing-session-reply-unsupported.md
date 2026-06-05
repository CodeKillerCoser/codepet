# 暂不支持 Qoder 现有会话回复

## 背景

Qoder remote-control 支持把本机注册为远程环境，但 Code Pet 需要向现有本地 Qoder session 发送消息。

## 决策

在稳定、已验证的 API 出现之前，不在桌宠 UI 中暴露 Qoder 现有会话回复。

## 备选方案

- 把 Qoder remote-control daemon 当作本地聊天 API 使用：拒绝，因为观察到的形态是通过 broker 注册环境。
- 使用本地 MCP-like 端口 `127.0.0.1:52345`：拒绝，因为探测只发现 VM 信息工具，没有 chat/session send API。
- 使用聚焦和粘贴：拒绝，因为对 provider 专属 remote-control 功能来说不可靠。

## 影响范围

后端 `QoderDriver` 对回复返回 unsupported。前端将 Qoder 的 `canReply` 设为 false。

## 后续观察

启用此能力前，需要研究官方 qoder.com broker 或 remote-control API。
