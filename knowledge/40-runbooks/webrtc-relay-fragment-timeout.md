# Windows 公网 RTC 中继连接后分片超时

## 现象

2026-09-10 手机从 Wi-Fi 切换到 4G 后，Windows 连接恢复缓慢，恢复后再次出现
`RTC fragment timed out`。本记录与 Mac 首次公网扫码配对失败分开；早期仅分析，
后续诊断和已实施修复见“2026-09-11 新样本”一节。

## 复现路径

Windows 同 LAN 配对并完成公网授权，手机切换 4G，等待自动重连，加载项目和会话。
用户报告多次恢复后再次断开，所提供日志记录了两次中继连接后的同类失败。

## 证据

来源：`D:/17633/Downloads/codepet-logs-2026-09-09T16-55-11-506062Z/codepet.log`。
以下时间为北京时间，原始 JSONL 为 UTC；JSONL 同时含普通日志与不带 message 的 trace 记录。

| 时间 | 证据 |
| --- | --- |
| 00:51:15 | Mac channel-bootstrap 返回 503，公网配置或可达性仍存在缺口 |
| 00:51:37 | Windows channel-bootstrap 返回 200，Cloud pairing upgrade completed |
| 00:52:01 | Windows RTC peer closed |
| 00:53:00.886 | 第 5494 行：Cloud signaling established RTC Gateway route; path=relay |
| 00:53:01–07 | protocol.handshake、protocol.describe、event.subscribe、project.list、conversation.list 等 RPC 成功；收到运行时事件 |
| 00:53:13.166 | 第 5555 行：RTC fragment timed out，随后多个待处理 RPC 和事件流失败 |
| 00:54:14.622 | 第 5574 行：再次建立 path=relay，握手及订阅成功 |
| 00:54:26.646 | 再次 RTC fragment timed out |

Remote `lib/gateway/webrtc/webrtc_gateway_transport.dart::_onMessage`：解码器返回未完成消息时，
使用 `_fragmentTimer ??= Timer(Duration(seconds: 5), ...)`。从首个未完成分片开始固定计时，
后续有效分片不会刷新期限。到期调用 `_abort`，失败所有待处理请求并关闭通道。
`framing.dart` 使用 16 KiB 帧、最大 4 MiB 消息。

Host `crates/codepet-host/src/remote/channels/webrtc/mod.rs`：接收端也有固定 5 秒分片期限；
发送端存在 64 KiB 高水位、16 KiB 低水位背压。因此需同时检查两端，避免只修手机方向。

`routed_rtc_transport.dart`：重连先尝试 LAN（8 秒超时），再使用公网连接（55 秒建连上限）。
日志中两轮恢复到在线分别约 59 和 62 秒；不能把全部时间解释成重试退避，需细分协商阶段。

VPS 只读核查：firewalld public zone 没有 sources 或 rich rules 来源限制，相关端口开放；
Nginx 配置未见 allow/deny 来源 IP 限制。coturn 配置没有 allowed-peer-ip，
denied-peer-ip 限制私网、回环、链路本地等中继目标，非手机公网来源白名单。
TURN 文件中的私网 peer 被拒绝不能单独解释本次故障，因实际中继已建立并传输了业务数据。

## 初步影响范围

- Remote RTC 组包：超时会使整个会话和所有并发请求失败。
- Host RTC 组包和发送背压：检查反方向期限以及发送是否持续取得进展。
- 路由重连：4G 下每轮先等待不可达 LAN，恢复耗时需要单独优化和观测。

## 2026-09-11 新样本：持续接收被固定期限截断

来源：`D:/17633/Downloads/codepet-logs-2026-09-10T16-05-19-139102Z/codepet.log`
及本机 `C:/Users/17633/AppData/Local/code-pet/logs/code-pet.log`。
两端以 `offerId=1b7df3f5a7b6a256702cd5d6` 和相同响应长度关联，以下为北京时间。

| 时间 | 证据 |
| --- | --- |
| 00:04:54.598 | Host 开始发送 3,264,247 字节，记录发送背压等待 |
| 00:04:55.188 | 手机收到首帧，预期总长度 3,264,247 字节 |
| 00:04:56.905–59.401 | 接收进度从 65,488 增至 147,348 字节 |
| 00:05:00.189 | 手机第 23 行 `channel.abort`：`fragmentTimeout`，已收 196,464 字节、12 帧，`partialAgeMs=5002`、`lastFragmentAgeMs=39` |
| 随后 | 手机 `conversation.get` 及 Codex snapshot 失败；Host 未记录该消息发送完成，随后通道关闭 |

此次明确是 Codex 对话快照 `conversation.get`，不能泛称为 `thread/list` 超时。
关闭前 39 毫秒仍有有效分片，证明固定 5 秒组包总期限截断了持续有进展的传输。
这是新样本已确认的直接断开原因；先前样本缺少进度字段的证据限制仍然成立。
同一导出中的 Claude 快照成功记录包含 2–4 条消息，与用户描述相符，但不据此推断所有
Claude 响应大小。此次总长度低于 4 MiB 消息上限，未触发超大消息拒绝。

断开前手机选中的候选对包含远端 relay，00:04:59 的检查状态为 succeeded、RTT 约
649 ms；关闭附近另一次采样约 1,385 ms。网络确实较慢，但现有证据不足以把低吞吐量
归因于 VPS 带宽、丢包或某个拥塞控制实现。应用主动关闭与底层吞吐原因应分别处理。

此次分析后用户明确要求普通 RPC 期限为 30 秒，请求失败不得误伤连接，存活由心跳负责。
下面的实现取代早期增加组包空闲/总期限的建议。

### 修复实现与验证

- Remote `webrtc_gateway_transport.dart`：默认请求期限从 15 秒改为 30 秒，超时只结束
  对应请求并记录 `rpc.timeout`，不再广播通道错误或关闭 peer。
- 两端 RTC 组包移除固定 5 秒期限，保留 CPG1 长度、偏移、内存上限检查。已开始的
  消息继续完整收发，Remote 丢弃已超时请求的迟到响应并记录 `rpc.response.discarded`。
  不能在 RPC 超时时清空解码器或中途停止发送，否则后续消息边界会损坏。
- LAN `pinned_web_socket_transport.dart` 同步采用 30 秒普通请求期限，保留原有 LAN 行为。
- 存活仍由现有 `protocol.ping`/pong 心跳管理；底层实际关闭、畸形协议、显式关闭和
  凭据撤销仍可终止连接。这些不是普通业务 RPC 失败。

验证：Remote 原相关测试集 69 项通过，补充后 RTC 测试 11 项通过；新增/更新用例覆盖超过 5 秒的组包、迟到响应、
背压时请求超时以及心跳失效通知。Host RTC 单元测试 6 项通过，包含跨 5 秒接收及
`recv` 被取消后保留组包状态。Host `remote_lan_listener` 的 RTC/LAN 共用准入、大消息和
撤销集成测试通过，两个手动探针按声明忽略。定向 Flutter analyze 通过。
执行命令：`flutter test` 指定 `webrtc_transport_test.dart`、`pinned_web_socket_transport_test.dart`、
`gateway_client_test.dart`、`cloud_signaling_test.dart`、`rtc_diagnostics_test.dart`；补充后单跑
`webrtc_transport_test.dart`；Host 执行 `cargo test --manifest-path crates/codepet-host/Cargo.toml
--lib remote::channels::webrtc` 和同 manifest 的 `--test remote_lan_listener rtc`。
真机 4G 大对话尚待新版验收。

剩余风险：CPG1 单有序通道的大消息可能阻塞后续 pong，达到心跳失效窗口时仍会重连。
需要真机观察心跳与发送队列日志；本修复不保证慢链路上 3.26 MB 响应可在 30 秒内完成。

## 01:45 复测：遗漏的 Host 整条消息发送期限

本机安装版本已核实为 `0.3.9-beta+0069e34`。Host 日志中的
`offerId=cb417c85c52035c5b0afac2e` 在北京时间 01:44:47.836 开始发送 3,264,247 字节，
01:45:02.850 主动记录 `channel.close`，相隔 15.014 秒；没有该消息的发送完成记录。
01:45:02.518 仍有背压释放，关闭前选中候选对为 succeeded+nominated，SCTP 累计发送
计数从 01:45:00.319 的 2,020,490 增至关闭时 2,487,428 字节。此计数不是该响应的
手机实际接收字节数。证据表明通道仍有传输进展。

源码 `crates/codepet-host/src/remote/channels/session.rs:254` 仍以
`timeout(CHANNEL_SEND_TIMEOUT, sink.send(message))` 包裹整条消息，常量为 15 秒；
到期通知 `transport_failed` 并退出 writer，导致会话结束。这是上一轮仅修改 RTC
适配器而遗漏的共用发送层，意味着此前“连接仅由心跳判活”的实现尚不完整。
日志没有独立 writer timeout 原因字段，但 15.014 秒时序、未完成发送及源码路径高度吻合。

VPS 同期信令 offer/answer 成功交付（约 6.327 秒），signal/coturn/nginx 持续运行且
NRestarts=0。Host 关闭后 coturn 收到 lifetime=0 的分配释放，随后有 client close 和
allocation timeout 清理；不能把这些后续清理日志当成 VPS 先断开的原因。

### 补充修复：固定调用期限，发送不按调用期限取消

用户确认接口从发起起固定 30 秒，超时只结束该调用；已经开始的消息继续传输，迟到响应
丢弃。共用 `session.rs` writer 移除 `CHANNEL_SEND_TIMEOUT`，直接等待 `sink.send`
完成，仅真正发送失败才通知 transport_failed。心跳、权限撤销和显式关闭仍通过现有
会话退出及有界任务清理终止发送，不把发送期限简单改为 30 秒。

生产 writer 提取为 `run_channel_writer`，测试直接使用该函数模拟 31 秒背压后恢复，
验证不会标记通道失败，后续消息不交错；另测实际 sink 失败和阻塞任务可取消。
仅 dev-dependency 启用 Tokio test-util，以虚拟时间覆盖旧 15 秒期限和 30 秒调用期限。
Remote 已有 30 秒请求隔离及迟到响应逻辑，本次不改动。

执行 `cargo test --manifest-path crates/codepet-host/Cargo.toml --lib remote::channels`，
25 项通过；同 manifest 执行 `--test remote_lan_listener`，2 项通过、2 个手动探针忽略，
覆盖 LAN/RTC 准入、订阅隔离、大消息、凭据撤销及关闭。原接收测试和快速集成测试没有
覆盖共用 writer 的期限，这次补充该缺口。
仍需真机验证 4G 下大响应，特别是同通道排队对 pong 的影响；移除发送期限不改变有限
队列的过载保护，也不保证大响应能在调用期限内完成。

## 早期样本的假设与排查

### 直连未被选中的专项核查

对本次导出全部五个手机日志文件检索 `srflx`、`prflx`、ICE gathering、candidate pair、
STUN 错误和 `path=`，只有两条最终 relay 记录，没有候选检查失败原因。已有桌面文本日志
不覆盖故障时刻；本地 events JSONL 是 provider 事件，不能替代 ICE 诊断日志。

代码及 VPS 部署源码检查确认：Remote 正常路径 `CloudRtcSignaling(state)` 没有设置
forceRelay；Host RTCConfiguration 保留默认策略。`/v1/ice` 同时下发
`stun:<VPS>:3478` 和 TURN 地址，coturn 未配置 no-stun。两端在发送本地 SDP 前等待
候选收集完成，未发现应用层只保留 relay 候选的过滤逻辑。这排除了明显的强制中继配置，
但不证明运行时 STUN 成功或候选检查成功。信令 offer/answer 是短期内存数据，未持久化，
无法从 SQLite 恢复当时的 SDP。

检查时 Windows WLAN 为 `192.168.0.106`，同时存在 Meta 虚拟网卡 `198.18.0.1`。
`Find-NetRoute -RemoteIPAddress 172.96.254.12` 选出 Meta、下一跳 `198.18.0.2`。
这是优先核查的代理/TUN 路由线索；不能外推为测试时路由，也不能据此证明显式绑定网卡的
WebRTC UDP 实际走该出口。已询问用户当时 TUN/VPN 状态，未擅自关闭代理或修改路由。
Windows 存在已启用的 Code Pet Public 入站允许规则，仍需结合当时网络配置及端口检查，
不能把防火墙因素完全排除。

下一轮直连验证应记录候选类型/协议/所属接口、STUN 错误、候选对检查状态及选中路径；
对照相同两端网络下 TUN 开关的结果，避免仅看到 relay 就推断为运营商 NAT 限制。

用户后续报告关闭 TUN 后仍然失败。该结果不支持把 TUN 当作已确认根因；尚需确认是
未建连还是分片超时、是否重建连接，以及新连接选中的路径。随后实时路由查询仍选中 Meta，
只能描述查询时状态，不能反驳此前关闭 TUN 的测试。已授权 ADB 上 `com.codepet.remote`
可读日志最后为 2026-09-09 14:11 UTC，不覆盖本轮；rtcprobe 未读取到有效日志。
Downloads 也没有较 16:55 UTC 导出更新的文件，因此当前无法据日志判断关闭 TUN 后的路径。

直接断开机制已确认是应用分片定时器主动关闭通道。固定总时长可能误杀持续有进展的传输；
但现有日志没有逐片时间、总长度、接收偏移，尚不能证明这两轮是缓慢传输还是发送/网络停滞。
无法从失败的并发 RPC 名称确定未组装完的是哪条响应或事件。

## 已排除方向

本次 Windows 已完成公网授权，中继可达且 JSON-RPC 可用；不能继续归因为未授权或 VPS
完全不可达。也没有证据支持通过放开私网 TURN 目标限制解决本问题。

## 下一步排查

- 在通道层记录组包总长度、已收字节、首片/末片时间和发送背压等待时间，不记录业务正文或凭据。
- 设计有效进展可刷新空闲期限的组包策略，同时保留内存上限和合理总时长限制。
- 验证慢速但持续有进展、完全停滞、畸形帧、超大消息及关闭后的资源回收；覆盖双向传输。
- 分别测量 LAN 探测、ICE 收集、信令轮询、连通性检查耗时，验证 Wi-Fi/4G 切换及 LAN 回归。
- 真机通过当前 VPS 重复加载会话，确认不会进入“成功—超时—长时间重连”循环。

## 早期样本的未知项

- 分片超时前传输是否持续有进展，以及具体未完成消息的长度和类型。
- 约一分钟恢复时间中各公网阶段的实际占比。
- 新一次用户报告的失败未包含在此日志导出中，不能擅自当作第三个已核验样本。
