# Codex Remote App Server Provider

> 当前状态（2026-09-01）：独立二进制 `codepet-provider-codex` 按 Host manifest instance 持有长期纯读 observer App Server，并按 conversation/active turn 持有短生命周期执行 App Server。它通过 Provider Protocol v1 服务远程 Gateway，与本机 Codex Desktop companion 私有 IPC 完全分离，产生的数据不进入桌宠 activity store。

## 背景

远程控制需要项目/会话目录、创建、读取、继续、停止和审批。Desktop 私有 IPC 没有 thread directory 或 create，因此不能承担这组能力。实机已确认独立 App Server 创建并完成的 thread 会被当前打开的 Desktop 自动加载，但这项产品协同行为不等于两套协议共用 owner 或事件流。

## 目标

- 复用现有官方 App Server stdio wire client、mapper 和 Provider，实现完整 remote 能力。
- 长期 observer 只承担 list/read/model；writer 执行会话按 conversation 隔离，并与 active turn 生命周期绑定。
- 将 remote unavailable、事件、replay 和 runtime executable 生命周期限制在 remote Gateway。
- 保持 remote route/event 完整，不向 Desktop companion 或桌宠投影写入任何来源状态。

## 非目标

- 不把 App Server 当作 Desktop IPC 的 fallback 或 Desktop 当前 owner 的权威状态。
- 不把 remote snapshot/event 发往桌宠 activity store。
- 不恢复旧 `PetEvent` driver、全局 reply session、Hook、audit、transcript 或文件监听数据源。
- 本阶段不修改 Gateway LAN/WSS、配对、凭据或 relay；网络连接状态不能作为 execution 释放条件。

## 当前能力

Codex Provider v1 广告并实现：

- `conversation.list/get/create`
- `turn.start/steer/interrupt`
- `approval.resolve`
- App Server thread/turn/item/approval 通知到 Standard Protocol event 的映射

Gateway v1 把 `turn.send` 分发到 Provider `turn.start` 或 `turn.steer`。`RuntimeGatewayState` 只把既有 compat-v0 调用面适配到 `ProviderGatewayService`，不注册 adapter、不持有 App Server session，也不产生第二份事件。Tauri 远程边界继续使用 `runtime_gateway_request`、`runtime_gateway_replay` 与 `runtime-gateway-event`；桌宠不订阅这些接口。

## 实现路径

Provider instance 启动并 initialize 一个官方 App Server stdio observer 子进程，长期 reader 只服务 `model/list`、`thread/list` 与 `thread/read`。observer 从不发送 `thread/start`、`thread/resume`、turn 或 approval 写操作，因此长期存在也不会加载历史 thread 或持有 writer。

`conversation.create` 使用独立的一次性 App Server 执行 `thread/start`，拿到权威 response 后立即关闭。`turn.start/steer/interrupt` 首次写入某 conversation 时创建执行槽，spawn 新 App Server、subscribe、`thread/resume`，再在该槽的串行 operation lock 内完成 turn 操作；approval 必须回到同一 generation 的槽。response request id 仍允许乱序关联，notification 经 mapper 生成 Provider v1 conversation、turn、delta 与 approval event，不经过 Tauri 或 compat DTO。

执行槽只有 `Creating → Ready → Closed/Failed`。同 conversation 并发首次写共享同一个 Creating 槽和 resume 结果；不同 conversation 使用不同进程。Ready 槽记录 generation 与 active turn id，事件线程只持 runtime 的 `Weak` 引用，不与 runtime/session 形成强引用环。创建中的 session 在 spawn 完成后立即登记到槽，因此 Provider stop 或 observer fail 可以中断仍在等待 resume 的请求；initialize 自身使用已有 5 秒 fail-closed 边界，正常 turn 运行不使用固定短超时。

协议 DTO 以本机官方 `codex-cli 0.151.0` 的 `app-server generate-json-schema` 输出、官方 App Server 文档和同一 binary 的纵向 smoke 为证据。上游 wire 不要求 `jsonrpc`：request 是 `id + method`，可带或不带 `params`/`trace`；notification 是 `method`，可带或不带 `params`/`emittedAtMs`；response 必须是 `id + result` 或 `id + error` 二选一，error 必须有整数 code 与非空 message。历史 fixture 若携带 `jsonrpc`，只接受 `"2.0"`。缺失 `params` 由具体 method 的 typed DTO 决定是否成立。Provider 自己面向 Host 的 stdio Provider Protocol 仍严格使用 JSON-RPC 2.0，两层 envelope 不共用判定规则。

`initialize` 必须返回 `codexHome/platformFamily/platformOs/userAgent`。`Thread`、`Turn`、start/resume response 的必需字段严格解码，不能用 `serde(default)` 或伪造 workspace permission/status/timestamp。`ThreadStartResponse`、`ThreadResumeResponse` 的结构化 `SandboxPolicy` 是 create 后权限的唯一证据。

`thread/list`、`thread/read` 只返回历史目录/快照，不代表 thread 已加载进 observer 或执行 session。每个 `CodexAppServerSession` 独立维护 `Unknown → Resuming → Loaded` 状态：执行槽创建层先合并同 conversation 的首次 spawn；session 内再保证成功 resume 只产生一个 Loaded 证据。resume 失败不缓存，整个失败执行槽会关闭并移除；新显式请求才可创建新 session 并重新 resume。`thread/start`、成功 resume 或当前执行 session 的 thread/turn/approval 通知构成 loaded 证据，observer 的 read 不构成。

`conversation.get` 严格只发送一次 `thread/read(includeTurns: true)`，使用其 turn/item 顺序作为历史事实，不调用 `ensure_thread_loaded`/`thread/resume`，不扫描 transcript，也不写 loaded-thread 状态或创建 writer 执行上下文。因此另一个 runtime 已持有 thread writer 时，历史详情读取仍可成功。Mapper 把 Codex `ThreadItem` 投影为 Provider v1 的通用 `ConversationItem/ConversationContent`：每个 item 显式携带 owning conversation 与 turn，Host 要求 owning conversation 与请求的 routed conversation 完全相等；user/agent message、reasoning summary、command/file、MCP/dynamic tool 和未知安全占位均保留原生 item id。未知项不携带 arguments、result 或原 DTO。reasoning 只公开 `summary`，绝不公开 raw `content` 或 `item/reasoning/textDelta`。

无原生 content id 的内容使用位置稳定且与文本无关的派生 ID，例如 `${itemId}:input:${index}`、`${itemId}:text`、`${itemId}:summary:${index}`、`${itemId}:command` 与 `${itemId}:output`。live `turn.outputDelta` 使用相同 `itemId/contentId`。in-progress turn 的 agent/plan/reasoning body 和运行中 command output 不进入 committed snapshot；item identity/status 仍可见，权威完成内容来自 completed item。Gateway 保留查询前 cursor，客户端据稳定 content id 合并 replay，从而同时避免查询竞态丢事件和重复追加。

App Server client 内部保留 `Success`、`NotSent`、`ExplicitRpcReject`、`SentOutcomeUnknown` 四种请求证据，用于正确维护 loaded-thread 状态和执行槽清理；调用 writer 前的 Shutdown/锁失败属于 NotSent，writer 已被调用后的 I/O、断线、timeout、response 解码失败或 pending Shutdown 属于 SentOutcomeUnknown。这些证据不进入 Desktop companion，也不触发来源排除。Provider 不自动重试可能已经送达的 `turn/start`，`clientUserMessageId` 原样保留。`turn.steer/interrupt` 在 ack 后读取 `thread/read` 的权威 turn；不会根据 ack 伪造完整 Turn。

`thread/resume` 的明确 RPC reject 只有在 message 同时证明 writer ownership 且 owner 是其他 runtime/process/client 时，才标准化为现有 `ProtocolError`：`code=conversation_write_conflict`、`retryable=true`、details 为 `operation=thread/resume` 与 `reason=owned-by-other-runtime`。对外 message 不包含原生进程、客户端或内部 owner 身份。其他 reject 仍走普通 Provider error，不猜测冲突。

当前 Provider v1 只能安全表达普通 command/file 的 accept/decline 二元审批。含 `additionalPermissions`、`networkApprovalContext`、policy amendment、`writeStdin`、`grantRoot`、结构化 decision 或未知字段语义的请求不发布 Approval，并使用原 request id 返回上游 error `-32601`。tool user input、MCP elicitation 和其他未知 server request 同样明确拒绝。

每次执行 App Server 创建唯一 generation，approval `nativeResourceId` 同时编码 generation 与原 App Server request id。pending approval 只有 Provider instance runtime 一张权威 map；resolve 必须匹配四段 route、owning conversation、当前执行槽 generation 和完整 approval resource，确保响应回到持有请求的同一进程。Provider 同时保留实际观察到的 approval ledger，resolved 后不删除历史记录；执行槽异常关闭时未解决 approval 变为 expired，pending 与 session 强引用被移除。`conversation.get` 只把这些真实记录插到关联 command/file item 后。App Server 持久 ThreadItem 本身没有 approval item，因此不得从 command completed/declined 猜测是否曾请求审批；Provider 重启前未持久化的旧 session approval 历史属于已知限制。

执行会话由 active turn 状态所有，不由 Remote 详情页、event subscriber 或 WSS connection 所有。`running`、`waiting-approval`、`waiting-user-input` 都不是释放条件；Remote 离开或断线不调用 Provider lifecycle。`turn/completed` 的 completed/failed/interrupted 通知先发布权威 event，再关闭对应进程并从 map 删除；steer/interrupt 的权威 terminal snapshot 也走相同清理。完成 A 只移除 A 的 generation，B 的 operation、approval 与进程保持不变。App Server crash、initialize/resume 失败、明确 RPC reject、sent-outcome-unknown、instance stop 与 provider shutdown 都有确定关闭路径。

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
- 历史 thread 生命周期：`conversation.get` 重复读取只在 observer 发送 `thread/read`，不创建新进程、不参与 loaded-thread 状态机；后续首次写动作才允许执行 session resume。测试清空进程日志后重复读取，断言没有 `process/start` 或 `thread/resume`。
- turn/writer 生命周期：纵向 fixture 记录每个 App Server pid，证明 observer、一次性 create 与 execution 是三个边界；同一 active turn 的 steer/interrupt/approval 复用 execution；waiting approval/user input 和无 Remote 操作的间隔不释放；terminal notification/权威 snapshot 后下一次写使用新 pid；两 conversation 并行时释放 A 不影响 B。
- 并发与失败清理：并发首次 `turn.start` 只有一次 resume/一次 `turn/start`；resume 悬挂时 instance stop 会在 30 秒 request timeout 前关闭子进程并唤醒请求。fixture 另行覆盖 execution initialize reject、writer conflict、turn RPC reject、App Server 异步 crash 与 sent-outcome-unknown，失败后显式下一请求取得新 pid，且 Provider 没有后台重试 `turn/start`。
- 历史投影泄漏或重复：fixture 覆盖 ordered user/assistant/reasoning/command/file/tool/unknown，断言 raw reasoning 和原生 payload 不出现；completed/in-progress 对照断言可变 body 只在完成后进入 snapshot，delta 与 snapshot 使用相同 content id。
- server request 悬挂：真实 Provider 二进制测试覆盖 `additionalPermissions.network` 得到原 id 的 `-32601`，同时断言无 Approval event。
- 审批串 session：真实 Provider 二进制 stop/start 后复用相同 App Server request id，旧 approval 必须 `stale_approval_session`，当前 approval 才能回写。
- 子进程终态：超长 stdout/stderr 物理行先完整 drain；超长或读取失败随后进入同一个 session terminal path，清空 pending 并终止 App Server。terminal fault 与 subscribers 由同一把锁保护；barrier 竞态测试与 late-subscriber 测试覆盖“读取已有 fault / 注册等待未来 fault”的原子二选一。
- observer 子进程退出：Provider instance 变为 Error，并关闭全部 execution；当前没有持续 supervisor 自动拉起，需显式 refresh 或重启，不能借 companion 掩盖。单个 execution crash 只清理 owning conversation，不把健康 observer 或其他 execution 一并标错。
- 生命周期串扰：测试分别将 remote/companion 置为 unavailable，并断言另一条 Provider 与状态仍可用。
- 事件串流：Host 从真实 manifest binary 收到 Provider event，断言 remote replay/event 有数据而 companion replay、Pet channel 与 activity store 仍为空。
- remote thread 经 Desktop 再出现：两条链路允许暂时同名；隔离测试断言 Provider 不修改 companion state，也不进入 companion/Pet/activity。
- 网络能力误判：当前验证只证明 Provider/runtime 和进程内 transport，不宣称真实远程网络已上线。

## 测试计划

- App Server fake peer 使用官方 wire 完整请求/通知闭环；可选真实 executable smoke 覆盖 initialize → instance create/start → conversation list → instance stop。
- observer list/read/model 与 execution resume/send 的进程边界；conversation.create 一次性进程退出。
- 同 conversation 并发首次 send 合并创建；同 turn steer/interrupt/approval 复用；waiting approval/user input 保持；terminal notification/snapshot、crash、reject、sent-unknown、initialize/resume failure 与 stop 均清理。
- 两 conversation 并行隔离；A 终态与 Provider stop 后的新请求都必须以新进程 resume，B 在 A 释放前后保持原进程。
- `conversation.get` 在另一个 runtime 持 writer 时仍每次只发送一次 `thread/read(includeTurns)`；重复读取不改变 loaded/configuration 状态、不创建新 session，并继续覆盖 ordered history、稳定 item/content ID、safe unknown、reasoning summary-only 与真实 approval ledger 合并。
- 额外权限/结构化 decision/未知 server request 的 `-32601` response 与 capability/event 一致性。
- remote `conversation.list/create` 明确路由 Provider Gateway；companion 不广告或实现这两项能力。
- runtime executable refresh 只更新 Host instance setting 并重启 Codex 插件。
- remote event 不进入 companion Tauri replay/event，桌宠源码不引用 remote client。
- 相关 Rust tests、全量 Vitest 和前端 production build。

## 知识沉淀

- 双链路决策见 `../../50-decisions/codex-remote-and-desktop-companion-dual-channel.md`。
- 故障排查见 `../../40-runbooks/codex-app-server-unavailable.md`。
- channel 约束见 `../../60-rules/codex-provider-channel-isolation.md`。
- turn writer 生命周期规约见 `../../60-rules/codex-provider-turn-writer-lifecycle.md`。
- 完整边界、协议矩阵和开发安装见 `../../10-architecture/codex-provider-plugin-runtime.md`。

## 未知项

- observer App Server 的自动 supervisor/reconciliation 策略尚未实现；execution 只由显式写请求和 turn 终态管理。
- 本阶段 fixture 证明进程/路由生命周期；真实 Codex CLI 是否在所有版本都以相同 message 报告 writer ownership，仍需真实 smoke 持续观察。
- 真实远程网络 transport 尚未实现。
- remote/Desktop 同名资源的产品合并策略尚未定义；当前不做跨链路同步。
