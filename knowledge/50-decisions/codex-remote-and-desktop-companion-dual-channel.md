# Codex Remote 与 Desktop Companion 使用隔离双链路

## 背景

Code Pet 同时需要两类能力：远程端需要完整的项目/会话目录、创建、读取、继续、停止和审批；桌宠需要跟随当前 Codex Desktop owner 的实时任务，并把快捷回复、停止和审批精确定向到该 owner。

阶段一曾删除独立 `codex app-server --listen stdio://`，把 Desktop 私有 Owner/Follower IPC 注册成唯一 `codex` Provider；阶段二在这条 IPC 上完成了 generation、owner、revision、request、handler 和非幂等不重放等安全校验。该接线让桌宠能跟随 Desktop，但 IPC 没有任务目录或安全的 thread create 方法，因而远程 `conversation.list/create` 退化为缓存列表或 unsupported。

2026-08-30 的实机实验确认：Code Pet 独立 App Server 创建并完成的 thread 会被当前打开的 Codex Desktop 自动加载。这个证据说明独立 App Server 可以继续承担远程 runtime，但不证明它与 Desktop 私有 IPC 是同一协议实例，也不证明 App Server 能附着所有 Desktop-originated 实时审批。

## 决策

Codex 使用两个彼此隔离的运行时通道：

- `CodexRemote / AppServer`：进程外 `codepet-provider-codex` 持有长期官方 stdio JSON-RPC session，经 Provider Protocol v1、Plugin Manager 与 Provider Gateway 提供 `conversation.list/get/create`、turn start/steer/interrupt、`approval.resolve` 和通知事件。`RuntimeGatewayState` 只保留既有调用面的薄适配。Codex executable 检测与配置只驱动这条生命周期。
- `CodexDesktopCompanion / IPC`：Code Pet 作为本机 Desktop Owner/Follower 的 follower，通过 `CodexDesktopCompanionState`、专用 snapshot/replay/request 和专用 Tauri event 驱动桌宠。它不广告 `conversation.list/create`；初始状态只由 companion snapshot 返回已验证、已 bootstrap 的本地投影。

两条链路各自拥有 registry、event bus、event sequence/replay window、session 和 unavailable 状态。remote 使用 Provider Host/Gateway，companion 使用进程内 compat Gateway/LocalTransport；不得共享 provider slot、event sink、owner/revision/request 状态、thread provenance、projection 或重连策略。

Provider route 和 event 不写 `CodexThreadScope`，不调用 Desktop adapter，不发布 companion exclusion event，也不修改 Pet projection。remote 与 Desktop 即使报告相同 native thread id，也分别保留在各自 channel；本阶段不建立 quarantine、tombstone、动作 fence、去重或同步。链路隔离依赖生产 wiring，而不是收到 Provider event 后再做来源推断。

Hook、audit、transcript 扫描和文件监听继续不得成为 Codex 桌宠数据源或任一 Codex Provider 的 fallback。

## 备选方案

- 在同一 Gateway 注册两个不同 provider id：实现较少，但同一 event bus、replay sequence 和 Tauri event 仍会让桌宠收到远程事件；任一路 unavailable 也容易污染另一条 UI 状态，因此不采用。
- Desktop IPC 失败时回退 App Server：会把同一个桌宠任务在两个 owner/runtime 之间切换，无法保证回复和审批目标，因此不采用。并行 remote runtime 不是 companion fallback。
- 继续 IPC-only：能保持 Desktop 对齐，但无法提供真实目录和创建能力，不能满足远程控制，因此不采用。
- 继续 AppServer-only：能提供远程能力，但不能以 owner/revision 语义精确控制 Desktop 当前任务，因此不采用。
- 恢复 Hook、audit 或 transcript 补齐任务：这些来源缺少权威 revision 和审批路由，只能推断，不采用。

## 取舍理由

- 删除前的 App Server session/mapper 已有 typed request map、长期 reader/writer、list/read/start、turn、approval 和通知测试，可以提取到独立 Provider，而无需在 Tauri 重写官方协议。
- Desktop IPC 已有 socket 安全检查、Owner/Follower bootstrap、revision 连续性和写动作 fail-closed 测试；将它收窄为 companion 可以保留这些正确实现。
- 两个独立 event bus 从 Rust transport 边界阻止远程 conversation/turn/approval 进入桌宠，强于 UI 隐藏或 provider id 过滤。
- companion 专用 snapshot 如实表达“已跟随、已 bootstrap 的本地投影”，不会把缓存伪装成 Desktop thread directory。
- executable 配置和 Desktop socket 状态分别只影响一条生命周期，能够准确表达局部 unavailable。

## 影响范围

- `crates/providers/codepet-provider-codex/`：承载 remote session、mapper、Provider v1 实现和 stdio 主循环；不依赖 Tauri、Host 或 Pet SDK。
- `src-tauri/src/agent/codex_app_server/`：删除，不保留进程内直连或 fallback。
- `src-tauri/src/agent/codex_desktop_ipc/`：保留私有协议与安全校验，移除 `conversation.list` capability，并提供只来自 Desktop IPC 的 companion snapshot。
- `src-tauri/src/runtime_gateway/provider_host_compat.rs`：把 remote v0 command/replay/event 薄适配到 Provider Gateway；不启动 App Server。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs`：组合 Provider Host/resolver，并保持 remote 与 companion 两套 command、replay 和 event channel。
- `frontend/PetApp.svelte`：只订阅/调用 companion channel；remote `runtime-gateway-event` 不进入 activity store。
- Agent runtime 设置：set/clear/refresh 只更新 Host Codex instance setting 并重启该插件，不重启 Desktop IPC。
- 运维：App Server 与 Desktop IPC 使用独立 unavailable runbook；排查时不能用另一路状态替代本路证据。

回归验证包括：真实 fixture App Server 子进程下的 list/create/start/steer/interrupt/approval/notification 闭环；Host 从 manifest 启动 Provider binary 的纵向 RPC；remote conversation/turn/approval 只进入 remote event/replay，而 companion/Pet/activity 和 exclusion event 为空；Desktop snapshot 仍驱动任务和审批；前端只调用 companion command/event。

## 后续观察

- 当前仓库仍只有进程内 `LocalTransport`，真实手机 WebSocket/P2P/relay transport 尚未实现；本决策恢复的是 remote Provider/runtime 边界，不宣称远程网络已完成。
- remote 与 Desktop 的同名 thread 暂时独立显示在各自客户端；跨链路去重或 origin metadata 不属于本阶段。
- App Server 进程退出目前变为 remote Provider unavailable，自动 supervisor/reconciliation 仍是后续增强；不能借 Desktop IPC 掩盖故障。
- 每次 Codex Desktop 升级仍需复核私有 IPC 版本、owner/revision/write handler；每次 Codex CLI 升级需复核 App Server JSON-RPC mapper。
