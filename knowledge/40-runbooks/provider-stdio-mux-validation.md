# Provider stdio 复用验证

## 当前状态：默认且仅支持 mux

2026-09-06 按用户要求删除 `runtime/legacy`、Host 整帧 reader/writer、旧 profile 常量与选择分支。未声明 manifest transport、未设置环境变量均默认 mux；旧 profile/冲突配置拒绝。CPRF 仅保留为 stream 内消息正文和独立 codec 工具。

三个 Provider 的 `main.rs` 均调用公共 `serve_stdio`，manifest 均为 mux，initialize 都验证 v1 范围和 Host 身份，协议 DTO/dispatcher 均来自生成 SDK。业务实现不需要各写一份多路复用。独立进程测试现共用 `crates/providers/test_support/mux_stdio.rs`；Codex App Server JSONL、Claude CLI stream-json、OpenCode HTTP/SSE 属于各自上游接口，维持原协议。

当前验证结果：

| 范围 | 结果 |
| --- | --- |
| Provider SDK | 40 项通过（16 unit + 12 protocol + 12 wire） |
| Host | 69 项通过；含启动三个真实 Provider 二进制并连接上游 fixture 的端到端测试 |
| Codex | 97 项通过，1 项真实 CLI smoke 按既有标记跳过 |
| Claude | 17 项通过 |
| OpenCode | 22 项通过，1 项真实 Server smoke 按既有标记跳过 |
| canonical freshness/codegen | 17 项通过 |
| SDK 导出 / App staging | 3 / 2 项通过 |
| 编译后的独立 SDK 生成器 | 仓库外导出与 freshness 通过；导出 SDK 离线编译并通过 16 项 unit test |
| Codex Python smoke 包装器 | 用指定二进制调用 Rust mux 客户端，初始化、实例、会话查询、退出通过 |

命令：

```sh
cargo test --manifest-path sdk/rust/Cargo.toml -p codepet-provider-sdk
cargo test --manifest-path crates/Cargo.toml -p codepet-host -p codepet-provider-codex -p codepet-provider-claude -p codepet-provider-opencode --no-fail-fast
npm run protocol:check
npm run providers:test
bun test tools/cp-sdk-gen/cp-sdk-gen.test.mjs
bun build tools/cp-sdk-gen/cp-sdk-gen.mjs --compile --outfile /tmp/codepet-mux-only-20260906/cp-sdk-gen
python3 scripts/test_codex_provider_stdio.py --provider crates/target/debug/codepet-provider-codex --app-server crates/target/debug/codex-app-server-fixture
cargo run --release --manifest-path crates/Cargo.toml -p codepet-host --example mux_contention -- reports/stdio-mux-only-2026-09-06.json 1
git diff --check
```

复制 canonical protocol 到临时目录后，在仓库外执行编译产物的 `--package provider --role server --lang rust --protocol ./protocol --output ./exported-compiled` 与同参数 `--check`，再执行 `cargo test --offline --manifest-path exported-compiled/Cargo.toml -p codepet-provider-sdk --lib`。实际目录为 `/tmp/codepet-mux-only-20260906`。

本次独立进程测试发现并修复三处共享运行时退出缺陷：

- 输入泵直接 drop `tokio::io::split` 写半端；输出泵还持有读半端，使 duplex 不产生 EOF。改为显式 shutdown 写半端。
- 输出泵写失败只在最后 drain 时才被检查，输入仍开着时不能及时退出。改为在 runtime 主 select 监听物理输出完成/错误。
- incoming 队列关闭可能早于 driver 结果，异常退出被标为成功。SDK 与 Host 均收取 driver 结果；SDK 只收取一次，避免重复 poll 完成的 JoinHandle。

回归使用 stdin 仍开着的输出故障、输入 EOF、Claude 背压坏帧与进程树清理、Codex 49 个饱和请求关闭与异步事件断管、OpenCode 坏帧清理。Host 测试同步修正 PID 文件刚创建但尚未写完的等待竞态；LAN 测试完整消费最后 delta 及其 activity 事件，保留撤销凭据后必须 Close 的断言，避免旧整帧时序留下的事件被误判。

最新 6 个压测场景见 `reports/stdio-mux-only-2026-09-06.json`。4 MiB/s 下，8/16 MiB 双向大正文的小 RPC 为 102.46–103.99 ms；1 MiB/s 下为 387.17–387.80 ms。6 个场景小 RPC、心跳、事件顺序、正常退出均通过，且小 RPC 返回时大 RPC 仍未完成。两条 1 MiB/s 大 RPC 如预期触发默认 10 秒业务 deadline，未拖死小 RPC。以上是受控夹具单次样本，不是生产 SLO。

日志保存在 `/tmp/codepet-mux-only-20260906`。未运行格式化工具。真实 Codex CLI / OpenCode Server 的两个可选 smoke 未启用，未验证 Windows/Linux 原生管道；Codex 既存 `CodexThreadItem` 未使用 import warning 仍存在。

## 历史对照的适用范围

以下数字记录移除旧运行时前的双 profile 实验，原始 reports 保留。当前 `mux_contention` 只运行 mux；下方旧命令不再产生双 profile 对照。依赖整 CPRF 帧边界的 stdio_contention/burst/isolation 夹具已退役，不能用其旧限速假设测量分片后的协议。


## 场景与证据

2026-09-06，在 macOS arm64 / Apple M5 Pro / 48 GiB 上，用真实 `PluginProcess` 和 SDK `serve_stdio_with_io` 比较显式旧 Frame V1 与新 mux。代码见 `crates/codepet-host/examples/mux_contention.rs`，原始输出见 `reports/stdio-mux-2026-09-06.json`。

该夹具只做轻量业务处理：大请求放在 initialize.hostVersion，大响应放在 describe.plugin.displayName；小请求为 project.list，并产生 4 个编号事件，同时发送心跳。读写包装器每次最多 4096 字节，对所选方向按字节 sleep。累计收到/发出超过 1 MiB 后由 stderr 通知 Host 开始探测，避免只测大消息发送前的空闲期。

这份包装器适用于两种协议，不依赖旧 Frame V1 header，因此可同场对照。4 MiB/s 是设定速率，系统 sleep 调度使实际吞吐更低；不能直接把耗时与旧三份 runbook 的不同节流器混算。

## 复现步骤

```sh
cargo run --release --manifest-path crates/Cargo.toml -p codepet-host --example mux_contention -- reports/stdio-mux-2026-09-06.json 3
```

主矩阵为 8/16 MiB 高熵正文 × 请求/响应两个方向 × 两个 profile × 3 次，每个 case 设定 4 MiB/s。另做 16 MiB、1 MiB/s 的双方向对照各一次，观察默认 10 秒业务 deadline 下的恢复。记录小 RPC、心跳、大 RPC、事件顺序、shutdown 和“小请求结束时大请求是否仍在传输”。

## 对照结果

以下为退出修复后的 28 个 case。4 MiB/s 主矩阵每格 3 次，表中是小 RPC 延迟中位数：

| 大正文 | 方向 | 旧 Frame V1 | Mux |
| --- | --- | --- | --- |
| 8 MiB | 请求 | 2094.6 ms | 123.2 ms |
| 8 MiB | 响应 | 2071.7 ms | 120.8 ms |
| 16 MiB | 请求 | 4545.4 ms | 123.8 ms |
| 16 MiB | 响应 | 4550.8 ms | 121.7 ms |

主矩阵 12 次 mux 小请求均在大请求完成前返回，心跳也正常；大请求吞吐量保持同一数量级，没有通过取消大请求来制造改善。加上 1 MiB/s 两个 case，14 次 mux 的事件顺序与 clean shutdown 全部通过。

1 MiB/s、16 MiB 正文下，大 RPC 在两种 profile 中都触发默认 10 秒 deadline；旧 profile 的小 RPC/心跳也超时，新 profile 分别约 446 ms（请求方向）和 452 ms（响应方向）完成，并正常退出。超慢大请求自身仍须使用合适的业务 deadline。

事件预算进一步收紧后，按同一矩阵各跑一次（12 个 case），输出保存在 `reports/stdio-mux-event-budget-2026-09-06.json`。6 个 mux case 的小 RPC、心跳、事件顺序与正常退出全部通过；`mux_slow_event_consumer_keeps_one_budgeted_event_without_blocking_rpc_or_shutdown` 还验证消费者停读时只保留一条带预算事件，其他 RPC 和 shutdown 继续完成。

## 自动化验证

已执行：

- `cargo test --manifest-path sdk/rust/codepet-provider-sdk/Cargo.toml`：38 个测试通过，含 mux 协商、控制预算、解压边界、错误恢复、失活回收，以及旧 Frame V1 保持原语义。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host`：68 个测试通过；之后新增的慢事件消费者回归也通过（Host 当前合计 69 个测试），包含三个真实内置 Provider 使用新 manifest 启动、业务调用与退出。
- `cargo check --manifest-path crates/Cargo.toml -p codepet-provider-codex -p codepet-provider-claude -p codepet-provider-opencode --all-targets`：通过。Codex 原有 `CodexThreadItem` 未使用 import warning 仍存在。
- `npm run protocol:check`：17 个测试通过；`npm run sdkgen:test`：3 个通过。
- `bun tools/cp-sdk-gen/cp-sdk-gen.mjs --lang rust --output /tmp/codepet-mux-sdk-export-20260906` 后，对导出的 Provider SDK 执行 `cargo check --manifest-path /tmp/codepet-mux-sdk-export-20260906/codepet-provider-sdk/Cargo.toml --offline`：通过。
- `git diff --check`：通过。没有运行格式化工具。

重点回归：`blocked_large_stream_does_not_block_small_stream_and_cancel_releases_capacity` 同时停读两条大 stream，要求第二条大消息能取得传输机会，小 RPC 仍完成；取消后 normal slots 与编码字节额度恢复。

## 压测发现的退出问题

第一份 28 case 结果保存在 `reports/stdio-mux-before-shutdown-fix-2026-09-06.json`：14 个 mux case 中有 2 次 shutdown 未成功。小 RPC/心跳已正常返回，但该版本关闭时不允许读取剩余 stream 缓存，也未等待实际 stdout pump 完成。

新增 `shutdown_waits_for_the_blocking_output_pump_to_flush_and_finish`，在真实 Unix socket pair 字节流接口上给 flush 加延迟；修复前稳定失败于最后 response 无法读取。启用关闭后已缓存数据读取，并在 runtime 返回前等待 stdout owner 完成后，该测试通过。首次传输开发中的问题不应被写成历史生产版本的 Bug；此前生产版本尚无 mux。

## 边界与后续检查

此矩阵证明有界分片可显著改善同向大正文期间的小请求进展，不能代替生产长稳、RSS 或跨平台验收。之前的并发/RSS与跨进程基线继续作为后续参照：业务对象和事件尺寸计数仍有 CPU/内存成本，分片不会令吞吐超出物理 pipe 带宽。Windows/Linux 原生 pipe、真实大历史长期运行和多 Provider mux 高并发性能尚未专项测量。

若小 RPC 仍很慢，先检查 `provider.mux.ready` 日志确认协商模式与双方容量，再区分队列/编码、pipe、业务计算与事件消费。不要因为见到 CPRF 正文就误判为旧 profile；新协议的 CPRF 在 Yamux stream 内。
