# 手机接管超时但底层可能已获得控制权

## 11-10 手机导出日志补证

后续用户提供 `codepet-logs-2026-11-10T14-46-44-147255Z`。该包从北京时间
15:01:40 开始保留记录，未覆盖本机 12:09 的异常。全部五个日志中没有 `rpc.timeout`
或 `provider_request_timeout`。唯一初次接管为 15:11:08 的 `conversation.resume`，
280 ms 返回 `conversation_write_conflict`；15:11:18/28/38 的三次 acquire 重试分别
170/69/89 ms 返回错误。不能把这段已收到明确拒绝的链路归因为下面的期限倒置。

相邻的 timeout 属于 LAN WebSocket 建连（8 秒）与发现（4.4 秒）；接管走已连通的
IPv6 RTC，历史读取也成功。读取历史或收到会话事件不是已获得写入控制权的证据。
目标设备 ID 以 `device-8819250a` 开头，与本机 `device-8d247c27` 不同，不能直接
用本机 Provider 日志证明该远程请求的服务端结果。原始卡住/超时仍需对齐发生时间和设备。

另查本机 ChatGPT 日志发现 12:09:05 Codex CLI 收到 SIGKILL 后广播 fatal error；
现有强制接管的 macOS ChatGPT 进程识别有对应风险，详见
[强制接管与 ChatGPT 子进程异常](codex-force-takeover-chatgpt-child-exit.md)。

## 现象与证据

2026-11-10 用户反馈手机获取控制权报 timeout，但看起来已接管。本次检查的是本机运行的 `0.3.10-beta+topedge.20261110.7`；具体失败会话、时间及手机报错原文尚未确认，不能把下面的可复现缺陷直接等同于那次现场的唯一根因。

- 本机 `Library/Application Support/code-pet/logs/code-pet.log` 与 Provider 日志没有找到对应 acquire 请求追踪。`lsof -a -p 58859 -d 1,2` 显示 stdout/stderr 为 `/dev/null`；`remote/channels/session.rs` 的旧 trace 仅 `eprintln!`，从 Finder 启动的安装版因此丢失这部分证据。
- Codex 日志有模型刷新超时，但不能据此推断会话接管失败。Android 模拟器保留日志中找到的是 11-09 18:51 UTC 的 `conversation_write_conflict`，不是这次 timeout。
- `providers/process.rs` 原本对所有 Provider RPC 使用 10 秒期限，Codex `client.rs` 的原生请求期限为 30 秒。Host 可以先关闭等待，而底层恢复仍继续运行。
- Remote 的 WebSocket/WebRTC 原本统一等待 30 秒。初次加载使用 `conversation.resume`；Host 先 acquire，再 get 历史，手机拿到合并响应之前无法确认接管结果。
- 新进程级测试以 2 秒普通期限、3 秒 acquire 响应复现同一先后关系：修复前报 `provider_request_timeout / conversation.acquireInteraction / stage=stream`，修复后成功，期间并行普通 RPC 仍正常。

## 修复与边界

Host `PluginProcessOptions.interaction_timeout` 独立设置为 45 秒，只用于 `conversation.acquireInteraction`；普通 RPC 仍为 10 秒。该期限给原生 30 秒恢复及通常的初始化留出余量，仍受 Provider mux 60 秒生命周期上限约束；不是无限等待，也不保证异常启动或长排队一定成功。

Remote 的 `lib/gateway/request_deadline.dart` 统一 LAN/RTC 策略：acquire 和合并 resume 等待 75 秒，覆盖 Host 45 秒接管、10 秒历史及网络余量；普通请求仍 30 秒。没有改写接管结果，没有自动补发写操作；仍需成功响应才能允许交互，超时保持结果未知。

Host 共用通道分发增加 `interaction.rpc.started/completed`，走已有可落盘诊断 sink，记录方法、请求 ID、耗时、是否接管和错误码，不记录会话正文或凭据。completed 表示服务端处理结束，不证明手机已收到；排查时仍需对应手机回包日志。

## 验证

- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test process_rpc`：9 项通过，包括慢 acquire、并发调用和超时后通道恢复。
- `--test manager_gateway`：45 项通过，包括 resume、释放/重新接管及 Provider 生命周期相关回归。
- Remote `flutter test test/gateway/webrtc_transport_test.dart test/gateway/pinned_web_socket_transport_test.dart test/gateway/gateway_client_test.dart test/conversations/conversation_detail_screen_test.dart`：151 项通过。RTC 可控时钟验证第 31 秒的接管响应可接收、普通请求仍超时、第 76 秒无响应时报告结果未知且通道可继续使用。
- Android API 36 模拟器运行 `integration_test/interaction_deadline_test.dart`：通过。模拟器内隔离 TLS/WebSocket 服务延迟 31 秒响应 acquire/resume，真实 LAN 客户端成功接收；普通历史调用仍在 30 秒失败，后续 RPC 成功，无重复接管请求。
- Host `--test remote_lan_listener`：4 项通过、2 项需外部配置的探针忽略，覆盖 LAN/RTC 共用分发、通道切换与撤销。
- 本次修改的 Dart 文件静态分析通过。模拟器使用隔离测试服务，没有接管或释放用户真实会话，也不能替代真机现场复测。

## 后续定位与未知项

修复需同时重新构建桌面 Host 和 Remote 才完整生效；本次未发布或替换用户安装版。若安装后仍发生，先取得手机报错原文、Provider、时间和请求 ID，对照上述 started/completed 与手机 `rpc.timeout`。`provider_request_timeout`、手机 transport timeout、writer conflict 是不同失败，不能混为一谈。

缺陷引入提交未确认。极慢初始化、历史大包及断网仍可能超过预算，应根据请求阶段证据进一步处理，不能靠无限延长超时或收到 Ready 状态就认定接管成功。
