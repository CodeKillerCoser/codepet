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
3. Codex 旧活动源与回复 capability 当前已停用；Qoder 现有会话回复也仍是故意不支持。只有新 Runtime Gateway 接管后，才应重新排查 Codex 回复链路。
4. 仅排查旧版本或历史提交时，再检查 `src-tauri/src/agent/codex_app_server.rs` 行为和 app-server 启动路径。
   - 如果日志出现 `failed to start codex app-server: program not found`，优先检查 Windows 下 Codex binary 查找是否覆盖 `%LOCALAPPDATA%\OpenAI\Codex\bin\<hash>\codex.exe`，或临时设置 `CODE_PET_CODEX_BIN`。
5. 确认回复路径没有与审批处理路径混淆。

## 修复后验证

- `cargo test --manifest-path src-tauri/Cargo.toml activity_actions_tests agent_control_tests`
- `npx vitest run frontend/lib/activity.test.ts`
- 旧版本回归时才人工检查 Codex app UI；当前版本应验证 Codex 卡片不暴露回复入口。
