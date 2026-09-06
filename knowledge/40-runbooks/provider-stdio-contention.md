# Provider stdio 大帧阻塞的复现与性能基线

> 历史基线（2026-09-06，移除旧 STDIO 运行时之前）。以下数字及旧命令仅记录当时实验；依赖整帧边界的 `stdio_contention` 与 burst/isolation 驱动已随旧运行时退役。原始 reports 保留。当前 mux 的可执行测试入口见 [mux 验证](provider-stdio-mux-validation.md)。


## 现象

2026-09-06 检查 Host↔Provider Frame V1：合法的大请求、大响应会使同向小消息等待。需要区分请求执行并发、整帧传输、编码/解码 CPU 和人为背压，不能仅从“存在 writer 锁”推导必须升级 stream。

## 复现路径

基准使用真实 `PluginProcess`、`ProviderFrameCodec`、`serve_stdio_with_io` 和默认 dispatcher。单个测试程序以子进程模式充当 Provider；业务 fixture 用 initialize.hostVersion 承载大请求、describe.plugin.displayName 承载大响应，均为合法协议字段。它不调用实际 Harness 或模型，不读取用户历史。

```sh
cargo build --release --manifest-path crates/Cargo.toml -p codepet-host --example stdio_contention
crates/target/release/examples/stdio_contention reports/stdio-contention-2026-09-06.json 5
python3 scripts/summarize_stdio_contention.py reports/stdio-contention-2026-09-06.json
crates/target/release/examples/stdio_contention reports/stdio-contention-deadlines-2026-09-06.json 1 --deadlines
cargo test --release --manifest-path crates/Cargo.toml -p codepet-host --test process_rpc real_stdio_lifecycle_correlates_concurrent_responses_and_separates_events -- --exact
```

环境：macOS Darwin 25.3.0 arm64，Apple M5 Pro，48 GiB 内存，rustc 1.95.0，release 优化；Host 和 fixture 各 4 个 Tokio worker。生产源码基线 `71b48bf81d0f50f80b0027f8d4e1902f1fc0909c`。基准并非空闲独占机器，结果不是所有平台的延迟承诺。

每个场景独立启动并初始化 Provider。大帧开始 I/O 时，经 stderr 标记触发同一 Host 连接上的 project.list、provider.ping；project.list 还同步发布一个小事件。计时从 Host 收到开始标记到对应结果到达，包含任务调度、剩余传输、解码和业务分发。标记异步传播，对很小的帧不能保证探针发出时物理写入仍未结束；不把这个值称为纯锁等待。

载荷为重复 `x` 和固定种子的高熵可打印 ASCII，覆盖 1 KiB、1/8/16 MiB；重复文本另测 64 MiB。180 次进程场景（36 组×5 次）、94 次独立 codec 测量，另有请求/响应各一次超限和各一次超时实验。小样本报告中位数与范围，不估算 p95/p99。

限速为明确的故障注入：请求侧限制 fixture stdin 的有效读速；响应侧在 SDK 持有输出锁期间按 16 KiB 小块节拍写出，模拟慢输出。正常场景不主动限速。4 MiB/s、1 MiB/s 是人工设置，**不是本机 stdio 的测得吞吐量，也不是 Remote 网络实测**。节拍写入仍处于一个完整 Frame V1 写调用内，不允许其他消息插入。

## 证据

原始聚合记录与摘要（均不包含载荷正文）：

- [完整场景与 codec 指标](../../reports/stdio-contention-2026-09-06.json)
- [分组中位数、最小值、最大值](../../reports/stdio-contention-2026-09-06-summary.json)
- [默认 10 秒超时实验](../../reports/stdio-contention-deadlines-2026-09-06.json)

高熵载荷下，小 RPC 延迟中位数：

| 原始字符串大小 | 实际帧大小 | 大请求，正常 | 大响应，正常 | 大请求，4 MiB/s | 大响应，4 MiB/s |
| --- | --- | --- | --- | --- | --- |
| 1 KiB | 约 1.2 KiB | 0.34 ms | 0.28 ms | 0.73 ms | 0.67 ms |
| 1 MiB | 0.826 MiB | 1.56 ms | 1.21 ms | 210 ms | 211 ms |
| 8 MiB | 6.602 MiB | 9.86 ms | 10.68 ms | 1.665 s | 1.665 s |
| 16 MiB | 13.203 MiB | 18.81 ms | 46.65 ms | 3.324 s | 3.325 s |

16 MiB 正常响应场景的小 RPC 范围为 20.14–53.91 ms，正常请求为 18.60–19.24 ms（精确范围以摘要为准）。对应正常心跳中位数 18.93/46.67 ms；4 MiB/s 时 3.324/3.325 s。小事件端到端延迟同样随之增加；事件由小 RPC 生成，不能据此单独量化自主事件的 writer 等待。

64 MiB 重复文本压缩后的完整帧仅约 2.2 KiB，正常小 RPC 延迟仍为请求侧 14.97 ms、响应侧 26.08 ms，I/O wrapper 记录约 0.03/0.02 ms。这说明大 JSON 的接收处理也会拖慢后续消息，分片不能单独消除该开销。

分阶段证据：

- 16 MiB 高熵请求的 frame 读取中位数 2.78 ms；独立 codec 的 JSON 编码/压缩/解码分别约 6.41/9.66/15.17 ms。
- 16 MiB 高熵响应的 frame 写入中位数 14.51 ms；独立 codec 的 JSON 编码/压缩/解码分别约 5.17/11.28/18.49 ms。
- 4 MiB/s 时同一帧 I/O 约 3.302 s，基本解释小 RPC 的约 3.325 s 延迟。
- codec 数据来自另一次同载荷调用，解码含解压和 JSON 解析；不能与进程场景各阶段相加当作精确总耗时。Host writer 队列等待未单独埋点；SDK 自带 write 指标包含获取锁的等待，wrapper 只记录获得写入机会后的 I/O。

24 MiB 高熵载荷压缩后约 19.8 MiB，超过共享的最终帧 16 MiB 上限：请求返回 `provider_frame_too_large`，响应返回 `provider_response_too_large`。后续心跳分别 0.31/0.15 ms 成功，进程未因该拒绝退出。64 MiB 重复文本则可以通过，证明当前上限作用于编码后的帧，而不是解压后的 JSON。

1 MiB/s 实验：合法的 16 MiB 高熵载荷需约 13.2 秒传输。请求方向和响应方向都使大 RPC、小 RPC、心跳在约 10 秒返回 `provider_request_timeout`。从探针开始等待到第 15 秒再发心跳，分别 0.51/0.41 ms 成功。超时移除请求等待者，没有中断已在传输的整帧；此次未经过 PluginManager 的自动心跳监管，不能据此断言完整应用会采用相同恢复策略。

## 初步影响范围

- `crates/codepet-host/src/providers/process.rs`：请求 writer 顺序写完整帧；response reader 读完整帧并同步 decode 后才处理下一条，是两个方向的关键阻塞点。
- `sdk/rust/codepet-provider-sdk/src/stdio.rs`：请求 reader 先完整读取/解析，response/event/pong 共用输出锁；业务/control dispatcher 并发并不隔离这两处等待。
- `sdk/rust/codepet-provider-sdk/src/frame.rs`：整消息编码与压缩、完整帧解码、16 MiB 编码后上限决定传输和 CPU 成本。
- `crates/codepet-host/examples/stdio_contention.rs`、`scripts/summarize_stdio_contention.py`：新增的可重复实验和摘要工具；没有修改生产 transport 或协议。

## 当前判断

已复现整帧导致的同向队头阻塞；并非所有业务请求被配置为串行。正常本机单个近上限帧主要带来毫秒到几十毫秒延迟，背压可放大到秒级并使其他合法请求超时。引入历史未核实。

单帧证据本身不足以决定立即实现完整 stream 协议；后续 [并发突发与内存实验](provider-stdio-burst-contention.md) 已补充正常场景百毫秒延迟及 GiB 级 RSS 的证据。应先对真实工作负载检查大帧频率、实际 writer 等待和解析耗时；分页/按需载荷、将大 decode 从 reader 解耦应分别验证。若正常目标设备或持续负载确有长时间写入等待，再评估分片复用。分片实验必须重复本基准，并同时检查公平性、内存上限和取消清理；否则可能仅转移阻塞。

## 验证结果与下一步排查

主基准 180/180 场景完成，小 RPC、心跳、事件成功；大小边界错误码和后续心跳恢复断言通过；两次背压超时实验的大/小 RPC 与心跳均按预期超时并恢复。已有 `real_stdio_lifecycle_correlates_concurrent_responses_and_separates_events` 测试通过（1 passed）；摘要脚本编译检查、180 条样本分组/成功状态/主帧 trace 校验及两组超时恢复结果校验通过。

本 runbook 保留复现路径，不新增“必须升级 Frame V2”的规约。若以后修改 writer、reader、codec 或控制通路，优先复跑本实验；性能数字需重新测量，不作为毫秒级硬性 CI 断言。

## 未知项

- 未测真实 Harness、真实历史载荷、弱 CPU、Windows/Linux、长期持续负载和全应用端到端交互；最多 16 条同时发起的突发见 [后续实验](provider-stdio-burst-contention.md)。
- 本轮单帧未测 RSS；后续突发实验已采集峰值。当前解码后的 JSON 没有同等 16 MiB 上限，不能把实验成功当作内存安全证明。
- 没有精确分离 Host 队列等待、调度与 JSON 解压/解析；需要进一步的低开销阶段埋点才能精确归因。
- 未测试独立控制管道或 Frame V2，不能量化迁移收益。
