# 手机 4G 扫码绑定 Mac 超时：配对仍依赖 LAN，Mac 未配置公网身份

## 现象

用户于 2026-09-10 00:44 左右（北京时间）打开 Remote WebRTC 开关后，手机通过 4G
扫描 Mac 的配对二维码，绑定失败。用户已确认网络组合为 Mac + 手机 4G。
本轮为只读日志和代码分析，没有修改客户端、VPS 服务或授权数据。

## 复现路径

当前版本使用包含 Host 局域网 HTTPS 地址的二维码。手机处于无法访问该 LAN 的 4G
网络，扫码后向二维码地址发起 exchange；请求 10 秒后超时。该绑定入口未根据 WebRTC
开关改为公网配对入口，因此没有进入公网 SDP 交换或 DataChannel 建连。

## 证据

用户提供的 Remote 日志：
`D:/17633/Downloads/codepet-logs-2026-09-09T16-44-58-747491Z/codepet.log`。
日志 timestamp 为 UTC，以下已换算北京时间：

| 时间 | 证据 |
| --- | --- |
| 00:44:29 | 三个已登记设备在 `RoutedRtcTransport.connect` 报“请先在同一局域网连接新版 Host，完成公网通道授权。”，约 8 秒后失败 |
| 00:44:32 | `/remote/v1/webrtc/offer` 向旧 Host 地址发起的本地 HTTPS 请求超时；目标 Mac 为 `192.168.0.105:47622` |
| 00:44:39 | `codepet.log:5272–5274`：QR pairing started，目标 device-892a997a-13a2-4706-9442-15a713888e75，直接 POST `192.168.0.105:47622/remote/v1/pairings/<id>/exchange` |
| 00:44:49 | `codepet.log:5275–5276`：HTTP 10 秒 TimeoutException，调用栈 `LanPairingGateway.exchange`；总耗时 10004 ms |

代码证据（Remote 主工作区）：

- `lib/pairing/pairing_models.dart`：exchangeUrl 从 QR 的 httpsBaseUrl 构造。
- `lib/pairing/pairing_service.dart::LanPairingGateway.exchange`：始终通过 pinned HTTPS
  访问上述地址，未使用公网信令服务。
- `lib/gateway/webrtc/routed_rtc_transport.dart::connect`：本地连接成功后才 bootstrap
  公网授权；本地失败且安全存储没有 cloud token 时直接报错，尚未创建 CloudRtcSignaling。

VPS 只读检查：

- nginx、codepet-signal、coturn 均 active；VPS 本机访问 HTTPS 8443 `/health` 返回 ok。
  该结果证明检查时服务可用，不等同于已验证手机 4G 到 VPS 的可达性。
- SQLite 只查询 Host ID 和客户端数量，不读取或输出密钥：hosts 仅有
  `device-45a8659a-e862-4add-9e59-501981c3fa8c`（此前 Windows）及 `device-lan-listener`
  （探针）。本次 Mac 的 Host ID 不在库中；仅探针 Host 有一个信令客户端记录。
- 失败时间段无 codepet-signal/coturn journal 条目；coturn 文件最后写入为
  09-09 10:35:40 -0400，即北京时间 22:35:40，早于本次测试。
- Nginx `access_log off`，因此不能仅根据日志空白断言“从未有公网请求”。
  判定本次停止在公网协商之前，依据是手机的明确错误及对应分支代码。

用户提供的桌面日志 `D:/17633/Downloads/code-pet.2026-09-09.log` 来自 macOS，最后一条
为 09-09 18:59:43 +0800，未覆盖 09-10 00:44 失败时刻；不能用它判断当时 Host 是否在
监听、其安装版本、当前局域网地址或防火墙状态。旧记录的网卡不可用警告不是本次根因证据。

## 结论与影响范围

已确认两项缺口：

1. **首次公网扫码配对未实现**。WebRTC 开关选择绑定后的 Gateway 通道，扫码授权仍是
   LAN HTTPS。手机 4G 无法直达二维码内的 LAN 地址，本次 exchange 超时符合该路径。
2. **目标 Mac 未注册公网身份**。此前部署只配置了 Windows Host，服务安装到 VPS 并不
   会自动给所有安装桌面 App 的机器注册身份。Mac 需要独立的服务端登记及本地配置。

这些证据不支持把本次故障归因于 TURN/UDP、TURN/TLS、SCTP 分片或 DataChannel 稳定性。
当前已实现的路径是“可信 LAN 配对并完成公网授权后，再跨网连接”，没有覆盖本次验收场景。

## 下一步排查与修复方向

- 近期验证现有路径：为 Mac 独立配置公网身份，确认运行含公网通道代码的版本；同 LAN
  配对并确认 cloud bootstrap 成功后再切换 4G，记录 selected candidate pair。
- 满足直接 4G 扫码：新增公网配对协商入口和一次性、限时、受限的 QR 授权信息，绑定
  Host 身份并由 Host 批准，保留现有 LAN 配对兼容。不能直接把局域网 URL 替换为 VPS IP。
- 提高可观测性：记录 Host 公网配置加载状态、配对入口类型、信令阶段和错误码；不记录
  pairing secret、Bearer token、私钥或完整 SDP。分别验证缺配置、凭据无效、网络超时。

## 未知项

- Mac 当时所运行构建的精确版本、是否存在 rtc-cloud.json 及 listener 状态。
- 手机 4G 到 VPS 的 HTTPS/UDP 实际可达性，尚未进入可检验这一点的连接阶段。
- 新版 Google libwebrtc 的构建和 TURN/TLS 接入尚未完成，与本次扫码入口缺口分开处理。
