# 回复没有在 App 上屏

## 现象

桌宠卡片的回复操作看起来已经提交，但消息没有出现在 provider app 里。

## 需要收集的证据

- Provider、event id、状态和 session id。
- 前端 capability 是否对该事件暴露了回复。
- `send_activity_reply` 的后端结果。
- 可用时的 provider 专属日志或 app-server stderr。
- 人工确认 provider UI 是否显示了消息。

## 排查步骤

1. 确认事件状态是 `done` 或 `failed`。
2. 确认事件有非空 session id。
3. 确认 provider 是 Codex。Qoder 现有会话回复目前是故意不支持。
4. 对 Codex，检查 `src-tauri/src/agent/codex_app_server.rs` 行为和 app-server 启动路径。
5. 确认回复路径没有与审批处理路径混淆。

## 修复后验证

- `cargo test --manifest-path src-tauri/Cargo.toml activity_actions_tests agent_control_tests`
- `npx vitest run frontend/lib/activity.test.ts`
- 人工检查 Codex app UI，确认发送文本出现在对应 thread 中。
