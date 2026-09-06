# Provider stdio 跨进程隔离与 Host 调度延迟

> 历史基线（2026-09-06，移除旧 STDIO 运行时之前）。以下数字及旧命令仅记录当时实验；依赖整帧边界的 `stdio_contention` 与 burst/isolation 驱动已随旧运行时退役。原始 reports 保留。当前 mux 的可执行测试入口见 [mux 验证](provider-stdio-mux-validation.md)。


## 现象

[并发大消息实验](provider-stdio-burst-contention.md) 证明同一 Provider 内存在延迟累积。本轮继续区分两个问题：A 的管道背压是否占住另一个 Provider B 的管道，以及 A 的工作是否通过共享 Host runtime 影响 B。

## 复现路径

```sh
cargo build --release --manifest-path crates/Cargo.toml -p codepet-host --example stdio_contention
python3 scripts/benchmark_stdio_isolation.py
```

单场景：

```sh
BENCH_HOST_WORKERS=1 BENCH_PROVIDER_WORKERS=4 crates/target/release/examples/stdio_contention /tmp/isolation.json 1 --isolation response 67108864 repeat 8 0
```

环境为 2026-09-06、Apple M5 Pro、48 GiB、macOS arm64、release；生产源码基线 `71b48bf81d0f50f80b0027f8d4e1902f1fc0909c`。仅改变基准 Host 的 Tokio worker 数（1/4），两个 fixture Provider 均保持 4。worker 数不是弱 CPU 性能模拟，也不表示已查明安装版应用使用几个 worker。

每个场景创建同一 Host runtime 内的两套 `PluginProcess`，各自拥有独立子进程、stdin/stdout 和请求状态；断言 PID 不同。fixture 描述符相同，但进程和连接真实独立，不经过 PluginManager 的插件注册/路由。

A 同时发起 8 个大 RPC：8 MiB 高熵请求/响应（正常、4 MiB/s），或 64 MiB 重复文本请求/响应（正常）。B 始终执行小 project.list 和 provider.ping，无主动限速。A 的故障注入与上一轮相同，每个大帧都按设置限速；不使用真实 Harness 或用户历史。

对 B 先做 8 组无负载 RPC 基线，再从 burst 开始后 1 ms 安排第一组探针；正常场景后续间隔 5 ms、背压场景间隔 250 ms。跳过错过的时间槽，不制造探针积压。每个探针分别记录：

- `schedulerLagMs`：实际开始执行相对计划时间的延迟；可以发现请求尚未发起就已被 Host 调度阻塞的情况。
- `small.elapsedMs`、`ping.elapsedMs`：实际开始后到 API 返回的时间，包含共享 Host 的响应处理。
- `smallDueToCompletionMs`：同一探针的上述两项之和。不能把不同探针的两个最大值直接相加。

探针覆盖 A 的整个传输过程，A 的 RPC 即使超时，也继续等 8 个帧传完。摘要仅纳入计划发起时间不晚于 drain 的探针；被延误到 drain 之后才执行的探针仍计入。计时器粒度和操作系统调度本身会带来约毫秒级误差，不能把所有小延迟都归因于 A。探针是 Host 内的定时任务，不是外部客户端到达时刻。

## 证据

- [36 个场景原始记录](../../reports/stdio-isolation-2026-09-06.json)
- [分组中位数与实测最大值](../../reports/stdio-isolation-2026-09-06-summary.json)
- [双进程场景](../../crates/codepet-host/examples/support/stdio_isolation.rs)
- [运行与摘要脚本](../../scripts/benchmark_stdio_isolation.py)

12 组×3 次，A 共 288 个大 RPC。B 共 944 组定时探针，其中 901 组计划在 A drain 前发起；每组均包含小 RPC 与心跳，另有 288 组基线。所有 B 请求和心跳成功，没有超时。

B 从计划发起到小 RPC 完成的最长实测时间（正常、未限速）：

| A 的负载 | Host 1 worker | Host 4 workers |
| --- | --- | --- |
| 8×8 MiB 高熵请求 | 60.11 ms | 19.45 ms |
| 8×8 MiB 高熵响应 | 9.02 ms | 8.12 ms |
| 8×64 MiB 重复文本请求 | 200.56 ms | 98.61 ms |
| 8×64 MiB 重复文本响应 | 127.32 ms | 3.14 ms |

这是本轮最大值，不是 p99、上限保证或不同平台的延迟承诺。每组只有 3 个独立场景，探针共享同一次突发，不能将所有探针当作独立重复实验。

两个主要表现：

1. 大请求造成发起前延迟。64 MiB 重复文本请求场景，1/4 worker 的最大 `schedulerLagMs` 分别为 200.14/98.12 ms，而已开始的 B 小 RPC 最大仅 0.44/0.49 ms。只看 RPC RTT 会漏掉主要等待。
2. 大响应也可能阻塞已发起请求的处理。64 MiB 重复文本响应场景，1 worker 的 B 小 RPC 耗时最大 108.00 ms，4 workers 最大 0.79 ms；同一探针的计划到完成最大分别为 127.32/3.14 ms。这与 Host 同步 decode 占用共享执行线程的源码行为一致，本轮没有逐函数 CPU profile 来分摊耗时。

背压隔离：A 的 8 个 8 MiB 高熵消息在 4 MiB/s 下需约 13 秒传完，各场景均有 2 个 A RPC 超时，总计 24/288。B 未跟随等待整批数据：分组小 RPC 耗时中位数约 0.63–0.65 ms；最慢观察到 9.40 ms（A 请求、4 workers），其余背压组最大约 1.3–1.5 ms。计划发起后的总时间仍可能包括 A 请求编码期间的约 22/59 ms 调度延迟。全部 A 传完后恢复心跳成功。

## 初步影响范围

- `crates/codepet-host/src/providers/process.rs`：每个进程有独立 writer queue 和 RPC 状态，解释管道背压没有变成全局同一把写锁；request 编码和 response decode 在共享 Tokio runtime 内执行，解释为何进程隔离不等于调度隔离。
- `sdk/rust/codepet-provider-sdk/src/frame.rs`：整体编码、压缩、解压和 JSON 解析的 CPU 成本依赖原始数据，不仅依赖编码后字节数。
- `examples/stdio_contention.rs`：仅给基准增加 PID 标签和可配置 worker 数，默认仍为 4；没有更改生产 runtime 配置。
- `examples/support/stdio_isolation.rs`：新增双进程定时探针，区分调度延迟与发起后 RTT，并按 PID 校验 A 的帧完成数。
- `scripts/benchmark_stdio_isolation.py`：独立进程重复运行、保存完整样本和可重算摘要。

## 当前判断

不是所有 Provider 共用一个串行通道。独立进程/管道能隔离整帧写入背压，但共享 Host CPU/runtime 仍可造成跨 Provider 延迟。已排除“B 必须等 A 的全部帧写完才能收发”作为这些场景的一般解释；未排除操作系统调度对部分波动的影响。引入历史未核实。

因此分片、公平发送和控制消息优先级主要解决同一通道内的问题；不能承诺解决本轮观察到的全部跨进程延迟。后续应评估把耗时的编码和解码放入有界后台执行，配合编码前的字节预算，再重复相同场景验证 B 的调度延迟、RTT、内存和 A 的吞吐。不能用无界 `spawn_blocking` 或仅增加 worker 数代替资源控制。

这仍是实验支持的改进方向，尚未实现优化对照组。

## 验证结果

36/36 场景完成，PID 隔离、每场景 8 个大帧完成数、B 全部成功、A 错误仅为预期超时均校验通过。`python3 -m py_compile scripts/benchmark_stdio_isolation.py` 通过。旧 burst 模式另回归正常请求和限速响应，确认默认 4 workers 与既有背压路径仍可运行。

## 未知项

- 未测真实 Harness、外部 Remote 客户端到达时间、PluginManager 自动失联策略、其他系统或长期持续负载。
- 无逐函数 CPU profiling；无法把全部调度延迟精确划分给序列化、压缩、内存分配、解析或 runtime 调度。
- 无优化后的 A/B 对照；增加 worker 数的改善不代表根因已经修复。
