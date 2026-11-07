# 2026-11-07 重连约 50 秒与 IPv6 无候选

后续进展：已实现客户端增量候选交换及大响应压缩，见 [性能修复与验证](webrtc-incremental-ice-and-compression.md)。下文保留旧安装版实机证据，不代表用户设备已装入新代码。

## 现象与复现范围

用户提供 `codepet-logs-2026-11-06T17-07-06-739345Z`，要求解释重连慢并确认是否 P2P。手机构建为 Release `5947136`，尚未安装后续纯文本日志改动。本次仅诊断，未修改连接代码或重启服务。用户随后表示关闭 VPN 复测，新包 `codepet-logs-2026-11-06T17-12-43-547501Z` 的对照结果见下文。

## 证据

下述时间均为北京时间 2026-11-07；源目录位于 `/Users/wangxin/Downloads/`。

- `raw/codepet.jsonl:911–914`：01:04:32.611 当前 Mac 的 LAN 以 code=1002 关闭，retryable=false，关闭 race；01:05:04.496 才出现下一次连接请求，间隔 31.885 秒。不能推定此间隔全是系统等待或确认用户何时点击重连。
- `raw/codepet.jsonl:974–990`：当前 Mac race `remote-1793984704496131-9`，四路在约 9 ms 内并行启动，排除串行依次尝试。
- `raw/codepet.jsonl:996–1003`：IPv6 路只得到 IPv4 和 IPv6 loopback 候选；141 ms 后 noCandidates，没有开始 IPv6 信令交换。只证明 WebRTC 未获得可用 IPv6，不证明运营商没有分配 IPv6。
- `raw/codepet.jsonl:1047`：01:05:08.168 出现 IPv4 relay UDP 候选 port=49247，后来成为选中候选；`1245` 直到 01:05:47.193 才开始中继信令，二者相差约 39 秒。
- `raw/codepet.jsonl:1280`：01:05:51.372 transport.selectedCandidatePairId 对应的 local 和 remote 候选均为 relay/IPv4/UDP；local.networkType=vpn。`1288` 在 01:05:52.461 选中 relay，`1576` 的 01:06:37.370 快照仍选择同一候选对，流量计数增加。不能把路径标签直接当作 P2P 证据。
- 01:05:55.089 当前 Mac 业务上线，连接总耗时 50,593 ms。中继路径依次为配置/创建约 2.87 s、候选收集 39.82 s、信令 2.65 s、应用应答/开通道 2.62 s、业务初始化 2.63 s。
- 电脑 `code-pet.log` 对 offerId `7a09dafb611b5b56d18b94c0` 记录中继 offer、answer 和 DataChannel 成功，补充证明本次双端已连通。IPv4 直连 offerId `d657fb1a29cdec075ef6e982` 得到 answer 后仅尝试约 2.09 秒即因 relay 胜出取消，不能据此断言该网络永远无法 IPv4 P2P。

## 已确认原因与边界

主要等待发生在手机 `webrtc_gateway_transport.dart` 的 `await gathered.future`：等全部 ICE 收集完成才发送 offer，已有后来成功使用的候选也未提前交换。底层为何持续收集到约 40 秒仍缺少原生候选错误信息，不能指定某一个服务器/协议为根因。

手机选中网卡的 vpn 标记来自 WebRTC getStats，不是读取手机系统 VPN 配置；无法凭此确定 VPN 产品或断言它必然屏蔽 IPv6。检查当前与 5947136 的应用配置未发现主动关闭 IPv6，IPv6 分支使用 host 候选并排除回环/链路本地地址。原生插件没有在应用调用处被设置 IPv6 禁用项。

额外空档涉及 `pinned_web_socket_transport.dart` 的关闭码重试策略和 `device_session.dart` 的失败处理：1002 不在可重试列表。关闭原因本身尚未证明，不能不区分协议错误就一律无限重试。另一个旧 Host 的 55 秒失败不要与当前 Mac 混为一次连接。

## 下一步与验证

先在相同网络关闭 VPN 复测，比较 IPv6 host 候选、networkType 和最终 selectedCandidatePairId；若仍无 IPv6，再核对手机实际接口地址与 IPv6 可达性，区分网络未提供、VPN/路由策略和原生采集问题。

候选收集优化需允许尽早交换并保留迟到候选（如增量 ICE），保持四路并行。验证首候选较早但收集很晚、迟到候选才可达、取消/超时、双端签名与候选关联等场景；不能简单截断 3 秒收集并宣称已修复。上线前必须再次实机比较建连耗时与直连/中继结果。

## 关闭 VPN 后的实机对照

新包仍为相同构建 `5947136`。当前 Mac race 为 `remote-1793985120249442-2`、offerId 为 `61036d43b6737eed144bef7c`。以下行号均指新包 `raw/codepet.jsonl`：

- 第 431 行，01:12:00.356 手机已采集 public/IPv6/host/UDP 候选；该路候选收集仅 125 ms。
- 第 514 行，01:12:02.445 实际 selectedCandidatePairId 对应本地和远端均为 public/IPv6/host/UDP，手机 networkType=cellular，state=succeeded、nominated=true，RTT 25 ms。这是直连实证，不只是路径名称。
- 第 517 行，01:12:02.514 竞速选择 ipv6-direct（路径 2242 ms，整个 race 2264 ms）；LAN、IPv4、relay 随即被取消。
- 第 560 行，01:12:02.691 当前 Mac 业务上线，共 2442 ms；上次相同目标为 50593 ms。
- 第 782 行，01:12:42.345 仍使用同一 IPv6 候选对，RTT 26 ms，bytesReceived=773687、bytesSent=27350，证明业务流量持续通过该直连。

对照支持此前 VPN 网络路径影响 IPv6 候选可见性：从 vpn/无可用 IPv6 变为 cellular/公网 IPv6，并且两端直接成功连接。仍不能凭应用日志确定具体 VPN 的地址、路由或过滤配置；若复开 VPN 需要兼容，须单独检查其 IPv6 支持。此次绕过了此前慢的 IPv4/relay 分支，不代表约 40 秒等待完整 ICE 的代码问题已经修复。VPS 仍承担信令交换，实际会话数据在本次记录中不走 TURN。
