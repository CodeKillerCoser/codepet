# Codex Remote App Server Provider

> 当前状态（2026-08-30）：独立二进制 `codepet-provider-codex` 按 Host manifest instance 持有长期 Codex App Server session，并通过 Provider Protocol v1 服务远程 Gateway。它与本机 Codex Desktop companion 私有 IPC 完全分离，产生的数据不进入桌宠 activity store。

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

Codex Provider v1 广告并实现：

- `conversation.list/get/create`
- `turn.start/steer/interrupt`
- `approval.resolve`
- App Server thread/turn/item/approval 通知到 Standard Protocol event 的映射

Gateway v1 把 `turn.send` 分发到 Provider `turn.start` 或 `turn.steer`。`RuntimeGatewayState` 只把既有 compat-v0 调用面适配到 `ProviderGatewayService`，不注册 adapter、不持有 App Server session，也不产生第二份事件。Tauri 远程边界继续使用 `runtime_gateway_request`、`runtime_gateway_replay` 与 `runtime-gateway-event`；桌宠不订阅这些接口。

## 实现路径

Provider instance 启动并 initialize 官方 stdio JSON-RPC 子进程，reader/writer 长期运行。request id 映射允许 response 乱序返回；notification 经 mapper 生成 Provider v1 conversation、turn、delta 与 approval event。Provider 复用同一 session 完成 list/read/start/steer/interrupt/approval，不经过 Tauri 或 compat DTO。

协议 DTO 以本机 ChatGPT 内置 `codex-cli 0.151.0-alpha.7.1` 的 `app-server generate-json-schema` 与 `generate-ts` 输出为证据。该版本的 `ThreadStartResponse`、`ThreadResumeResponse` 都返回带 `type` discriminator 的 `SandboxPolicy` 对象；adapter 同时接受旧版字符串形状，但权限映射以结构化 `readOnly`、`workspaceWrite`、`dangerFullAccess` 为主。该版本实机对不存在 thread 的 `thread/resume` 返回 JSON-RPC `-32600` 与 `no rollout found for thread id ...`，这组 code/message 是已明确拒绝且未接管的证据。

`thread/list`、`thread/read` 只返回历史目录/快照，不代表 thread 已加载进当前 session。每个 `CodexAppServerSession` 独立维护 `Unknown → Resuming → Loaded` 状态：并发成功时等待者共享同一 Loaded 证据；resume 失败不缓存，等待者被唤醒后重新竞争并发起下一次 resume。`thread/start`、成功 resume 或当前 session 的 thread/turn/approval 通知构成 loaded 证据。新 session 不继承这张表，因此必须重新 resume。

App Server client 内部保留 `Success`、`NotSent`、`ExplicitRpcReject`、`SentOutcomeUnknown` 四种请求证据，用于正确维护同一 session 的 loaded-thread 状态；调用 writer 前的 Shutdown/锁失败属于 NotSent，writer 已被调用后的 I/O、断线、timeout 或 pending Shutdown 属于 SentOutcomeUnknown。当前 Provider Protocol 的 `ProtocolError` 没有把这组交付证据暴露给 Host。Tauri compat 边界在 Gateway create/turn 周围维护 transient source fence，成功 response/event 标记 remote；调用开始后的错误保守按歧义处理，避免 Desktop companion 误投影 remote thread。approval 使用自身 routed conversation 标记来源并回写原 Provider instance。

官方 permissions request 的 response 不是二元 decision，当前 Provider 无法无损表达，因此不发布可操作 Approval，并使用原 request id 返回 JSON-RPC `-32601`。tool user input、MCP elicitation 和其他未知 server request 同样明确拒绝，不进入 pending approval，也不伪造空 grant 或成功。

runtime resolver 提供已验证 executable。应用启动、设置 path、清除 path 或刷新 runtime 时，Host 只更新 Codex manifest instance 的 `appServerExecutable` 并显式重启该插件；Desktop companion 的 socket connection、generation、owner 与 revision 不受影响。App Server unavailable 只改变 remote Provider 状态，不清空 companion projection。

compat `conversation.create` 在 Gateway 请求前建立带 epoch 的 source guard。response 或 Provider event 给出 thread id 时先写入共享 `CodexThreadScope`；scope 只协调 provenance、quarantine 和本地动作 permit，Desktop adapter 仍有独立协议、owner 状态和生命周期。

## 涉及模块

- `crates/providers/codepet-provider-codex/src/{client,protocol,mapper,provider}.rs`：App Server 子进程、官方 DTO、Provider v1 映射与实例路由。
- `crates/providers/codepet-provider-codex/src/main.rs`：生成 SDK codec/dispatcher 驱动的 stdio JSON-lines 主循环。
- `crates/codepet-host`：manifest、进程、实例、Gateway route 与事件 replay。
- `src-tauri/src/runtime_gateway/provider_host_compat.rs`：既有远程调用面到 Gateway v1 的薄适配。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs`、`src-tauri/src/lib.rs`：resolver 注入、插件 refresh 与 remote event bridge。

## 风险与验证

- App Server JSON-RPC 漂移：fake peer 覆盖 initialize、乱序 response、list/read/create/start/steer/interrupt/approval 和通知；升级 CLI 后重跑。
- response schema 漂移：用当前 binary 重新生成 JSON Schema/TS binding，核对 start/resume `SandboxPolicy` 与 permissions request/response；真实对象 fixture 必须保持可反序列化。
- 历史 thread 生命周期：list/read 后首次运行时动作必须先 resume；测试覆盖真实 `-32600/no rollout`、写前 Shutdown、写后断线、并发失败 waiter 重试、成功缓存和新 session 重新 resume。
- server request 悬挂：permissions 与未知 method 必须得到 `-32601`；测试同时断言无 Approval event、pending 为空且 session 继续运行。
- 子进程退出：remote Provider 变为 unavailable；当前没有持续 supervisor 自动拉起，需显式 refresh 或重启，不能借 companion 掩盖。
- 生命周期串扰：测试分别将 remote/companion 置为 unavailable，并断言另一条 Provider 与状态仍可用。
- 事件串流：Host 从真实 manifest binary 收到 Provider event，断言 remote replay/event 有数据而 companion replay、Pet channel 与 activity store 仍为空。
- remote thread 经 Desktop 再出现：create guard 测试覆盖 response/notification 标记、保守歧义结果和 epoch 竞态；companion publication/snapshot/action 和前端 tombstone 测试断言 remote id 被排除且旧事件不能恢复。
- 网络能力误判：当前验证只证明 Provider/runtime 和进程内 transport，不宣称真实远程网络已上线。

## 测试计划

- App Server fake peer 完整请求/通知闭环。
- list → resume → send、同 session 成功缓存、失败 waiter 重试与新 session 重新 resume。
- permissions/未知 server request 的 `-32601` response 与 capability/event 一致性。
- remote `conversation.list/create` 明确路由 Provider Gateway；companion 不广告或实现这两项能力。
- runtime executable refresh 只更新 Host instance setting 并重启 Codex 插件。
- remote event 不进入 companion Tauri replay/event，桌宠源码不引用 remote client。
- 相关 Rust tests、全量 Vitest 和前端 production build。

## 知识沉淀

- 双链路决策见 `../../50-decisions/codex-remote-and-desktop-companion-dual-channel.md`。
- 故障排查见 `../../40-runbooks/codex-app-server-unavailable.md`。
- channel 约束见 `../../60-rules/codex-provider-channel-isolation.md`。
- 完整边界、协议矩阵和开发安装见 `../../10-architecture/codex-provider-plugin-runtime.md`。

## 未知项

- App Server 的自动 supervisor/reconciliation 策略尚未实现。
- 真实远程网络 transport 尚未实现。
- 跨应用重启的 remote thread provenance 尚未持久化。
