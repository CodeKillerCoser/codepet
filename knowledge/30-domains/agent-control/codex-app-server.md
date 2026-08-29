# Codex Remote App Server Provider

> 当前状态（2026-08-30）：Code Pet 独立持有长期 `codex app-server --listen stdio://` session，作为远程 Provider。它与本机 Codex Desktop companion 私有 IPC 完全分离，产生的数据不进入桌宠 activity store。

## 背景

远程控制需要项目/会话目录、创建、读取、继续、停止和审批。Desktop 私有 IPC 没有 thread directory 或 create，因此不能承担这组能力。实机已确认独立 App Server 创建并完成的 thread 会被当前打开的 Desktop 自动加载，但这项产品协同行为不等于两套协议共用 owner 或事件流。

## 目标

- 复用现有官方 App Server JSON-RPC client、mapper 和 Provider，实现完整 remote 能力。
- 持有一个长期 stdio session，关联并发 request、approval 和 notification。
- 将 remote unavailable、事件、replay 和 runtime executable 生命周期限制在 remote Gateway。
- 在创建成功后记录 remote thread provenance，阻止它被 Desktop companion 再投影到桌宠。

## 非目标

- 不把 App Server 当作 Desktop IPC 的 fallback 或 Desktop 当前 owner 的权威状态。
- 不把 remote snapshot/event 发往桌宠 activity store。
- 不恢复旧 `PetEvent` driver、全局 reply session、Hook、audit、transcript 或文件监听数据源。
- 本阶段不实现 WebSocket/P2P/relay 等真实手机网络 transport；仓库仍使用进程内 `LocalTransport`。

## 当前能力

remote Provider 广告并实现：

- `conversation.list/get/create`
- `turn.send`：没有活动 turn 时 start，有活动 turn 时 steer
- `turn.interrupt`
- `approval.resolve`
- App Server thread/turn/item/approval 通知到 Standard Protocol event 的映射

`RuntimeGatewayState` 只注册 App Server adapter，并持有独立 registry、event bus、sequence/replay 和 local transport。Tauri 远程边界继续使用 `runtime_gateway_request`、`runtime_gateway_replay` 与 `runtime-gateway-event`；桌宠不订阅这些接口。

## 实现路径

`CodexAppServerClient` 启动并 initialize 官方 stdio JSON-RPC 子进程，reader/writer 长期运行。request id 映射允许 response 乱序返回；notification 经 mapper 生成标准 conversation、turn、item 与 approval event。Provider 复用同一 session 完成 list/read/start/steer/interrupt/approval，不重新手写协议。

runtime resolver 提供已验证 executable。应用启动、设置 path、清除 path 或刷新 runtime 时，只刷新 remote App Server adapter；Desktop companion 的 socket connection、generation、owner 与 revision 不受影响。App Server unavailable 只改变 remote Provider 状态，不清空 companion projection。

`conversation.create` 在发出请求前建立带 epoch 的 source guard。response 或 notification 给出 thread id 时先写入共享 `CodexThreadScope`；明确未派发的错误释放已证明为本地的候选，session 已建立后的请求错误按歧义结果保守排除同 epoch 候选。scope 只协调 provenance、quarantine 和本地动作 permit，Desktop adapter 仍有独立协议、owner 状态和生命周期。

## 涉及模块

- `src-tauri/src/agent/codex_app_server/client.rs`：子进程、initialize、typed request map、reader/writer 和通知。
- `src-tauri/src/agent/codex_app_server/protocol.rs`：官方 JSON-RPC DTO。
- `src-tauri/src/agent/codex_app_server/mapper.rs`：App Server 与 Standard Protocol 的双向映射。
- `src-tauri/src/agent/codex_app_server/provider.rs`：remote Provider capability、请求路由和 provenance 标记。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs`：remote state、transport 与 event bridge。
- `src-tauri/src/agent/runtime.rs`、`src-tauri/src/lib.rs`：executable 解析和 remote adapter 刷新。

## 风险与验证

- App Server JSON-RPC 漂移：fake peer 覆盖 initialize、乱序 response、list/read/create/start/steer/interrupt/approval 和通知；升级 CLI 后重跑。
- 子进程退出：remote Provider 变为 unavailable；当前没有持续 supervisor 自动拉起，需显式 refresh 或重启，不能借 companion 掩盖。
- 生命周期串扰：测试分别将 remote/companion 置为 unavailable，并断言另一条 Provider 与状态仍可用。
- 事件串流：bridge 测试向 remote sink 发布 conversation/turn/approval，断言 companion replay 仍为空。
- remote thread 经 Desktop 再出现：create guard 测试覆盖 response/notification 标记、明确未派发与歧义结果、epoch 竞态；companion publication/snapshot/action 和前端 tombstone 测试断言 remote id 被排除且旧事件不能恢复。
- 网络能力误判：当前验证只证明 Provider/runtime 和进程内 transport，不宣称真实远程网络已上线。

## 测试计划

- App Server fake peer 完整请求/通知闭环。
- remote `conversation.list/create` 明确路由 App Server adapter；companion 不广告或实现这两项能力。
- runtime executable refresh 只替换 remote adapter。
- remote event 不进入 companion Tauri replay/event，桌宠源码不引用 remote client。
- 相关 Rust tests、全量 Vitest 和前端 production build。

## 知识沉淀

- 双链路决策见 `../../50-decisions/codex-remote-and-desktop-companion-dual-channel.md`。
- 故障排查见 `../../40-runbooks/codex-app-server-unavailable.md`。
- channel 约束见 `../../60-rules/codex-provider-channel-isolation.md`。

## 未知项

- App Server 的自动 supervisor/reconciliation 策略尚未实现。
- 真实远程网络 transport 尚未实现。
- 跨应用重启的 remote thread provenance 尚未持久化。
