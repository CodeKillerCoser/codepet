# 架构决策

决策文档记录未来 Agent 不应随意推翻的长期技术选择。

## 当前决策

- `codex-remote-and-desktop-companion-dual-channel.md`
- `language-neutral-protocol-idl-and-sdk-boundary.md`
- `shared-agent-domain-between-provider-and-gateway.md`
- `qoder-existing-session-reply-unsupported.md`
- `semantic-file-tree-as-knowledge-index.md`

## 已取代决策

- `codex-desktop-ipc-as-provider-source.md`：已由 Codex 双链路决策取代；其中 Desktop IPC fail-closed 约束继续适用于 companion。
- `codex-app-server-as-primary-reply-path.md`：旧 `PetEvent` 一次性回复路径已取代；当前 remote App Server 由双链路决策约束。

## 规则

决策文档要短而明确。除了最终选择，还要写备选方案和后果。
