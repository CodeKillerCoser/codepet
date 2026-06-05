# Codex App Server 回复路径

## 当前用途

Codex 是当前已验证的主要远程回复 provider。`src-tauri/src/agent/actions.rs` 会把符合条件的 Codex 回复路由到 `src-tauri/src/agent/codex_app_server.rs`。

Codex 激活也由同一个 provider driver 处理，但它使用 `codex://threads/<thread-id>` deeplink，而不是 app-server RPC。

## 能力边界

前端只在以下条件同时满足时暴露 Codex 回复：

- 事件状态是 `done` 或 `failed`。
- 事件包含非空 `sessionId`。

这与后端的 `is_replyable_event()` 和 `has_session_id()` 保持一致。

## 已知能力

此前本机探测已验证：Codex app-server 可以把消息发送到现有 Codex app thread，并在 app UI 上屏。同一路径没有验证窗口激活或打开指定 thread 的 RPC，因此激活使用独立 thread deeplink。

## 验证

- 单元测试：`src-tauri/tests/activity_actions_tests.rs`。
- 前端能力测试：`frontend/lib/activity.test.ts`。
- 修改 JSON-RPC 方法序列或 thread deeplink 格式时，需要人工验证。
