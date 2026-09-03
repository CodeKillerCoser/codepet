# Codex Provider writer 必须由交互租期与 active turn 共同约束

## 规则

Codex Provider 的长期 App Server 必须是纯读 observer。任何可能取得 thread writer 的 `thread/start`、`thread/resume`、turn 或 approval 操作都不得进入 observer；历史 conversation 的 writer 只能由按 conversation 隔离的执行 App Server 持有。

Gateway/Provider 只使用幂等 `conversation.acquireInteraction` 管理交互权。Codex 首次 acquire 执行 `thread/resume` 并返回真实 permission/model/reasoning selection，后续 acquire 只延长 30 秒租期；Remote 在详情页每 10 秒续租。Claude/OpenCode 等没有 writer lock 的 harness 可以不执行 resume，返回成功结构即可。

执行会话由交互租期与 active turn 共同所有。无 active turn 且租期过期时必须关闭 writer；`running`、`waiting-approval`、`waiting-user-input` 不能因租期过期而被中断，终态后若租期已过期再释放。App Server terminal fault、initialize/resume 失败、明确 RPC reject、sent-outcome-unknown、instance stop 与 provider shutdown 仍立即移除映射并关闭子进程。

Host stdio request 不能逐条 await：普通请求与 lifecycle control 必须使用分别有界的 dispatch/queue，control 有保留容量；普通队列满不能阻塞 reader，而要按原 id 明确拒绝，使 EOF/fatal 仍可见。conversation 内部仍由 execution slot operation lock 串行。

## 适用场景

- 修改 `codepet-provider-codex` 的 instance、conversation、acquire、turn、approval 或 App Server process control。
- 修改 Remote 详情页的 acquire 周期、发送门控、模型/权限选择初始化或生命周期。
- 增加新的写操作、terminal notification、权威 snapshot reconciliation 或 Provider restart 路径。
- 修改 App Server 请求重试、超时、writer conflict 或 `clientRequestId` 语义。
- 修改 Provider binary stdio reader/dispatcher、response writer、EOF/fatal cleanup 或并发上限。

## 反例

- 让 observer 在 acquire 或第一次 `turn.start` 时 `thread/resume`，随后一直持有所有访问过的 thread writer。
- 进入详情页只读 `conversation.get` 后直接开放发送，没有先确认 acquire/resume 成功。
- 每次续租都重新 spawn/resume，或把 acquire response 反复覆盖到用户正在编辑的 selection。
- 手机离开详情页或 WSS 断开就立即关闭仍在 running/waiting 的 turn；或停止续租后永不释放空闲 writer。
- 每个请求临时 spawn 一个进程，使 steer、interrupt 或 approval 落到不同 generation。
- 同 conversation 并发首次 acquire/send 各自 spawn/resume，产生双 writer 竞争；或在 `turn/start` 结果未知时自动重发。
- A 的 terminal event 清空 Provider instance 的全部执行会话，误杀仍 active 的 B。
- Provider stdio 主循环逐条 await，导致 A 的 resume 悬挂同时阻塞 B 和 `instance.stop`；或无界 spawn Host request。
- 请求在拿 operation lock 前预取 Ready handle，拿锁后不复核 generation；或 cancel 与 resume write 之间没有共同线性化点。

## 推荐做法

- instance start 只创建 observer；list/search/get/model 全部走 observer。conversation.create 的 `thread/start` session 在返回后直接成为该 conversation 的临时 execution slot，首条 `turn.start` 必须复用它，不等待 list/get/read 物化，也不先关闭再 `thread/resume`。
- 新 thread 已能被 metadata `thread/read` 读取、但首条用户消息前 `thread/turns/list` 返回当前 thread id 对应的官方 `-32600 not materialized` 响应时，Provider 将其映射为空历史，并将 create readiness 判为成功。若独立 observer 对同一个未 materialize id 返回精确的 `-32600 thread not loaded`，只允许使用当前 instance 的 create response 暂存 metadata 返回空历史；完整历史可读或 instance stop/fail/restart 后删除暂存。错误 code、message、thread id 不精确，或该 id 没有暂存证据时继续 fail closed。
- 用 conversation-keyed 创建槽合并首次 spawn/resume，并用 per-slot operation lock 串行化 acquire/start/steer/interrupt/approval。事件线程只持 runtime 弱引用。
- acquire 在 operation lock 内复核 Ready generation，更新单调时钟租期，并从成功 resume 的 session configuration cache 构造 response。协议时间戳使用 wall clock，只用于客户端观察，Provider 的过期判断使用单调时钟。
- turn started/权威 in-progress snapshot 记录 active turn；同 turn 的 item upsert、output delta 和 approval request 也是正在执行的权威证据，必须能修复缺失或滞后的 `turn/started`。只有 `turn/completed` terminal notification 可以在 operation lock 内清除 active turn。租约有效时发布终态后保持 Ready；租约缺失或过期时先切 Closing，但必须先可靠映射并发布 terminal `TurnUpserted`，再 shutdown/kill/wait、按 conversation + generation 删除同一槽并唤醒等待者。
- 后台 reaper 只关闭“租期已过期且没有 active turn”的槽。Remote 不发送显式 release；页面退出、App 挂起或断网通过停止续租最终释放。
- App Server 的流式 `item/started` 可以为 `agentMessage` 携带合法的空 `text`，后续内容由 delta 到达。Provider 必须接受空字符串并保持 reader/writer；只有 `text` 缺失或不是字符串时才按协议错误 fail closed。
- Creating 槽从插入 map 起携带 attempt generation/cancellation；子进程完成 spawn 后、initialize 前立即登记。cancel 与第一条 `thread/resume` request 写入用短锁线性化，锁只包含最终复核与 frame 写入，不能包含悬挂 response 等待；cancel 先发生后不得再写 resume/start。
- 只对官方 active-writer 形状的 `thread/resume` reject 返回 `conversation_write_conflict`：`code=-32600`、`data` 缺失/null、message 精确匹配当前 conversation id。details 固定为 `operation=thread/resume`、`reason=owned-by-other-runtime`；wrong code、近似 message、其他 id 或非空 data 不得猜测为冲突。
- Remote 在首次 acquire 成功前禁用 composer/send；失败立即关闭发送入口并显示冲突或协议错误，周期任务可继续重试。首次非空 acquire selection 可以覆盖只读 snapshot/default，后续续租不得覆盖用户手动选择。
- 保留 `NotSent`、`ExplicitRpcReject`、`SentOutcomeUnknown` 的区别。未知发送结果关闭会话并返回错误，不在 Provider 内重试 `turn/start`，继续原样传递 client request id。
- stdio reader 与 dispatch 分离：普通请求 16 active + 32 pending，lifecycle control 2 active + 4 pending；入队必须非阻塞，过载按原 id 返回 retryable `provider_overloaded`，被拒请求不得执行。event/response 只在共享 writer mutex 内串行；EOF/fatal 先关闭 Provider/App Server，再有界 drain/abort。

## 来源

- `../10-architecture/codex-provider-plugin-runtime.md`
- `../30-domains/agent-control/codex-app-server.md`
- `conversation.get` 纯读修复、turn-scoped writer 生命周期与 `conversation.acquireInteraction` 租期决策。

## 验证方式

- 清空 fixture 进程日志后重复 `conversation.get`，observer 只允许 metadata `thread/read(includeTurns=false)` 与 cursor `thread/turns/list(itemsView=full)`，不得出现新 process、`thread/resume` 或其他写操作。
- 用 Provider binary fixture 直接覆盖首消息前未 materialize 响应，断言 `conversation.create` 成功且 `conversation.get` 返回空 items；错误 code、thread id 或 message 近似但不相同时不得被吞掉。
- 连续两次 acquire 只允许一次 `thread/resume`，两次都返回权威 selection 与租期；租约有效时 terminal 后的新 turn 必须复用同一 PID。
- 覆盖首次 acquire 未完成/失败时 Remote composer 不可用，成功后才开放；acquire selection 覆盖 snapshot 默认，但周期续租不覆盖用户修改。
- 覆盖 waiting approval、waiting user input、停止续租后的空闲过期、active turn 跨过期、terminal notification、两 conversation 隔离与并发首次 acquire/send。有效租约场景覆盖：`turn/start` 初始 items 为空、随后收到 user item completed、模型输出跨过至少一个 reaper 周期，writer 在 terminal event 发布前仍存活；不要用已过期且未续租的测试租约替代 Remote 的 interaction lease 责任。
- 覆盖 execution initialize failure、resume conflict/悬挂、RPC reject、async crash、sent-outcome-unknown、instance stop 与 provider shutdown；每条失败路径都必须无 map/pending/process 残留。
- 用真实 Provider binary 悬挂 16 个普通 write，断言保留通路的 `instance.stop` 仍在 2 秒内返回；填满 16+32 后验证第 49 个按 id 过载且不执行，并在相同饱和度下用 EOF 验证有界回收。
- 用确定性 handle barrier 覆盖“请求预取 Ready、terminal 先 Closing”，断言请求转到新 generation；用 resume/cancel barrier 覆盖 cancel 先线性化且 fixture 无 resume/start。
- 运行 Codex Provider all-targets/vertical、相关 Host/Gateway、Remote detail/gateway tests、`npm run protocol:check` 与 `git diff --check`，确认 Provider/Gateway schema 和 generated SDK 零漂移。
