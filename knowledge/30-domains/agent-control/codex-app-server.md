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

Provider instance 启动并 initialize 一个官方 App Server stdio observer 子进程，长期 reader 只服务 `model/list`、`thread/list`、metadata `thread/read` 与 `thread/turns/list`。observer 从不发送 `thread/start`、`thread/resume`、turn 或 approval 写操作，因此长期存在也不会加载历史 thread 或持有 writer。

`conversation.create` 使用独立的一次性 App Server 执行 `thread/start`，拿到权威 response 后立即关闭。observer 与一次性 create 都从 spawn 前开始登记到 instance-generation lifecycle registry；spawn 后、initialize 前登记真实 session，最终 `thread/start` 写入与 cancel 通过短门线性化。`turn.start/steer/interrupt` 首次写入某 conversation 时创建执行槽，spawn 新 App Server、subscribe、`thread/resume`，再在该槽的串行 operation lock 内完成 turn 操作；approval 必须回到同一 generation 的槽。response request id 仍允许乱序关联，notification 经 mapper 生成 Provider v1 conversation、turn、delta 与 approval event，不经过 Tauri 或 compat DTO。

执行槽使用 `Creating → Ready → Closing → Closed`，创建失败则进入 `Failed` 并从 map 淘汰。同 conversation 并发首次写共享同一个 Creating 槽和 resume 结果；不同 conversation 使用不同进程。Ready 槽记录 generation 与 active turn id，事件线程只持 runtime 的 `Weak` 引用，不与 runtime/session 形成强引用环。Creating 从插入 map 起就有 attempt generation/cancellation；子进程完成 spawn、尚未 initialize 前即登记，因此 Provider stop 或 observer fail 可以直接关闭 pending initialize/resume。initialize/subscribe 后继续核对 instance 与 slot；resume request 的写入和 cancel 通过一把短锁线性化，锁只覆盖最终复核与 frame 写入，不覆盖 response 等待，也没有给正常 turn 增加短 timeout。

协议 DTO 以本机官方 `codex-cli 0.151.0` 的 `app-server generate-json-schema` 输出、官方 App Server 文档和同一 binary 的纵向 smoke 为证据；另一个实际故障环境的 `0.151.0-alpha.7.2` 已验证 `thread/turns/list` cursor/full 可用，而 `thread/items/list` 返回 `-32601`，因此当前实现不得依赖后者。上游 wire 不要求 `jsonrpc`：request 是 `id + method`，可带或不带 `params`/`trace`；notification 是 `method`，可带或不带 `params`/`emittedAtMs`；response 必须是 `id + result` 或 `id + error` 二选一，error 必须有整数 code 与非空 message，可选 `data` 会保留在内部错误证据中。历史 fixture 若携带 `jsonrpc`，只接受 `"2.0"`。缺失 `params` 由具体 method 的 typed DTO 决定是否成立。Provider 自己面向 Host 的 stdio Provider Protocol 仍严格使用 JSON-RPC 2.0，两层 envelope 不共用判定规则。

生产 stdio reader 与 RPC dispatch 分离：普通 request 最多并发 16 个、排队 32 个；`instance.stop`、`instance.destroy`、`provider.shutdown` 使用独立的 2 并发/4 排队保留通路。reader 对 channel 使用非阻塞入队，队列满则用原 id 返回 retryable `provider_overloaded`，因此仍能继续读取 EOF/fatal。每个 response 保留原 id，可按完成顺序乱序返回，event/response 最终通过同一 stdout mutex 串行写出；EOF、坏帧以及 response/event stdout 写失败都会触发 Provider shutdown 与有界 drain/abort。event sink 在释放 stdout mutex 后只发送一次 terminal 信号，清理路径不依赖继续输出。

`initialize` 必须返回 `codexHome/platformFamily/platformOs/userAgent`。`Thread`、`Turn`、start/resume response 的必需字段严格解码，不能用 `serde(default)` 或伪造 workspace permission/status/timestamp。`ThreadStartResponse`、`ThreadResumeResponse` 的结构化 `SandboxPolicy` 是 create 后权限的唯一证据。

`thread/list`、metadata `thread/read` 与 `thread/turns/list` 只返回历史目录/快照，不代表 thread 已加载进 observer 或执行 session。每个 `CodexAppServerSession` 独立维护 `Unknown → Resuming → Loaded` 状态：执行槽创建层先合并同 conversation 的首次 spawn；session 内再保证成功 resume 只产生一个 Loaded 证据。resume 失败不缓存，整个失败执行槽会关闭并移除；新显式请求才可创建新 session 并重新 resume。`thread/start`、成功 resume 或当前执行 session 的 thread/turn/approval 通知构成 loaded 证据，observer 的 read/list 不构成。

`conversation.get` 先发送 `thread/read(includeTurns: false)` 取得 metadata，再用 `thread/turns/list(limit: 10, itemsView: full, sortDirection: asc)` 按 opaque cursor 拉取完整 turns。每页保持上游升序，映射完成即释放原始 turn/item DTO；重复 cursor 或超过 10,000 页会 fail closed。该路径不调用 `ensure_thread_loaded`/`thread/resume`，不扫描 transcript，也不写 loaded-thread 状态或创建 writer 执行上下文，因此另一个 runtime 已持有 thread writer 时仍可读取历史。Mapper 跨页保留旧投影顺序与审批插入位置；user/agent message、reasoning summary、command/file、MCP/dynamic tool 和未知安全占位继续使用相同 item/content ID，reasoning 只公开 `summary`，绝不公开 raw `content` 或 `item/reasoning/textDelta`。

无原生 content id 的内容使用位置稳定且与文本无关的派生 ID，例如 `${itemId}:input:${index}`、`${itemId}:text`、`${itemId}:summary:${index}`、`${itemId}:command` 与 `${itemId}:output`。live `turn.outputDelta` 使用相同 `itemId/contentId`。in-progress turn 的 agent/plan/reasoning body 和运行中 command output 不进入 committed snapshot；item identity/status 仍可见，权威完成内容来自 completed item。Gateway 保留查询前 cursor，客户端据稳定 content id 合并 replay，从而同时避免查询竞态丢事件和重复追加。

App Server client 内部保留 `Success`、`NotSent`、`ExplicitRpcReject`、`SentOutcomeUnknown` 四种请求证据，用于正确维护 loaded-thread 状态和执行槽清理；调用 writer 前的 Shutdown/锁失败属于 NotSent，writer 已被调用后的 I/O、断线、timeout、response 解码失败或 pending Shutdown 属于 SentOutcomeUnknown。这些证据不进入 Desktop companion，也不触发来源排除。Provider 不自动重试可能已经送达的 `turn/start`，`clientUserMessageId` 原样保留。`turn.steer/interrupt` 在 ack 后读取 `thread/read` 的权威 turn；不会根据 ack 伪造完整 Turn。

`thread/resume` 的明确 RPC reject 只有同时满足官方 active-writer 形状才标准化为现有 `ProtocolError`：native `code=-32600`、`data` 缺失或 null，且 message 精确等于 `thread <当前 conversation id> already has an active writer`。标准错误为 `code=conversation_write_conflict`、`retryable=true`，details 是 `operation=thread/resume` 与 `reason=owned-by-other-runtime`；对外不回传原生 thread/owner message。wrong code、近似 message、其他 thread id 或非空 data 都保留普通 Provider error。

当前 Provider v1 只能安全表达普通 command/file 的 accept/decline 二元审批。含 `additionalPermissions`、`networkApprovalContext`、policy amendment、`writeStdin`、`grantRoot`、结构化 decision 或未知字段语义的请求不发布 Approval，并使用原 request id 返回上游 error `-32601`。tool user input、MCP elicitation 和其他未知 server request 同样明确拒绝。

每次执行 App Server 创建唯一 generation，approval `nativeResourceId` 同时编码 generation 与原 App Server request id。pending approval 只有 Provider instance runtime 一张权威 map；resolve 必须匹配四段 route、owning conversation、当前执行槽 generation 和完整 approval resource，确保响应回到持有请求的同一进程。Provider 同时保留实际观察到的 approval ledger，resolved 后不删除历史记录；执行槽异常关闭时未解决 approval 变为 expired，pending 与 session 强引用被移除。`conversation.get` 只把这些真实记录插到关联 command/file item 后。App Server 持久 ThreadItem 本身没有 approval item，因此不得从 command completed/declined 猜测是否曾请求审批；Provider 重启前未持久化的旧 session approval 历史属于已知限制。

执行会话由 active turn 状态所有，不由 Remote 详情页、event subscriber 或 WSS connection 所有。`running`、`waiting-approval`、`waiting-user-input` 都不是释放条件；Remote 离开或断线不调用 Provider lifecycle。每条 writer route 都通过同一个 helper 获取 operation lock 后复核 slot identity、Ready state 与 generation；预取 handle 若已 Closing/Closed 就重试新 generation，approval 还必须匹配原 owning generation。`turn/completed` 通知在同一 operation lock 内先切 Closing，再发布权威 event、关闭进程、删除同一槽并唤醒等待者。完成 A 只移除 A 的 generation，B 的 operation、approval 与进程保持不变；旧 generation 事件不能删除新槽。

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
- 历史 thread 生命周期：`conversation.get` 重复读取只在 observer 发送 metadata `thread/read` 与 `thread/turns/list` pages，不创建新进程、不参与 loaded-thread 状态机；后续首次写动作才允许 execution session resume。测试清空进程日志后重复读取，断言只有只读分页方法，没有 `process/start` 或 `thread/resume`。
- 历史帧边界：App Server 单 page 与 Provider→Host 最终 JSONL 分别受 16 MiB 限制。binary fixture 用多页小于上限、合并后大于上限的历史证明 Provider 返回稳定 `provider_response_too_large` 后仍可服务；bounded-line 单元测试只证明超长上游物理行会完整 drain 并形成 protocol error，尚未纵向覆盖某个 turns page 超限。真正任意大历史仍需要更细的上游分页和 Provider/Gateway 公共协议分页。
- 历史分页一致性：App Server 没有给 metadata read、turn pages 与 Provider approval ledger 提供共同 snapshot token；分页期间若 turn/approval 正在变化，同一次 `conversation.get` 可能混合相邻时刻的 metadata、active turn 或 approval 状态。当前静态 fixture 只验证确定性历史；在宣称原子 snapshot 前必须增加跨页并发变更 fixture，或由上游/公共协议提供 revision 语义。
- turn/writer 生命周期：纵向 fixture 使用每 PID 独立日志证明 observer、一次性 create 与 execution 是三个边界；同一 active turn 的 steer/interrupt/approval 复用 execution；terminal notification/权威 snapshot 后下一次写使用新 pid。handle barrier 精确覆盖“请求先拿 Ready handle、terminal 先拿 operation lock 并 Closing”，断言请求随后在新 PID 成功；resume barrier 覆盖 cancel 先线性化，断言无 resume/start。64 轮重复测试锁定终态释放与 PID 证据稳定性。
- 并发与失败清理：并发首次 `turn.start` 只有一次 resume/一次 `turn/start`；observer、one-shot create 与 execution 的 initialize/resume 悬挂时 instance stop 都在 2 秒内关闭子进程并唤醒请求。one-shot EOF、五轮 observer start/stop 以及异步 event stdout broken pipe 分别证明全局 shutdown、generation 终态和 PID 回收。生产 Provider binary 在 16 个普通 dispatch 全悬挂时仍用保留通路完成 stop；16 active + 32 pending 后第 49 个请求按 id 明确过载且未执行，EOF 仍在 2 秒内回收。fixture 另行覆盖 execution initialize reject、官方 writer conflict 及反例、turn RPC reject、App Server crash 与 sent-outcome-unknown。
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
- Provider stdio 覆盖乱序 id response、跨 conversation 并发、16 个悬挂请求期间的保留 stop、16+32 后按 id 过载、至少 49 帧后的 EOF 有界回收。
- observer model/list、thread list、metadata read/turns list 与 execution resume/send 的进程边界；conversation.create 一次性进程退出。
- 同 conversation 并发首次 send 合并创建；同 turn steer/interrupt/approval 复用；waiting approval/user input 保持；terminal notification/snapshot、crash、reject、sent-unknown、initialize/resume failure 与 stop 均清理。
- 两 conversation 并行隔离；A 终态与 Provider stop 后的新请求都必须以新进程 resume，B 在 A 释放前后保持原进程。
- `conversation.get` 在另一个 runtime 持 writer 时仍只发送 metadata `thread/read` 与升序 `thread/turns/list(itemsView=full)` pages；重复读取不改变 loaded/configuration 状态、不创建新 session，并继续覆盖跨页 ordered history、稳定 item/content ID、active turn、safe unknown、reasoning summary-only 与真实 approval ledger 合并。
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
- instance session 生命周期规约见 `../../60-rules/codex-provider-instance-session-lifecycle.md`。
- 完整边界、协议矩阵和开发安装见 `../../10-architecture/codex-provider-plugin-runtime.md`。

## 未知项

- observer App Server 的自动 supervisor/reconciliation 策略尚未实现；execution 只由显式写请求和 turn 终态管理。
- 本阶段 fixture 证明进程/路由生命周期；真实 Codex CLI 是否在所有版本都以相同 message 报告 writer ownership，仍需真实 smoke 持续观察。
- 真实远程网络 transport 尚未实现。
- remote/Desktop 同名资源的产品合并策略尚未定义；当前不做跨链路同步。
