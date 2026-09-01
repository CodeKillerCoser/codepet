# Codex Provider writer 必须与 active turn 同生命周期

## 规则

Codex Provider 的长期 App Server 必须是纯读 observer。任何可能取得 thread writer 的 `thread/start`、`thread/resume`、turn 或 approval 操作都不得进入 observer；历史 conversation 的 writer 只能由按 conversation 隔离的执行 App Server 持有，并在 active turn 终态或执行失败后立即通过进程退出释放。

执行会话的 owner 是 conversation/active turn，不是 Remote 页面、WSS connection、event subscriber 或某个请求 future。`running`、`waiting-approval`、`waiting-user-input` 都必须保留执行会话；completed、failed、interrupted、App Server terminal fault、initialize/resume 失败、明确 RPC reject、sent-outcome-unknown、instance stop 与 provider shutdown 都必须移除映射并关闭子进程。

Host stdio request 不能逐条 await：普通请求与 lifecycle control 必须使用分别有界的 dispatch/queue，control 有保留容量；普通队列满不能阻塞 reader，而要按原 id 明确拒绝，使 EOF/fatal 仍可见。conversation 内部仍由 execution slot operation lock 串行。

## 适用场景

- 修改 `codepet-provider-codex` 的 instance、conversation、turn、approval 或 App Server process control。
- 增加新的写操作、terminal notification、权威 snapshot reconciliation 或 Provider restart 路径。
- 修改 Remote/WSS 生命周期、event subscription 或断线恢复，并可能把网络连接状态传入 Provider。
- 修改 App Server 请求重试、超时、writer conflict 或 `clientRequestId` 语义。
- 修改 Provider binary stdio reader/dispatcher、response writer、EOF/fatal cleanup 或并发上限。

## 反例

- 让 observer 在第一次 `turn.start` 时 `thread/resume`，随后一直持有所有访问过的 thread writer。
- 手机离开详情页或 WSS 断开就关闭仍在 running/waiting 的 turn，或反过来因页面仍开着而在 turn completed 后继续保留进程。
- 每个请求临时 spawn 一个进程，使 steer、interrupt 或 approval 落到不同 generation。
- 同 conversation 并发首次 send 各自 spawn/resume，产生双 writer 竞争；或在 `turn/start` 结果未知时自动重发。
- A 的 terminal event 清空 Provider instance 的全部执行会话，误杀仍 active 的 B。
- Provider stdio 主循环逐条 await，导致 A 的 resume 悬挂同时阻塞 B 和 `instance.stop`；或无界 spawn Host request。
- terminal event 发布期间仍把 Ready 旧槽交给新请求，随后关闭该 session，使新请求误报 unavailable。
- 请求在拿 operation lock 前预取 Ready handle，拿锁后不复核 generation；或 cancel 与 resume write 之间没有共同线性化点。

## 推荐做法

- instance start 只创建 observer；list/search/get/model 全部走 observer，conversation.create 走完成即关闭的一次性会话。
- 用 conversation-keyed 创建槽合并首次 spawn/resume，并用 per-slot operation lock 串行化 start/steer/interrupt/approval。事件线程只持 runtime 弱引用。
- turn started/权威 in-progress snapshot 记录 active turn；terminal notification 先在 operation lock 内把 Ready 槽切成 Closing，再发布事件、shutdown/kill/wait、按 conversation + generation 删除同一槽并唤醒等待者。所有 writer route 必须通过单一 helper 在拿 operation lock 后复核当前 Ready generation；旧 handle 只可重试，不可写。
- Creating 槽从插入 map 起携带 attempt generation/cancellation；子进程完成 spawn 后、initialize 前立即登记。cancel 与第一条 `thread/resume` request 写入用短锁线性化，锁只包含最终复核与 frame 写入，不能包含悬挂 response 等待；cancel 先发生后不得再写 resume/start。
- 只对官方 active-writer 形状的 `thread/resume` reject 返回 `conversation_write_conflict`：`code=-32600`、`data` 缺失/null、message 精确匹配当前 conversation id。details 固定为 `operation=thread/resume`、`reason=owned-by-other-runtime`；wrong code、近似 message、其他 id 或非空 data 不得猜测为冲突。
- 保留 `NotSent`、`ExplicitRpcReject`、`SentOutcomeUnknown` 的区别。未知发送结果关闭会话并返回错误，不在 Provider 内重试 `turn/start`，继续原样传递 client request id。
- stdio reader 与 dispatch 分离：普通请求 16 active + 32 pending，lifecycle control 2 active + 4 pending；入队必须非阻塞，过载按原 id 返回 retryable `provider_overloaded`，被拒请求不得执行。event/response 只在共享 writer mutex 内串行；EOF/fatal 先关闭 Provider/App Server，再有界 drain/abort。

## 来源

- `../10-architecture/codex-provider-plugin-runtime.md`
- `../30-domains/agent-control/codex-app-server.md`
- 阶段一 `conversation.get` 纯读修复与阶段二 turn-scoped writer 生命周期实现。

## 验证方式

- 清空 fixture 进程日志后重复 `conversation.get`，必须只有 observer `thread/read`，没有新 process 或 `thread/resume`。
- 记录 App Server pid：同一 active turn 的 resume/start/steer/interrupt/approval 必须同 pid；terminal 后下一次写必须是新 pid。
- 覆盖 waiting approval、waiting user input、无 Remote 生命周期调用的断开间隔、terminal notification、terminal snapshot、两 conversation 隔离与并发首次 send。
- 覆盖 execution initialize failure、resume conflict/悬挂、RPC reject、async crash、sent-outcome-unknown、instance stop 与 provider shutdown；每条失败路径都必须无 map/pending/process 残留。
- 用真实 Provider binary 悬挂 16 个普通 write，断言保留通路的 `instance.stop` 仍在 2 秒内返回；填满 16+32 后验证第 49 个按 id 过载且不执行，并在相同饱和度下用 EOF 验证有界回收。
- 用确定性 handle barrier 覆盖“请求预取 Ready、terminal 先 Closing”，断言请求转到新 generation；用 resume/cancel barrier 覆盖 cancel 先线性化且 fixture 无 resume/start。fixture 使用每 PID 独立日志，并至少覆盖超过历史失败轮次的重复终态压力测试。
- 运行 Codex Provider all-targets/vertical、真实 CLI 可行 smoke、相关 Host/Gateway、`npm run protocol:check` 与 `git diff --check`，确认 Provider/Gateway schema 和 generated SDK 零漂移。
