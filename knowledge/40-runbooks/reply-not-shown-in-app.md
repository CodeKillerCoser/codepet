# 回复没有在 App 上屏

> 当前状态（2026-08-29）：Codex Desktop IPC Provider 只对由 follower 公告或显式已知、完成 bootstrap 且 owner 仍有效的会话支持 start/steer/interrupt 与 command/file 二元审批；本 runbook 只排查其中的 start/steer 回复。旧独立 App Server 回复链路已从当前源码删除，只能作为历史版本背景；当前排障不应寻找或恢复该源码。

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
2. 当前 Codex Desktop Provider 只应在会话已 bootstrap 且 owner 仍有效时受理 `turn.send`；活动 turn 走 steer，无活动 turn 走 start。按钮缺失先检查 capability 与当前权威状态。
3. Qoder 现有会话回复仍是故意不支持；其他 Provider 只有在 Runtime Gateway 明确声明 `turn.send` 时才继续排查。
4. 对确实声明回复能力的 Provider，确认标准请求包含正确 conversation identity，并检查 Gateway 返回的明确成功或错误。不得绕过 capability 直接调用 provider。
5. 确认回复路径没有与审批或等待输入路径混淆；等待状态不等于可以发送普通回复。
6. 仅排查旧发布包或历史提交时，使用该版本自己的源码和知识文档确认当时的实现。当前分支不再保留独立 App Server 文件、启动命令或 CLI fallback，不要按历史错误文案修改当前 runtime 设置。

## 修复后验证

- 运行当前 Provider capability 和前端交互能力测试。
- 当前 Codex 验证重点是 start/steer 选择、owner/revision/handler 校验，以及超时或重连后不重放。
- 人工检查消息是否由目标 owner 明确接受，并等待后续权威 snapshot/patch 上屏；Gateway ack 不代表最终状态。
