# Codex Provider writer 必须与 active turn 同生命周期

## 规则

Codex Provider 的长期 App Server 必须是纯读 observer。任何可能取得 thread writer 的 `thread/start`、`thread/resume`、turn 或 approval 操作都不得进入 observer；历史 conversation 的 writer 只能由按 conversation 隔离的执行 App Server 持有，并在 active turn 终态或执行失败后立即通过进程退出释放。

执行会话的 owner 是 conversation/active turn，不是 Remote 页面、WSS connection、event subscriber 或某个请求 future。`running`、`waiting-approval`、`waiting-user-input` 都必须保留执行会话；completed、failed、interrupted、App Server terminal fault、initialize/resume 失败、明确 RPC reject、sent-outcome-unknown、instance stop 与 provider shutdown 都必须移除映射并关闭子进程。

## 适用场景

- 修改 `codepet-provider-codex` 的 instance、conversation、turn、approval 或 App Server process control。
- 增加新的写操作、terminal notification、权威 snapshot reconciliation 或 Provider restart 路径。
- 修改 Remote/WSS 生命周期、event subscription 或断线恢复，并可能把网络连接状态传入 Provider。
- 修改 App Server 请求重试、超时、writer conflict 或 `clientRequestId` 语义。

## 反例

- 让 observer 在第一次 `turn.start` 时 `thread/resume`，随后一直持有所有访问过的 thread writer。
- 手机离开详情页或 WSS 断开就关闭仍在 running/waiting 的 turn，或反过来因页面仍开着而在 turn completed 后继续保留进程。
- 每个请求临时 spawn 一个进程，使 steer、interrupt 或 approval 落到不同 generation。
- 同 conversation 并发首次 send 各自 spawn/resume，产生双 writer 竞争；或在 `turn/start` 结果未知时自动重发。
- A 的 terminal event 清空 Provider instance 的全部执行会话，误杀仍 active 的 B。

## 推荐做法

- instance start 只创建 observer；list/search/get/model 全部走 observer，conversation.create 走完成即关闭的一次性会话。
- 用 conversation-keyed 创建槽合并首次 spawn/resume，并用 per-slot operation lock 串行化 start/steer/interrupt/approval。事件线程只持 runtime 弱引用。
- turn started/权威 in-progress snapshot 记录 active turn；terminal notification 先发布事件，再按 conversation + generation 精确释放。steer/interrupt 后的权威 terminal snapshot 使用同一路径。
- 创建中的 session 一旦 spawn 完成就登记到槽，确保 stop 能中断 pending resume；关闭时先从 map 删除，再 shutdown/kill/wait，清理 pending approval，并唤醒创建等待者。
- 只对明确证明由其他 runtime 持有 writer 的 `thread/resume` reject 返回 `conversation_write_conflict`，details 固定为 `operation=thread/resume`、`reason=owned-by-other-runtime`；不得泄露原生 owner 身份。
- 保留 `NotSent`、`ExplicitRpcReject`、`SentOutcomeUnknown` 的区别。未知发送结果关闭会话并返回错误，不在 Provider 内重试 `turn/start`，继续原样传递 client request id。

## 来源

- `../10-architecture/codex-provider-plugin-runtime.md`
- `../30-domains/agent-control/codex-app-server.md`
- 阶段一 `conversation.get` 纯读修复与阶段二 turn-scoped writer 生命周期实现。

## 验证方式

- 清空 fixture 进程日志后重复 `conversation.get`，必须只有 observer `thread/read`，没有新 process 或 `thread/resume`。
- 记录 App Server pid：同一 active turn 的 resume/start/steer/interrupt/approval 必须同 pid；terminal 后下一次写必须是新 pid。
- 覆盖 waiting approval、waiting user input、无 Remote 生命周期调用的断开间隔、terminal notification、terminal snapshot、两 conversation 隔离与并发首次 send。
- 覆盖 execution initialize failure、resume conflict/悬挂、RPC reject、async crash、sent-outcome-unknown、instance stop 与 provider shutdown；每条失败路径都必须无 map/pending/process 残留。
- 运行 Codex Provider all-targets/vertical、真实 CLI 可行 smoke、相关 Host/Gateway、`npm run protocol:check` 与 `git diff --check`，确认 Provider/Gateway schema 和 generated SDK 零漂移。
