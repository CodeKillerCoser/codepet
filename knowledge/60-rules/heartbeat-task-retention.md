# 心跳不控制任务生命周期，持续空闲才自动释放

## 规则

手机在线状态和心跳只表达连通性。无论手机在线还是离线，实例存在运行/排队任务、待授权/待输入、未完成请求或未知发送结果时都不能自动释放。只有适配器声明 `instance.releaseControl` 能力，Host 请求释放，并且全部活动结束后持续空闲达到独立阈值，才释放共享 App Server；SDK 无自主回收定时器，不能把 Codex 的会话锁适配策略套用到所有 Provider。

## 适用场景

Provider SDK heartbeat/activity/release_control/mux、Host 按需恢复、Codex App Server generation 与 Remote 重新交互。

## 反例

- 60 秒没有心跳就调用 instance.stop。
- 最后一部手机离开后立即停止空闲进程，不给连续空闲窗口。
- 手机一直在线就永久保留所有已加载会话。
- 回收后禁止任何正常启动，或相反，启动时批量 resume 全部旧会话。
- 心跳、历史轮询不停刷新空闲计时。
- start 尚未返回或发送结果未知，就认为不存在活动任务。
- 为解决 Codex 会话锁而默认每 300 秒停止 Claude/OpenCode，或改成循环重启它们。

## 推荐做法

Host 统一调度 releaseControl，但不得统一 stop/restart 全部 Provider。仅对启用、Ready 且声明能力的实例发请求，默认窗口 300 秒；每实例单飞，插件之间独立，错误后限频重试。SDK 用唯一活动投影和独占准入门复核连续空闲，适配器执行释放；当前只有 Codex 实现释放并启动干净 Server。Claude/OpenCode 不声明，误收到时返回 unsupported，无副作用。读请求执行中保护但不续期，业务操作重置计时；心跳不参与判断。

SDK heartbeat 不得调用 Provider 生命周期方法，也不再根据心跳携带的 instances 缺失推导停止；显式实例管理仍走业务命令。自动释放只能通过统一回收器，不能复用无条件取消活动任务的手动 release RPC。

已准入的释放不得随调用方取消响应而半途停止；shutdown/EOF 必须能取消回收并清理子进程，不能等待独占门造成死锁。回收响应不得用旧实例快照覆盖较新事件。明确停止后的实例不得由调度器复活；自动释放不能使用可取消活动任务的 conversation.releaseInteraction。

## 来源

2026-11-05 用户澄清：释放旧 App Server 会释放其积累的会话锁，重启不会自动重新占用旧会话；活跃会话保护与心跳无关。2026-11-08 明确空闲重启是 Codex 适配，不应作用于其他 Provider，见 [默认空闲回收误停其他 Provider](../40-runbooks/provider-idle-policy-opt-in.md)。参见 [实现架构](../10-architecture/interaction-runtime-lifecycle.md)。

## 验证方式

SDK 可控时钟验证跨会话活动、授权/输入、完整计时窗口、准入竞态、未知发送结果和不支持能力的 no-op；mux 验证取消响应后继续与 shutdown 中断。Codex binary 验证旧 PID 退出、新 PID 不批量 resume、离线任务与审批保留、重启失败/卡住清理。Host 验证单飞、错误限频、只回收 Codex 而 Claude/OpenCode 保持 Ready，且无手机时也能调度。独立 SDK 导出包含相同协议与准入模块。详见 [Host 调度架构](../10-architecture/host-provider-release-control.md)。真实桌面锁验证不能以 fixture 代替。
