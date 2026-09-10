# LAN RTC 路径失效与公网重连候选收集过慢

## 现象

2026-09-10 用户提供 15:47:52 UTC 导出的手机日志，确认连接的是本机 Windows。
本轮只分析日志，没有更改连接策略。桌面安装版文件版本为 `0.3.9-beta+b39c36e`。

## 复现路径

用户已确认：手机先通过局域网完成连接，然后关闭 Wi-Fi 切换到 4G，随后进入公网重连。
因此原 LAN UDP 路径失效有明确触发条件；问题集中在切网后的恢复过程。

## 证据

手机来源：`D:/17633/Downloads/codepet-logs-2026-09-10T15-47-52-864021Z/codepet.log`。
Host 来源：`C:/Users/17633/AppData/Local/code-pet/logs/code-pet.log`；VPS 为信令 journal。
下表手机时刻已换算北京时间，Host/VPS 时钟略有偏差，按 offerId/attempt 关联。

| 时间 | 证据 |
| --- | --- |
| 23:46:27 | offerId `b0d6ad97e4c5371b55614bd6` 的 DataChannel 打开；选中 UDP、手机 Wi-Fi/private prflx → Host private host，RTT 4–6 ms，明确是 LAN 直连 |
| 23:46:33.647 | 最后一条业务消息收到；共 38 条接收完成，最大 15032 字节，每条均为单片 |
| 23:46:34–48 | 原选中候选对 bytesReceived 固定 88411；requestsSent 从 7 增至 48，responsesReceived 固定 6，之后还尝试其他候选对 |
| 23:46:41 / 51 | 手机 ICE 先 Disconnected 后 Failed；随后 RTC peer closed，七个在途请求失败 |
| 23:46:51 | channel.abort：partialReceived=0、partialTotal=0、partialFrames=0，最后一片已在 17562 ms 前收到；本次没有 fragmentTimeout |
| 23:46:52–23:47:00 | 重连仍先尝试 LAN；整体 LAN 预算约 8 秒，底层 HTTPS 10 秒超时稍后出现 |
| 23:47:03–04 | 公网 ICE 配置获取成功，手机已经收集到 2 个 srflx、4 个 relay 候选 |
| 23:47:42.974 | 等待 gathering complete 后才发送 offer，公网连接开始后已用时 42659 ms |

对应公网 offerId 为 `a24b7df4203216c065623be2`，attempt 为
`6c54f2979953310922594a719f134e1249c89b40426e56593aea87fea7a11bfd`。
VPS 的 UTC 记录：15:47:45.791 接收 offer，47.636 交给 Host，53.347 接收 answer，
54.047 将 answer 返回手机，服务端 offer 到 answer 交付约 8.3 秒。
Host 用约 5 秒收集候选，15:47:54.381 ICE connected，55.838 peer connected，
56.311 channel.openResult opened=true，同时 dataChannel.closed，56.315 完成关闭。
Host stats 存在 succeeded+nominated 的 relay → srflx 候选对，公网可用路径仍是中继。

LAN 同一 offer 的 Host 记录同样先 disconnected 后 failed；38 次 message.send.start
均有 complete，未记录 send.backpressure。没有 Host 卡在大消息分片发送的证据。

代码证据：Remote `WebRtcGatewayTransport._open` 必须等待 gathering complete 才发送 SDP；
`RoutedRtcTransport` 公网 connectTimeout=55 秒，覆盖配置获取、收集、信令、ICE/DTLS/SCTP 全过程。

## 初步影响范围与当前假设

- LAN 路径：原本健康的直连在用户关闭 Wi-Fi、切换 4G 后失效，不是 VPS 中继慢，也不是大列表组包超时。
  Host 并未整体退出，仍持续同步公网身份。手机没有立即重新协商可用公网路径，而是等待 ICE failed 后恢复。
- 公网重连：候选早已部分可用，但全量收集等待接近 40 秒，占满大部分总建连预算。
  Host 通道打开又立即关闭，与手机 55 秒总期限高度吻合；手机日志截止 23:47:52，
  尚无最后几秒的 timeout 记录，故不能把总期限触发写成完全证实的最终根因。
- 业务请求：conversation.list/resume/ping 等在连接 Failed 后一并报错；当前证据不支持
  把列表执行超时当成本次连接中断的起点。

## 已排除方向

初始连接未经过 VPS；手机没有待组装消息；信令 offer/answer 实际均交付成功，
不是公网未授权、VPS 没有回答或 Host 始终不可达。

## 下一步排查与验证

1. 已确认切换 Wi-Fi/4G；补充网络变化事件，并评估立即触发受控重连/ICE restart，避免先等待旧路径失败。
2. 获取 23:47:53–23:48:00 手机日志，验证总建连超时与 Host 通道关闭的因果关系。
3. 将候选收集耗时独立限界；评估完整 trickle ICE，或有界收集后发送并明确迟到候选处理，
   不能只截断等待而忽略信令兼容和慢速 TURN 候选。
4. 区分收集、信令、通道握手的预算，测试慢/无响应 ICE server 与已有可用候选的组合。
5. 验证 LAN → 4G → LAN 切换、取消与旧 peer 回收、并发请求不重放，并分别测量恢复耗时。

## 未知项

候选收集尾部等待的具体 ICE server/协议；手机最后几秒的关闭原因。
