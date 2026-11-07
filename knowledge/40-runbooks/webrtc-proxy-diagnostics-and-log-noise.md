# 代理环境的网络采证与重复日志排查

## 现象与证据

2026-11-07 用户要求补充代理环境下 IPv6 采证，并分析此前导出中的异常和重复日志。企业 VPN 已能经现有 LAN 通道工作，本次不调整路由或绕过 VPN。

原始证据为 Downloads 下 `codepet-logs-2026-11-06T17-12-43-547501Z/raw/codepet.jsonl`，共 793 条、790862 字节。下列行号均对应此文件，时间为 UTC；原始文件未改写。

- 177 条 `ice.stats` 占 543017 字节，约 69%；90 条 `signal.http` 中 73 条为 `GET /v1/answer` 返回 200。28 条小请求发送各重复记录 start/complete；请求本身还有 RPC 关联及 span 日志。
- 69–83、298–312 行的两次 55 秒失败属于设备 `device-544a159c-5b27-45f2-8a0a-53ae6b1bae77`。404、628、632 行的缺少公网授权属于 `device-9c90579e-3989-4942-a21a-b9e90d1bad67`。这些不是当前 Mac 直连失败。
- 382–383、514–517 行：当前 Mac 的 race `remote-1793985120249442-2`，选中 IPv6 host/host UDP 候选对。520–524 行其他分支 `cancelled/anotherPathSelected` 是正常竞速收尾，不是报错。本文件当前 Mac 连接上线后的 RPC 没有 error response、timeout 或 channel.abort。
- 93、326 行：切换网络前两次接收 680798/1385488 字节，组包分别耗时 12663/24380 ms；相应 `conversation.resume/get` span 均成功，期间心跳有约 4/9 秒等待。729 行：IPv6 连接中同样 680798 字节接收耗时 460 ms。只能判断大消息在旧通道较慢且存在排队，不能仅凭应用日志断言带宽上限或丢包原因。
- 17:07 导出第 911–914 行的 LAN close 1002 仍缺少关闭方具体协议错误证据，不能把它直接改为无条件重试。第 1209 行有 Android NSD 调用超时，缺少当时系统调度/原生日志，尚不能判定平台根因。
- 更早 13:11 导出的本地解析缓存 `/tmp/codepet-phone-parsed.json` 包含 1976 条相同 `SocketException errno=1 port=5353`，全部发生在 11-05 00:40–16:46 UTC；17:12 文件没有 SEVERE。当前 discovery 已采用 Android NSD 和失败限频，不能把旧累计记录算成本次新增故障。

## 原因与边界

日志膨胀来自把周期全量 ICE stats、成功空轮询、单帧发送/接收生命周期全部持久化；降低日志级别并无效，因为 `AppLog` 持久化 Level.ALL。

代理与 IPv6 的根因仍待实机采证。此前开 VPN 的 IPv6 分支在本机无候选、尚未发信令就失败；关闭后可直连。信令 `server/server.py` 只保存并转发签名信封，没有把 HTTP 来源 IP 改写为 ICE 候选地址。系统看到接口 IPv6 不代表应用有权限从该网络发送。

约 40 秒的建连等待已定位到完整 ICE gathering 后才交换 SDP，见 [重连等待诊断](webrtc-reconnect-ice-gathering-delay.md)。本记录的诊断阶段未修改该流程；后续已按用户要求实现增量候选与协商压缩，验证结果及仍有限制见 [性能修复](webrtc-incremental-ice-and-compression.md)。

## 本次改动及相关模块

- Remote `diagnostics/network_environment.dart` 与 Android `NetworkDiagnosticsBridge.kt`：提供只读统一接口；每次 race 开始、选中或失败时采集默认/进程绑定网络、Wi-Fi/蜂窝/VPN、接口地址族和范围、默认路由、MTU、HTTP 代理配置是否存在。Android 一次性读取 ConnectivityManager，不请求新网络、不 bind socket；其他平台仅枚举接口，未知能力不伪造为 false。读取限时 1 秒、异步执行，不阻塞竞速。
- Remote `rtc_diagnostics.dart`、`path_policy.dart`：用实际过滤函数输出拒绝原因及总数；`networkAddressId` 以 race 为盐并规范化地址，可把原生接口和不同 WebRTC 分支对应。SDP 的 `addressId` 继续以 offer 为盐，便于双端核对；原始 IP、代理主机、凭证与 SDP 不写日志。每条 RTC 日志携带目标摘要，避免混淆不同设备。
- 手机与 Host 的 RTC diagnostics：继续采样实际通道；只在拓扑/候选对状态变化、关键阶段、关闭或稳定 30 秒时输出完整 stats。保留真实计数，不用默认零推断没有发送检查包。
- 双端 RTC transport：单帧快速消息省略重复传输生命周期，保留 RPC 关联、分片进度、大消息、背压和慢发送记录。Remote cloud signaling 合并成功应答轮询，在验证完成/关闭时记录次数和等待时长；HTTP 错误仍即时记录。
- 沿用纯文本写盘、轮转、导出，不新增 JSON/JSONL 文件；原生快照只经 MethodChannel 传回 Dart 并进入同一日志链路。

## 验证与回归防线

单测验证系统接口与候选关联、IPv6 文本表示规范化、敏感字段不落盘、采集失败隔离、过滤理由与 SDP 选择一致、stats 稳定降频但状态变化和关闭不漏记、小 RPC 降噪且大消息仍记录。Android 原生桥接需编译并在模拟器运行读接口→写纯文本的集成测试。文本导出仍由现有 diagnostics 测试检查。

已执行：

- Remote `flutter test test/gateway test/diagnostics test/discovery`：162 通过；后续补充小消息降噪用例、目标关联与轮询汇总后，复跑相关 RTC/信令/diagnostics 集合 47 通过。新用例显式使用单帧小响应，并确认多帧大响应仍留记录。
- Host 在 `crates/` 运行 `cargo test -p codepet-host --lib remote::channels::webrtc -- --test-threads=1`：13 通过。
- Android `./gradlew :app:compileDebugKotlin` 成功；`CODEPET_RTC_PROBE=true flutter test integration_test/network_diagnostics_test.dart -d emulator-5554`：1 通过。实际读取 ConnectivityManager，写盘验证来源/PID/线程、地址摘要和纯文本；使用独立 `.rtcprobe` Debug 包，未替换用户 Release。测试后关闭本次启动的模拟器。
- 修改涉及的 Dart 文件及测试 `dart analyze` 无问题；两个仓库 `git diff --check` 通过。

Android 一次性快照所用 API、VPN transport 与 HTTP 代理字段含义依据 [ConnectivityManager 官方文档](https://developer.android.com/reference/android/net/ConnectivityManager)。`getAllNetworks` 只在边界读取，未用作轮询监控；快照可能与网络切换竞争，不能把缺少结果当成系统永久无 IPv6。

实机使用同一手机同一网络，依次关闭代理、开启代理、代理允许时按应用排除 CodePet；每次新建连接。按 targetId/raceId/offerId 关联；先看 network.snapshot 是否可见 IPv6，再看 candidateFilter 的拒绝原因，随后比较手机 localOffer 与电脑 remoteOffer，最后确认 selectedCandidatePairId 和实际传输。接口可见而候选缺失、候选被过滤、候选交换不一致、连通性检查失败是不同问题。双方候选都有仍不通时，再限定端口抓包；现有 Host ICE 库部分检查计数不可用，不能假造结果。

## 引入历史与未知项

高频日志引入提交未确认。当前修改尚未在用户手机的代理网络实测，也未替换用户安装版；模拟器无法证明真实蜂窝 IPv6 或 VPN 路由可用。1002 原因、NSD 偶发超时、代理具体的 IPv6 限制仍需新日志/原生证据。
