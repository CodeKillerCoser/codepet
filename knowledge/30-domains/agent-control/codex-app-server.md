# Codex Remote App Server Provider

> 当前状态（2026-08-31）：独立二进制 `codepet-provider-codex` 按 Host manifest instance 持有长期 Codex App Server session，并通过 Provider Protocol v1 服务远程 Gateway。它与本机 Codex Desktop companion 私有 IPC 完全分离，产生的数据不进入桌宠 activity store。

## 背景

远程控制需要项目/会话目录、创建、读取、继续、停止和审批。Desktop 私有 IPC 没有 thread directory 或 create，因此不能承担这组能力。实机已确认独立 App Server 创建并完成的 thread 会被当前打开的 Desktop 自动加载，但这项产品协同行为不等于两套协议共用 owner 或事件流。

## 目标

- 复用现有官方 App Server stdio wire client、mapper 和 Provider，实现完整 remote 能力。
- 持有一个长期 stdio session，关联并发 request、approval 和 notification。
- 将 remote unavailable、事件、replay 和 runtime executable 生命周期限制在 remote Gateway。
- 保持 remote route/event 完整，不向 Desktop companion 或桌宠投影写入任何来源状态。

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

Provider instance 启动并 initialize 官方 App Server stdio 子进程，reader/writer 长期运行。request id 映射允许 response 乱序返回；notification 经 mapper 生成 Provider v1 conversation、turn、delta 与 approval event。Provider 复用同一 session 完成 list/read/start/steer/interrupt/approval，不经过 Tauri 或 compat DTO。

协议 DTO 以本机官方 `codex-cli 0.151.0` 的 `app-server generate-json-schema` 输出、官方 App Server 文档和同一 binary 的纵向 smoke 为证据。上游 wire 不要求 `jsonrpc`：request 是 `id + method`，可带或不带 `params`/`trace`；notification 是 `method`，可带或不带 `params`/`emittedAtMs`；response 必须是 `id + result` 或 `id + error` 二选一，error 必须有整数 code 与非空 message。历史 fixture 若携带 `jsonrpc`，只接受 `"2.0"`。缺失 `params` 由具体 method 的 typed DTO 决定是否成立。Provider 自己面向 Host 的 stdio Provider Protocol 仍严格使用 JSON-RPC 2.0，两层 envelope 不共用判定规则。

`initialize` 必须返回 `codexHome/platformFamily/platformOs/userAgent`。`Thread`、`Turn`、start/resume response 的必需字段严格解码，不能用 `serde(default)` 或伪造 workspace permission/status/timestamp。`ThreadStartResponse`、`ThreadResumeResponse` 的结构化 `SandboxPolicy` 是 create 后权限的唯一证据。

`thread/list`、`thread/read` 只返回历史目录/快照，不代表 thread 已加载进当前 session。每个 `CodexAppServerSession` 独立维护 `Unknown → Resuming → Loaded` 状态：并发成功时等待者共享同一 Loaded 证据；resume 失败不缓存，等待者被唤醒后重新竞争并发起下一次 resume。`thread/start`、成功 resume 或当前 session 的 thread/turn/approval 通知构成 loaded 证据。新 session 不继承这张表，因此必须重新 resume。

`conversation.get` 只使用 `thread/read(includeTurns: true)` 的 turn/item 顺序作为历史事实，不扫描 transcript。Mapper 把 Codex `ThreadItem` 投影为 Provider v1 的通用 `ConversationItem/ConversationContent`：每个 item 显式携带 owning conversation 与 turn，Host 要求 owning conversation 与请求的 routed conversation 完全相等；user/agent message、reasoning summary、command/file、MCP/dynamic tool 和未知安全占位均保留原生 item id。未知项不携带 arguments、result 或原 DTO。reasoning 只公开 `summary`，绝不公开 raw `content` 或 `item/reasoning/textDelta`。

无原生 content id 的内容使用位置稳定且与文本无关的派生 ID，例如 `${itemId}:input:${index}`、`${itemId}:text`、`${itemId}:summary:${index}`、`${itemId}:command` 与 `${itemId}:output`。live `turn.outputDelta` 使用相同 `itemId/contentId`。in-progress turn 的 agent/plan/reasoning body 和运行中 command output 不进入 committed snapshot；item identity/status 仍可见，权威完成内容来自 completed item。Gateway 保留查询前 cursor，客户端据稳定 content id 合并 replay，从而同时避免查询竞态丢事件和重复追加。

App Server client 内部保留 `Success`、`NotSent`、`ExplicitRpcReject`、`SentOutcomeUnknown` 四种请求证据，用于正确维护同一 session 的 loaded-thread 状态；调用 writer 前的 Shutdown/锁失败属于 NotSent，writer 已被调用后的 I/O、断线、timeout 或 pending Shutdown 属于 SentOutcomeUnknown。这些证据不进入 Desktop companion，也不触发来源排除。Provider `turn.steer/interrupt` 在 ack 后读取 `thread/read` 的权威 turn；不会根据 ack 伪造完整 Turn。

当前 Provider v1 只能安全表达普通 command/file 的 accept/decline 二元审批。含 `additionalPermissions`、`networkApprovalContext`、policy amendment、`writeStdin`、`grantRoot`、结构化 decision 或未知字段语义的请求不发布 Approval，并使用原 request id 返回上游 error `-32601`。tool user input、MCP elicitation 和其他未知 server request 同样明确拒绝。

每次 App Server session 创建唯一 generation，approval `nativeResourceId` 同时编码 generation 与原 App Server request id。pending approval 只有 Provider instance runtime 一张权威 map；resolve 必须匹配四段 route、当前 generation 和完整 approval resource，确保响应回到持有请求的同一进程。Provider 同时保留当前 session 内实际观察到的 approval ledger，resolved 后不删除历史记录；`conversation.get` 只把这些真实记录插到关联 command/file item 后。App Server 持久 ThreadItem 本身没有 approval item，因此不得从 command completed/declined 猜测是否曾请求审批；Provider 重启前未持久化的旧 session approval 历史属于已知限制。

runtime resolver 提供已验证 executable。应用启动、设置 path、清除 path 或刷新 runtime 时，Host 只更新 Codex manifest instance 的 `appServerExecutable` 并显式重启该插件；Desktop companion 的 socket connection、generation、owner 与 revision 不受影响。App Server unavailable 只改变 remote Provider 状态，不清空 companion projection。

compat 是无状态 DTO/event 映射：delta 自带 conversation route，所有资源直接携带 `providerPluginId`。它不保存 turn map、不回查 plugin registry，也不访问 `CodexThreadScope`、Desktop adapter 或 Pet projection。

## 涉及模块

- `crates/providers/codepet-provider-codex/src/{client,protocol,mapper,provider}.rs`：App Server 子进程、官方 DTO、Provider v1 映射与实例路由。
- `crates/providers/codepet-provider-codex/src/main.rs`：生成 SDK codec/dispatcher 驱动的 stdio JSON-lines 主循环。
- `crates/codepet-host`：manifest、进程、实例、Gateway route 与事件 replay。
- `src-tauri/src/runtime_gateway/provider_host_compat.rs`：既有远程调用面到 Gateway v1 的薄适配。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs`、`src-tauri/src/lib.rs`：resolver 注入、插件 refresh 与 remote event bridge。

## 风险与验证

- App Server wire 漂移：必跑 fixture 使用无 `jsonrpc` 的官方 response/notification 形状，并覆盖可选 `trace`、`emittedAtMs` 与缺失 `params`；升级 CLI 后重新生成 schema 并重跑。
- response schema 漂移：用当前 binary 重新生成 JSON Schema/TS binding，核对 start/resume `SandboxPolicy` 与 permissions request/response；真实对象 fixture 必须保持可反序列化。
- 本机可执行文件兼容性：显式 integration test 使用 resolver 得到的绝对 executable，已覆盖 initialize、instance create/start、conversation list 与 instance stop；无 executable 的环境只跑官方录制 fixture，不把 integration test 假装成必过。
- 历史 thread 生命周期：list/read 后首次运行时动作必须先 resume；测试覆盖真实 `-32600/no rollout`、写前 Shutdown、写后断线、并发失败 waiter 重试、成功缓存和新 session 重新 resume。
- 历史投影泄漏或重复：fixture 覆盖 ordered user/assistant/reasoning/command/file/tool/unknown，断言 raw reasoning 和原生 payload 不出现；completed/in-progress 对照断言可变 body 只在完成后进入 snapshot，delta 与 snapshot 使用相同 content id。
- server request 悬挂：真实 Provider 二进制测试覆盖 `additionalPermissions.network` 得到原 id 的 `-32601`，同时断言无 Approval event。
- 审批串 session：真实 Provider 二进制 stop/start 后复用相同 App Server request id，旧 approval 必须 `stale_approval_session`，当前 approval 才能回写。
- 子进程终态：超长 stdout/stderr 物理行先完整 drain；超长或读取失败随后进入同一个 session terminal path，清空 pending 并终止 App Server。terminal fault 与 subscribers 由同一把锁保护；barrier 竞态测试与 late-subscriber 测试覆盖“读取已有 fault / 注册等待未来 fault”的原子二选一。
- 子进程退出：remote Provider 变为 unavailable；当前没有持续 supervisor 自动拉起，需显式 refresh 或重启，不能借 companion 掩盖。
- 生命周期串扰：测试分别将 remote/companion 置为 unavailable，并断言另一条 Provider 与状态仍可用。
- 事件串流：Host 从真实 manifest binary 收到 Provider event，断言 remote replay/event 有数据而 companion replay、Pet channel 与 activity store 仍为空。
- remote thread 经 Desktop 再出现：两条链路允许暂时同名；隔离测试断言 Provider 不修改 companion state，也不进入 companion/Pet/activity。
- 网络能力误判：当前验证只证明 Provider/runtime 和进程内 transport，不宣称真实远程网络已上线。

## 测试计划

- App Server fake peer 使用官方 wire 完整请求/通知闭环；可选真实 executable smoke 覆盖 initialize → instance create/start → conversation list → instance stop。
- list → resume → send、同 session 成功缓存、失败 waiter 重试与新 session 重新 resume。
- `thread/read(includeTurns)` 的 ordered history、稳定 item/content ID、safe unknown、reasoning summary-only 与真实 approval ledger 合并。
- 额外权限/结构化 decision/未知 server request 的 `-32601` response 与 capability/event 一致性。
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
- remote/Desktop 同名资源的产品合并策略尚未定义；当前不做跨链路同步。
