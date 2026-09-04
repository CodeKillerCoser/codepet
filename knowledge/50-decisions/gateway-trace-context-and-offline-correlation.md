# Gateway Trace Context 与离线关联

## 背景

Remote 手机端的慢响应可能出现在 UI 触发、Gateway RPC、Host/Provider 执行、事件回传、状态投影或首帧渲染中的任一阶段。当前没有集中遥测平台，排查输入通常是 Remote 与 Host 导出的若干日志。Gateway 的 `turn.send` 又是 fire-and-forget：RPC 返回只表示受理，后续输出通过独立 Stream 到达，不能依赖一次协程或 Dart Zone 一直存活。

## 决策

Gateway v1 envelope 使用可选 `meta: TraceContext`，格式采用 W3C `traceparent` 与可选 `tracestate`。业务 request、response 与 event payload 不加入 trace 字段，SDK 负责同步 RPC 的自动注入和解析。

Remote 在一次用户动作内用 Dart Zone 保存当前短生命周期 context。每个 RPC 创建子 span；Zone 只覆盖当前 async 调用或单次事件处理，不覆盖长生命周期 Stream，也不作为跨请求的全局状态。

`turn.send` 后的异步事件使用因果链接：Host 在单个已认证 WebSocket session 内，以完整 routed conversation/turn identity 关联该请求的不可变 TraceContext。turn id 可用后从 conversation pending 映射迁移到 turn 映射，事件发送时复制到 envelope，turn 进入 completed、failed 或 interrupted 后清理。这个链接表示“由哪个用户动作导致”，不表示 RPC span 一直处于 active 状态。

新 Remote 先执行不带 `meta` 的 `protocol.handshake`，再以同样不带 `meta` 的 `protocol.describe` 探测 `trace-context-v1`。Host 明确声明后才启用 wire 注入；旧 Host 返回 method-not-found 时 Remote 继续工作但只保留本地 trace。这样握手响应保持旧形状，旧 Remote 也能连接新 Host。

两端输出 `codepet.trace.v1` JSONL。记录只包含 trace/span id、时间、阶段名、duration、RPC/event 名和 routed id 等诊断属性，不记录 prompt、delta 正文、credential 或 bearer。高频 `turn.outputDelta` 仅记录每个 turn 的第一条接收事件；首屏投影和 post-frame 渲染单独记录。

## 备选方案

- 让 `turn.send` span 或 Zone 一直存活到 Stream 结束：看似能形成连续父子关系，但生命周期跨多个并发请求和重连，容易泄漏、串链，取消语义也不清晰，因此不采用。
- 保存进程级“最后一个 trace id”：无法区分并发 conversation/turn，会产生错误关联，禁止采用。
- 在每个业务 DTO 中显式增加 trace id：侵入领域模型和所有调用方，也会把传输诊断变成业务契约，因此不采用。
- 只依赖 event cursor 或时间窗口离线猜测：重放、并发和跨设备时钟偏差会导致误配，只能作为缺失 trace 时的辅助证据。
- 立即引入集中 collector：查询体验更好，但增加部署、隐私和可用性成本；当前先以可导出的 JSONL 满足实际排查。

## 取舍理由

可选 envelope 元数据让 SDK 自动化覆盖绝大多数调用，对业务层影响限于边界适配。Zone 的隔离语义保证并发 async 分支不会互相覆盖；Stream 关联使用显式 routed key，避免依赖执行上下文偶然传播。session-local 映射不会跨客户端复用，终态清理限制了内存生命周期。

离线日志用 trace id 合并后可以看到 `ui.turn.send`、`gateway.rpc.client`、`gateway.rpc.received/completed`、首个 `gateway.event.received`、`mobile.timeline.projected` 和 `mobile.first_output.rendered`。多设备绝对时间仍依赖系统时钟；分析时应优先用同进程 duration，并用同一 RPC 的 client/server 边界估计时钟偏差，不把负网络耗时直接解释为真实性能结论。

## 影响范围

- `protocol/core/v1` 与 `protocol/gateway/v1`：定义 TraceContext、envelope 字段和 feature discovery。
- `tools/protocol-codegen` 与签入 SDK：生成 trace-aware codec 和客户端 instrumentation hook，保留原有无 trace API。
- `crates/codepet-host`：记录 RPC 边界并维护 session-local turn 因果映射。
- `codepet-remote/lib/diagnostics`：生成 Zone-local context 和 JSONL span/event。
- `codepet-remote` Gateway/application/UI：在边界携带观测信息并记录首输出投影、渲染阶段；领域业务事件不增加 trace 字段。
- `codepet-remote/tool/trace_analyzer.dart`：读取多个 JSONL 或诊断 zip，按 trace id 合并时间线。

## 后续观察

- 观察本地日志写入对低端手机 IO 的影响；若仍明显，应改为批量 flush，而不是提高 delta 采样率。
- 观察 turn 终态之后是否仍存在有意义的尾随事件；若存在，需要把清理点调整到明确的 stream terminal 语义。
- 离线分析若频繁受设备时钟偏差影响，再增加握手时钟样本与偏差估计，不应修改业务时间戳。
- 当前 trace 只跨 Remote 与 Host Gateway；Provider 子进程若需要细分执行耗时，应沿 Provider 协议继续传递同一标准 context，而不是从日志文本猜测。
