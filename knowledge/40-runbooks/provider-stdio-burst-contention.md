# Provider stdio 并发大消息与内存峰值基线

> 历史基线（2026-09-06，移除旧 STDIO 运行时之前）。以下数字及旧命令仅记录当时实验；依赖整帧边界的 `stdio_contention` 与 burst/isolation 驱动已随旧运行时退役。原始 reports 保留。当前 mux 的可执行测试入口见 [mux 验证](provider-stdio-mux-validation.md)。


## 现象

[单帧实验](provider-stdio-contention.md) 已确认大消息会延迟小 RPC，但不能代表多个消息同时到达时的等待和内存占用。本轮继续测有限突发负载，不修改生产 Frame V1、并发数、超时或消息路由。

## 复现路径

```sh
cargo build --release --manifest-path crates/Cargo.toml -p codepet-host --example stdio_contention
python3 scripts/benchmark_stdio_bursts.py
```

单个场景也可运行（参数依次为方向、原始字符串字节数、内容类型、并发数、每秒字节限速；0 表示不限速）：

```sh
crates/target/release/examples/stdio_contention /tmp/burst.json 1 --burst request 8388608 entropy 8 4194304
```

环境沿用 2026-09-06 单帧基线：Apple M5 Pro、48 GiB、macOS arm64、release，Host 与 Provider 各 4 个 Tokio worker，生产源码基线 `71b48bf81d0f50f80b0027f8d4e1902f1fc0909c`。每组 3 次，共 18 组、54 个独立 Host 进程，避免上一场景污染 RSS 历史峰值。

- 每个 RPC 使用真实 Host `PluginProcess` 与 Provider SDK；fixture 业务及数据类型与单帧实验相同。
- 预先生成各调用者的输入，再以 barrier 同时释放 1/4/8/16 个大 RPC。Host 观察到首帧 I/O 开始后提交一个小 project.list 和一个 provider.ping。
- 小 RPC 时间从探针提交前开始计，包含调度、编码、等待、传输和解析。额外记录从整个突发开始到小 RPC 结束的时间；前置编码/运行时繁忙可能延迟探针本身被提交。
- 各大消息真正入队顺序由运行时决定；探针可能在部分大消息之前入队，不能声称它一定排在全部 N 个消息之后。
- 4 MiB/s 仍为人工背压：请求侧控制读取，响应侧控制持锁写入；本轮对每个超过 32 KiB 的大帧限速，不只处理第一帧。
- 内存使用 `getrusage` 的本进程与已回收子进程 RSS 高水位，macOS 返回字节。每个场景只有一个 Provider 子进程；峰值包含 fixture、调用方原始输入、序列化对象、runtime 与分配器，不能当作纯 transport 缓冲或泄漏。Host/Provider 的各自峰值不保证同时发生，不直接相加声称同时总峰值。

## 证据

- [54 次原始聚合记录](../../reports/stdio-bursts-2026-09-06.json)
- [分组中位数、范围、超时数与内存峰值](../../reports/stdio-bursts-2026-09-06-summary.json)
- [运行器与摘要逻辑](../../scripts/benchmark_stdio_bursts.py)
- [并发场景实现](../../crates/codepet-host/examples/support/stdio_burst.rs)

8 MiB 高熵字符串的实际单帧约 6.60 MiB。以下为小 RPC 延迟中位数，每组 3 次：

| 并发大消息数 | 大请求，正常 | 大响应，正常 | 大请求，4 MiB/s | 大响应，4 MiB/s |
| --- | --- | --- | --- | --- |
| 1 | 10.0 ms | 9.9 ms | 1.665 s | 1.665 s |
| 4 | 27.3 ms | 35.2 ms | 4.992 s | 6.616 s |
| 8 | 46.0 ms | 52.9 ms | 8.312 s | 10 秒超时，3/3 |
| 16 | 124.8 ms | 131.5 ms | 未测 | 未测 |

正常场景的 8 个 1 KiB 消息基线，小 RPC 约 1.8/1.5 ms。16 个大消息正常突发，从 burst 开始到小 RPC 完成约 162.7/142.3 ms；不能仅凭探针后的 124.8/131.5 ms 忽略发起前的延迟。

心跳并不总与普通 RPC 同时完成：正常的 16 个大响应场景，心跳中位数约 27.1 ms，小 RPC 为 131.5 ms。8 个大响应限速场景，小 RPC 全部超时，心跳仍在 1.664–3.318 秒成功。控制通路绕过业务排队确实有收益，但不能保证即时响应或隔离共享 writer。

每个方向的 8 个大消息限速实验均有每轮 2 个大 RPC 超时，共 12/348 个大 RPC 超时；普通小 RPC 共 3/54 个超时；54 个首次心跳均成功。全部 burst 传完后再发恢复心跳，54/54 成功，最大约 0.36 ms；未观察到进程崩溃。仍不涵盖 PluginManager 自动心跳监管的应用级恢复策略。

RSS 峰值中位数：

| 场景 | Host | Provider |
| --- | --- | --- |
| 8×1 KiB 基线 | 约 4 MiB | 约 4 MiB |
| 1×8 MiB 大请求 | 34.9 MiB | 29.6 MiB |
| 16×8 MiB 大请求 | 313.0 MiB | 150.3 MiB |
| 1×8 MiB 大响应 | 37.8 MiB | 42.8 MiB |
| 16×8 MiB 大响应 | 158.7 MiB | 168.0 MiB |
| 8×64 MiB 重复文本请求 | 1285.4 MiB | 583.1 MiB |
| 8×64 MiB 重复文本响应 | 680.5 MiB | 1093.3 MiB |

64 MiB 重复文本每条编码后仅约 2.2 KiB，8 条实际 wire 合计不到 18 KiB，仍出现 GiB 级进程 RSS。请求方向的 Host 持有的原始调用输入本身就有 512 MiB。该场景不能解释为“18 KiB 管道数据占用 1 GiB”；它说明原始对象、复制、编码和解析成本不能由压缩后的帧大小代表。

## 初步影响范围

- `crates/codepet-host/src/providers/process.rs`：请求先形成完整编码帧再进入有界 writer 队列；按条数有界不等于按字节有界。reader 仍顺序接收并解析完整消息。
- `sdk/rust/codepet-provider-sdk/src/stdio.rs`：response 编码及同步输出锁可能占用 Tokio worker，控制请求和普通请求完成顺序不同。该机制由源码支持；本轮未单独测量线程饥饿耗时。
- `sdk/rust/codepet-provider-sdk/src/frame.rs`：16 MiB 编码后上限允许极大的高压缩率 JSON，通过大小校验不代表解码内存低。
- `examples/stdio_contention.rs` 与新增 `examples/support/stdio_burst.rs`：仅扩展基准模式，增加所有大帧限速、时间标记、独立进程内存高水位采集。
- `scripts/benchmark_stdio_bursts.py`：独立进程重复测量与可重算摘要；不采集载荷正文。

## 当前判断

后续 [跨进程隔离实验](provider-stdio-process-isolation.md) 已区分同一管道排队与共享 Host runtime 干扰，不能把本轮结果泛化为所有 Provider 都共用同一个串行通道。

并发突发确实放大了单帧实验观察到的影响：本机正常情况下可达到百毫秒，背压下可发生合法小请求超时。高压缩率数据还暴露出独立的内存放大问题。引入历史未核实。

优先评估按字节的入场预算、减少大对象复制及将编码/解码与 reader/控制处理解耦，预算需要在大规模编码/复制前发挥作用。若目标工作负载经常并发传输大帧且要求小 RPC 在背压下保持低延迟，再实现分片公平调度。只增加并发数或只按压缩帧大小限流，不能解决这些结果；只有分片也无法解决大 JSON 的 CPU/内存成本。

这是基于合成负载的后续方向，不是已实现优化，也不设定未经确认的产品延迟 SLO。

## 验证结果与下一步

54 个场景全部跑完；每组 3 次。基准断言正常场景无 RPC 错误、故障注入只允许预期的请求超时、所有大帧最终传完、随后心跳恢复。样本数量、分组、逐场景 I/O 帧数、RSS 和 timeout 明细校验通过（348 个大 RPC，12 个预期超时；54 个小 RPC，3 个预期超时；54 个首次心跳成功）。旧单消息模式回归通过：`crates/target/release/examples/stdio_contention /tmp/codepet-stdio-legacy-regression.json 1` 完成 36 个场景及 2 个超限恢复检查。`python3 -m py_compile scripts/benchmark_stdio_bursts.py` 通过。

未来优化至少重复本轮 normal/背压/高压缩率三类场景，对比小 RPC 完成时间、超时数与 RSS 峰值；不能只对比大消息总吞吐。需要持续负载或内存泄漏结论时，另做长时间采样及 drain 后当前 RSS/存活对象检查，不能使用高水位替代。

## 未知项

- 未测真实 Harness/历史、弱 CPU、Windows/Linux、长时间连续负载和超过 16 条的突发。
- RSS 包含所有进程开销且不区分活跃对象、缓存与分配器保留，未证明存在或不存在泄漏。
- 未单独拆分每次锁等待、解压和解析耗时，也未实现 Frame V2 对照组。
