# 回复没有在 App 上屏

> 当前状态（2026-08-30）：先区分 remote App Server 与 Desktop companion。remote 回复走独立 App Server；桌宠回复只走 Desktop IPC owner。两路请求、状态和错误不得互相替代。

## 现象

桌宠卡片的回复操作看起来已经提交，但消息没有出现在 provider app 里。

## 需要收集的证据

- Provider、event id、状态和 session id。
- 前端 capability 是否对该事件暴露了回复。
- Runtime Gateway Provider 是否实际声明 `turn.send`，以及发送请求的标准错误。
- 可用时的 provider 专属安全诊断；不要收集完整原生 payload。
- 人工确认 provider UI 是否显示了消息。

## 排查步骤

1. 先确认正在排查的构建版本和 Provider capability，不要仅凭卡片状态推断支持回复。
2. 若来自桌宠，确认请求使用 companion Tauri command，provider extension 是 `codepet.codex-desktop` 且 source 是 `codex-desktop-private-ipc`。会话必须已 bootstrap 且 owner 仍有效；活动 turn 走 steer，无活动 turn 走 start。Provider route 或同名 remote thread 不参与判断。
3. Qoder 现有会话回复仍是故意不支持；其他 Provider 只有在 Runtime Gateway 明确声明 `turn.send` 时才继续排查。
4. 若来自远程控制，确认请求使用 `runtime_gateway_request` 并命中 App Server Provider；若来自桌宠，确认使用 `codex_desktop_companion_request`。标准请求必须包含正确 conversation identity，不得绕过 capability 或跨 channel 重试。
5. 确认回复路径没有与审批或等待输入路径混淆；等待状态不等于可以发送普通回复。
6. App Server unavailable 只排查 executable、initialize、RPC/timeout 和 remote Provider；Desktop IPC unavailable 只排查 socket、owner/revision 和私有协议。不要为了让一路成功而切换到另一路。

## 修复后验证

- 运行当前 Provider capability 和前端交互能力测试。
- 当前 Codex 验证重点是 start/steer 选择、owner/revision/handler 校验，以及超时或重连后不重放。
- 人工检查消息是否由目标 owner 明确接受，并等待后续权威 snapshot/patch 上屏；Gateway ack 不代表最终状态。
