# 连接引擎独立测试与双端验收

2026-11-07。用于验证 [连接引擎](../10-architecture/connection-engine.md) 的选择、健康、流量与交接边界。通过核心压测不能推导真实网络带宽或手机原生资源稳定性。

## 独立功能与压力测试

在 CodePet Remote 的 `packages/codepet_connection_engine` 目录使用 Dart 3.13.2：

```sh
dart pub get
dart format --output=none --set-exit-if-changed lib test tool
dart analyze --fatal-infos
dart test
dart run tool/stress.dart 10000 16
```

这套测试可以把 `lib/test/tool`、pubspec 与 analysis_options 单独复制到临时目录运行；没有对 App、Flutter、业务账号、VPS 或真实网络的依赖。允许以适配器故障注入替换 socket、ICE、时钟、健康事件和业务交接。核心架构检查同时审计 import/export，禁止业务/平台依赖。

已验证 21 项核心测试：四路竞争、首个可用再择优、去重、超时与迟到、关闭事件恢复、取消交接、授权变化、恢复优先的候选预算、后台预算、空闲计数、dispose 隔离、native close 失败保护，以及 1,000 次连接断开资源清零。

本机混合压测基线（模拟 IO、Dart JIT，非 Release 网络指标）：16 个设备、10,000 轮，包含 3,334 次硬故障恢复、2,500 次健康升级。一次记录 ready p50/p95 为 133/465μs，恢复 p95 392μs，升级 p95 460μs，清理 p95 34μs；峰值进程 RSS 255,541,248 字节，包含 VM/适配器/统计数组，不能据此声称引擎单独占用该内存。最终 liveConnections/permits/waiting 均为 0。数值随机器、JIT/GC 与并发变化，以重新执行结果为准。

每批结束都断言连接、候选许可和队列资源清零；任何失败退出码非零。压力结果只写纯文本。CI 独立 `connection-engine` 作业安装 Dart，保存 `connection-engine-stress` 产物；App 的 Quality 作业单独初始化该包的开发依赖，避免干净环境中 `package:test` 解析缺失。

## 应用桥接与 Host 集成

已执行：

| 命令 | 结果与覆盖 |
| --- | --- |
| Remote `flutter analyze` | 无问题 |
| Remote `flutter test` | 450 项通过，含桥接、设备会话、旧行为和页面回归 |
| Remote `flutter build apk --debug` | 成功；验证 Android NetworkCallback/EventChannel 编译，非 Release/签名验收 |
| Host `npm run protocol:check` | 22 项通过，schema/fixture/生成物一致 |
| Host `cargo test --manifest-path crates/Cargo.toml -p codepet-host --lib remote` | 57 项通过 |
| Host `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test remote_lan_listener` | 4 项通过、2 项按原测试配置忽略；包括真实 TLS/WSS 和原生 RTC 跨通道交接 |
| Host `cargo test --manifest-path sdk/rust/Cargo.toml -p codepet-channel-control-sdk` | 编译通过；DTO 正反例由协议生成器 fixture 检查承担 |

`negotiated_channel_handover_fences_old_requests_and_projects_one_active_connection` 用真实 TLS/WSS 与原生 RTC 验证 prepared 不接纳业务、不显示在线，CAS 切换后旧通道被拒绝、确认重复幂等，Host 只显示一个活动连接的流量。旧端仍走原兼容流程。Bridge 测试另覆盖确认丢失查询、健康升级排空写请求、事件续订游标、普通 RPC 超时不重放、WSS 大响应心跳宽限、硬关闭恢复及显式断开不复活。

Tauri library 编译：默认 `cargo check --manifest-path src-tauri/Cargo.toml --lib` 被既有 macOS allowlist 检查阻断；使用已有 runbook 的编译专用覆盖后通过：

```sh
TAURI_CONFIG='{"app":{"macOSPrivateApi":false},"bundle":{"resources":[]}}' cargo check --manifest-path src-tauri/Cargo.toml --lib
```

这只验证实际桌面调用方/公开 re-export 能编译，不等于 macOS Release 打包、资源 staging 或真实窗口运行验收。

## 排查与仍需实机验证

- 关联 Remote `connection.engine` 的 target/epoch/connectionId 与 Host `channel.control` 的 connectionId/activeConnectionId；关注 prepare、activate、CAS 冲突和清理隔离。日志沿用年月日、时分秒、PID、线程、文件行号和文本字段，不存新的 JSON 日志文件。
- “有流量但心跳慢”：区分有序大响应和黑洞。RTC 可观察片段进度，WSS 只能看到完整消息；应用心跳宽限最长 60 秒，不能用短探测超时截断合法帧。
- `cleanup_failed_quarantined`：适配器没有证明 native close 完成，许可保留，达到预算停止分配。先修原生关闭失败，不通过清空计数掩盖资源泄漏。
- 公网 VPS probe 与 Android instrumentation probe 本轮未执行。还需手机 Wi-Fi↔4G、企业 VPN/Tailscale、VPN 开关、IPv6 地址变化、纯上游断网、前后台恢复及持续 30 分钟的 CPU/RSS/FD 趋势验收。
- 真实环境分别量触发→探测、ready、Host active 确认、首个业务成功；记录 p50/p95、中断时间和传输量。统计是应用载荷字节，不含 IP/TLS 重传，不能当公网链路带宽测速或运营商流量账单。
