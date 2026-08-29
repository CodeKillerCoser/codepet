# Codex Desktop IPC 不可用

## 现象

- 桌宠显示 Codex Provider unavailable、disconnected 或 error。
- Codex Desktop 中有任务，但 Code Pet 没有显示任务或停止更新。
- 日志出现 socket 安全检查、initialize、路由、frame、revision 或重连相关错误。

阶段一没有 App Server、Hook、transcript 或文件监听回退。Codex CLI 能被设置页检测到，也不代表 Desktop IPC Provider 可用。

## 需要收集的证据

- Codex Desktop 是否正在同一用户会话中运行。
- `~/.codex/ipc/ipc.sock` 是否存在、是否为当前有效用户所有的 Unix socket、权限是否为 `0600`。
- Provider 当前状态及公开的安全诊断原因。
- 最近一次连接、initialize、断线和重连日志；不要收集完整私有 payload。
- 当前 client identity、消息是定向还是广播，以及定向目标是否匹配；日志中应使用截断或脱敏标识。
- 受影响 thread id 是否已知，以及是否完成 owner discovery、following、完整历史和 snapshot bootstrap。
- 当前 revision、收到的 revision，以及是否处于等待完整 snapshot 状态。
- Codex Desktop 版本，以及 adapter 当前验证的 Owner/Follower/状态广播版本。

## 排查步骤

1. 确认 Codex Desktop 正在运行。Desktop 未运行时 Provider unavailable 是预期行为；不要因此启动 `codex app-server`。
2. 检查 `~/.codex/ipc/ipc.sock`：
   - 不存在：记录 socket missing，启动或恢复 Desktop 后等待有界重连。
   - 不是 Unix socket：停止排查并按不安全目标处理，不要连接、改权限或删除未知文件。
   - owner 不是当前有效用户：Provider 应 fail closed，不要连接或自动改 owner。
   - 权限不是 `0600`：Provider 应 fail closed。先确认文件来源，不要让 Code Pet 自动 `chmod`。
3. 检查 initialize 是否成功并分配了本次连接唯一 client identity。握手前失败通常属于连接或版本问题；握手后失败再检查路由和 follower 生命周期。
4. 检查消息路由：发给其他 client 的消息应被忽略，广播才进入广播分发。若当前 client 收不到目标消息，记录目标不匹配，不要关闭路由过滤来“修复”。
5. 对已知 thread id 检查 bootstrap 顺序：owner discovery → `following=true` → 初始 snapshot → 完整历史返回 revision → 对应 revision 的 snapshot。任何一步缺失都不能把 thread 标记为已同步。
6. 检查 revision：
   - 重复或旧 patch 可以安全忽略。
   - 前向缺口必须让旧基线失效；当前实现断开该 IPC generation，再通过有界重连和完整 bootstrap 请求新 snapshot。
   - 等待期间任务不应继续应用 patch；如果 UI 仍变化，优先排查状态机错误。
7. 检查断线复位：pending request 应显式失败，revision、following 和旧 owner route 应清空。重连应使用有界退避重新 initialize 和 bootstrap。
8. 若 Code Pet 是晚加入且没有任何已知 thread id，当前协议没有任务目录，无法保证从零枚举。其他 follower 可能机会性公告 `following=true`，但当前联调已观察到空 known-set 客户端收不到现有任务公告的合法情况。这是已知限制，不要通过 transcript、audit、Hook 或文件监听绕过。

## 结果判断

- socket 缺失或连接拒绝：Provider unavailable，可重试；Desktop 恢复后应重新连接。
- socket 类型或权限不安全：Provider unavailable，不自动修复文件。
- 协议或版本不兼容：Provider error/unavailable，停止解析并记录 Desktop 版本。
- revision 缺口：thread 等待 snapshot，不发布由缺口 patch 推导的状态。
- 无已知 thread id：报告发现范围限制，不宣称 Desktop 没有任务。
- 回复或审批按钮缺失：阶段一设计如此，不是连接故障。

## 恢复后验证

- Provider 状态按 connecting → ready 恢复，并保留唯一的新 client identity。
- 已知 thread 重新完成完整 bootstrap，revision 从新 snapshot 建立，不沿用断线前基线。
- 运行、等待审批或输入、完成、失败和中断状态可以继续更新。
- 多客户端定向消息不串流。
- 生产日志没有私有完整 JSON、命令内容、diff 或其他敏感 payload。
- 代码和日志均没有启动独立 App Server、恢复 Hook 或扫描 transcript 的迹象。

## 仍需升级处理的情况

- 当前 Codex Desktop 版本改变了 Owner/Follower 或状态广播版本。
- 同一 revision 出现互相冲突的 snapshot/patch。
- 完整历史与 snapshot 长期无法在同一 revision 收敛。
- socket 安全检查通过，但不同 client 仍收到彼此的定向状态。

这些情况属于私有协议兼容或路由正确性问题，不能通过扩大 capability 或启用写操作缓解。
