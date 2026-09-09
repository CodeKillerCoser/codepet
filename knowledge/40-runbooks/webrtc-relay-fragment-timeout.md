# Windows 公网 RTC 中继连接后分片超时

## 现象

2026-09-10 手机从 Wi-Fi 切换到 4G 后，Windows 连接恢复缓慢，恢复后再次出现
`RTC fragment timed out`。本记录与 Mac 首次公网扫码配对失败分开；本轮仅分析，未实施修复。

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

## 当前假设

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

## 未知项

- 分片超时前传输是否持续有进展，以及具体未完成消息的长度和类型。
- 约一分钟恢复时间中各公网阶段的实际占比。
- 新一次用户报告的失败未包含在此日志导出中，不能擅自当作第三个已核验样本。
