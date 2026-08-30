# Codex Desktop Companion 私有 IPC

> 当前状态（2026-08-30）：桌宠通过本机 `~/.codex/ipc/ipc.sock` 跟随 Codex Desktop Owner/Follower。该通道只负责本地任务投影和面向 Desktop owner 的动作，不是远程 Provider，也不提供任务目录或创建能力。

## 背景

Code Pet 需要展示并安全控制 Codex Desktop 当前活跃任务。Desktop 私有 IPC 能提供 owner、revision、活动 turn 和 pending request 等实时证据，因此适合做本地 companion；它没有公开兼容性承诺，也没有可验证的 thread list/create 方法。

独立 Codex App Server 同时作为另一条远程 runtime 存在。两者不是 fallback 关系，不共享 session、event bus、replay sequence、owner 状态或 unavailable 状态。Hook、transcript、audit 扫描和文件监听也不补充 companion 数据。

## 目标

- 安全连接当前用户的 Desktop IPC，并把连接失败表达为 companion unavailable。
- 对已知或连接期间被公告的 thread 完成 owner discovery、following、历史和 snapshot bootstrap。
- 严格按 revision 投影运行、等待、完成、失败、中断和可验证审批状态。
- 将 start/steer、interrupt 和 command/file 二元审批精确定向当前 Desktop owner。
- 通过 companion 专用 snapshot/replay/event 驱动桌宠；不读取、排除或同步 remote Provider thread。
- 把私有 DTO、方法版本、路由与重连逻辑限制在 desktop adapter 内。

## 非目标

- 不提供或伪造 `conversation.list/create`。
- 不在 IPC unavailable 时改用 App Server、CLI、Hook、audit、transcript 或文件扫描。
- 不把 permissions approval、MCP elicitation、user input 等无法无损表达的请求伪装成二元审批。
- 不让远程 App Server snapshot/event 进入桌宠 activity store。
- 不承诺在完全未知 thread id 时枚举 Desktop 的全部任务。

## 已验证事实

- IPC 使用当前用户所有、权限为 `0600` 的 Unix socket；frame 为 4 字节 little-endian 长度加 JSON，最大 256 MiB。
- 每次连接使用唯一 `clientId`；定向消息必须按路由过滤，广播消息才进入 follower 分发。
- 当前 Owner/Follower 协议族为 v1，状态广播为 v11；start、steer、interrupt 分别使用已验证的 v2、v1、v4，command/file approval decision 使用 v1。
- 写方法没有 `expectedRevision` wire 字段。adapter 通过本地冻结并复核 generation、owner、snapshot revision、turn/request 和 response handler 失败关闭，不能把它描述成 wire 级 CAS。
- Owner ack 只证明请求被协议接受，最终 turn/approval 状态仍来自后续权威 snapshot/patch。超时或断线时结果未知，非幂等请求不跨重连重放。
- patch 只有 revision 连续时才能应用；缺口会进入等待完整 snapshot 的状态，期间不发布推断状态。
- IPC 没有任务目录。`following=true` 公告是机会性发现，不是完整 late-join list。
- Desktop companion 与 remote runtime 使用不同 `GatewayEventBus`、`LocalTransport` 和 Tauri channel。remote event 无法进入 companion replay；桌宠前端也只监听 companion event。

## 实现路径

### 连接与状态

`transport.rs` 在连接前验证 socket 类型、owner 和权限，完成 frame codec、initialize、请求关联与关闭。`client.rs` 管理 owner discovery、following、bootstrap、写请求定向和重连边界；`state.rs` 管理 snapshot/patch 与 revision 状态机。

完整 snapshot 建立基线。重复或旧 patch 被忽略，前向缺口触发重同步；断线会使 revision、following、owner route 和 pending request 全部失效，新的 generation 必须重新 bootstrap。

### 本地 companion 投影

adapter 只广告 `conversation.get`、`turn.send`、`turn.interrupt` 和 `approval.resolve`。桌宠初始化调用 `codex_desktop_companion_snapshot` 获得已 bootstrap 的本地投影，实时变化和 replay 分别使用 `codex-desktop-companion-event` 与 `codex_desktop_companion_replay`。这套 snapshot 不是 Standard Protocol 的 thread directory。

运行中的 `turn.send` 使用 steer，空闲 thread 使用 start；interrupt 必须匹配当前活动 turn。command/file pending request 可映射为只含 `approve/deny` 的 Approval，其他请求只保留 waiting/unsupported 诊断。

### 来源隔离

Desktop Companion 不接收 Provider/Gateway event、route 或 provenance。旧 `CodexThreadScope`、Desktop 排除入口、Tauri exclusion producer 与 Pet tombstone 已删除。remote 与 Desktop 如果报告相同 native thread id，会在各自独立 channel 中存在；本阶段不建立 quarantine、tombstone、动作 fence 或跨链路去重。

真正的隔离点是 wiring：Provider 只发布 `runtime-gateway-event`；companion 只从 Desktop IPC 形成自己的 snapshot/replay/event。Pet UI 只消费 companion channel，因此无需依赖“先污染、再排除”的补偿逻辑。

## 涉及模块

- `src-tauri/src/agent/codex_desktop_ipc/`：私有 transport、协议、client、状态机、mapper 和 companion adapter。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs`：独立 `CodexDesktopCompanionState`、snapshot/replay/request 和 event bridge。
- `frontend/lib/codexDesktopCompanion.ts`：桌宠唯一使用的 companion Tauri client。
- `frontend/PetApp.svelte`：只订阅 companion channel，并将回复、停止和审批发往 companion owner。
- `frontend/lib/agentInteractions.ts`：动作除 capability 外还校验 `codepet.codex-desktop` namespace 和 `codex-desktop-private-ipc` source。

## 风险与验证

- 私有协议版本漂移：升级 Desktop 后重跑 frame、bootstrap、revision、owner/handler 和动作测试；不兼容即 unavailable。
- 晚加入时没有 thread id：UI 只描述已观察范围；不得扫描文件冒充目录。
- remote/Desktop 同名 thread：两条链路暂时独立展示和操作，不保证去重；任何未来合并都必须基于官方 origin metadata，并作为独立协议阶段设计。
- Owner ack 被误当成终态：Provider 测试验证 ack 不提前改 turn/approval，审批 resolved 需要 ack 与权威 request removal 两项证据。
- remote event 污染桌宠：双 transport 测试发布 remote conversation/turn/approval 后断言 companion replay 为空，前端静态测试断言不监听 remote event。

## 测试计划

- Rust adapter：socket/frame、路由、bootstrap、revision 缺口、重连、start/steer/interrupt、审批和 owner/revision/request/handler fail-closed。
- Rust bridge：remote 与 companion lifecycle 独立；remote event 不进入 companion transport/state 或 exclusion event；Desktop snapshot 仍能产生任务和审批。
- 前端：只调用 companion snapshot/replay/request、只监听 companion event，并对动作 source marker 失败关闭。
- 构建：相关 Rust tests、全量 Vitest 与前端 production build。

## 知识沉淀

- 双链路决策见 `../../50-decisions/codex-remote-and-desktop-companion-dual-channel.md`。
- IPC 故障见 `../../40-runbooks/codex-desktop-ipc-unavailable.md`。
- 安全边界见 `../../60-rules/codex-desktop-ipc-fail-closed.md` 与 `../../60-rules/codex-provider-channel-isolation.md`。

## 未知项

- 私有 IPC 仍没有完整任务目录或安全 create 方法。
- 支持的 Desktop 版本范围需要随发布验证。
- remote/Desktop 同名资源的产品呈现策略尚未定义；当前不做跨链路协调。
- 非二元审批和其他私有写操作仍无法由 v0 Standard Protocol 无损表达。
