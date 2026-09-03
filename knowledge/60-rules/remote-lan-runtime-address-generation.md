# Remote LAN 运行期地址代际规约

## 规则

Remote LAN 的 publishable IP、mDNS A/SRV/interface、TXT `pair`、status/QR `httpsBaseUrl` 和 pairing exchange `gatewayUrl` 必须作为一个 advertised generation 管理。网卡或默认路由变化不得重启 wildcard listener、TLS identity、credential store、Gateway 或 Provider；新 generation 收到 mDNS Announce 后才提交，失败必须撤下过期 discovery 并自动重试。Remote 必须按稳定 `deviceId` 持续刷新发现目录，并以 `deviceId + clientId + TLS fingerprint` 绑定已配对关系。IP、端口、mDNS 名称、QR URL 和 `gatewayUrl` 都只是可替换的 locator，不能成为设备身份或唯一重连入口。

## 适用场景

适用于 `codepet-host` 的 LAN listener/mDNS、Tauri `RemoteAccessRuntime` 的地址解析、pairing watch、status、QR、retry 和 shutdown，以及 CodePet Remote 的发现目录、Gateway endpoint 解析和 session 重连。凡是新增 advertised endpoint 字段、改变网卡选择、修改 mDNS TXT 或调整 pairing 生命周期，都必须检查 Host generation 与 Remote 候选切换是否仍以同一稳定设备身份收敛。

## 反例

Host 启动时把 `192.168.0.105` 分别复制进 listener handle、Axum pairing state、Tauri status 和 mDNS `ServiceInfo`，之后系统默认路由切到 `30.45.163.4`。`0.0.0.0:47622` listener 仍可从新 IP 访问，但 mDNS daemon 继续绑定旧 interface，QR 和 exchange 继续返回旧 URL；进程、5353 socket 和业务 Gateway 都存活，客户端仍无法发现或重连。另一个反例是 Remote 每次重连先等待旧 IP 超时，最后只打开一次短 mDNS 查询窗口；即使 Host 已发布 B，客户端也可能长时间停留在失败退避状态。先宣布 B、再更新 exchange state 还会使客户端在短窗口内发现 B 却收到 A 的 WSS URL。

## 推荐做法

- 跨 macOS/Windows/Linux 复用 `select_remote_lan_ipv4`，以有界低频轮询观察默认路由；连续两次相同结果再协调，shutdown watch 必须停止任务。
- listener 只绑定一次 `0.0.0.0:<actual-port>`；client-visible endpoint 使用共享的 committed/pending 状态，不把 URL 复制进各 handler。
- 网络变化和 pairing 变化共用一个 lifecycle mutex。替换顺序为：stage candidate、注销旧 service、启动新 daemon、等待完整新 service Announce、刷新 active QR、commit；Announce 窗口内，匹配 pending `Host` authority 的 exchange 返回 pending gateway URL。
- 无 publishable 地址时有界注销旧广告并清空 committed endpoint；status 保持 listener `available`，URL 为空并暴露 retryable discovery diagnostic。地址恢复或 Announce 失败后由同一低频检查重新发布。
- 失败 generation 不得放回 advertiser slot，也不得恢复旧 endpoint；listener/TLS/session/credential/Gateway 保持原对象。
- Remote 在 App 生命周期内只维护一份共享发现目录，以 `deviceId` 更新当前 SRV/A endpoint；缓存必须有过期时间，相同地址只刷新 freshness，不制造重复重连事件。
- Android 真机应优先使用系统 `NsdManager` 发现 DNS-SD 服务，并把解析出的 SRV 地址与 TXT 元数据送入与其他平台相同的严格校验；Dart 原始 UDP mDNS 只作为回退。不能把 `NetworkInterface.list` 返回的 Wi-Fi、蜂窝、VPN、虚拟和 loopback 接口全部加入多播的成功作为真机前提，也不能静默吞掉主发现路径失败后直接呈现“没有设备”。
- Android Emulator 的 `10.0.2.0/24` NAT 不转发宿主机 LAN 上的 UDP 5353 广播。调试模拟器可以额外探测固定别名 `10.0.2.2:47622` 的公开 HTTPS discovery metadata；该回退不得在真机或发布构建启用，不得替代 mDNS，也不得绕过证书实际指纹比对、数字口令确认或 Host 接受。
- `10.0.2.2` 只是模拟器本地可达别名，不是 Host advertised generation 的地址。Host 接受配对后可以返回真实 LAN `gatewayUrl`；经 pin TLS 连接、`deviceId`、证书指纹和配对码共同验证后，Remote 应接受 Host 授权的新 host/port locator，不得要求它与发起请求时的 IP 或端口相同。`gatewayUrl` 仍必须为无 userinfo/query/fragment 的 `wss://.../remote/v2/gateway`。
- 连接解析优先尝试仍新鲜的发现候选，再回退持久化 endpoint 和稳定端口。已配对发现候选必须在建连前同时匹配已保存的 `deviceId` 与 TLS fingerprint，建连时再通过 credential、证书 pin 和 Gateway handshake 完成最终验证。首次配对可把 TXT `fp` 用作候选 pin，但只有两端显示相同数字码并由 Host 明确接受后才能签发 credential；mDNS TXT 本身不能升级为信任来源。
- 通过验证的新 locator 只能更新与当前 `deviceId + clientId + TLS fingerprint` 完全一致的配对记录；重新配对并换证书后，旧 session 的延迟回调不得覆盖新记录的 endpoint。
- 发现到同一已配对设备的新 endpoint 时，应立即唤醒 retryable 网络失败；credential 拒绝、身份不匹配等非重试错误不得被发现事件反复重试。

## 来源

2026-09-01 现场证据：Host PID 59555 于 05:50 启动时 `en0=192.168.0.105`，09:59 切换为 `30.45.163.4`；新 IP 的 47622 可直连、旧 IP 不可达，daemon 与 5353 socket 仍在，但 DNS-SD 无实例。Git 证据显示 immutable listener endpoint 由 `6fa1c6b6` 引入，Tauri 启动期一次性解析与持有由 `daee063c` 引入；此前没有运行期网络监听。

2026-09-03 模拟器现场证据：macOS `dns-sd` 能发现 Host 的 `_codepet._tcp.local.` 和 47622，Android 端已获得 multicast 权限并持有 lock；模拟器 `eth0=10.0.2.15/24` 的抓包只看到发往 `224.0.0.251:5353` 的查询，看不到 Host 响应。因此该环境需要显式的宿主机别名回退，不能用模拟器结果否定真机 LAN mDNS。

2026-09-03 真机排查证据：Host 在 `192.168.0.105:47622` 监听，`/remote/v1/discovery` 返回完整 `id/name/fp/vmin/vmax/pair`，macOS `dns-sd -B` 与 `dns-sd -L` 均能在 en0 对应接口发现并解析 `_codepet._tcp.local.`；同一局域网 Android 真机仍显示空目录。Remote 当时只使用 Dart `multicast_dns`，其启动会枚举所有 IPv4 接口并逐个 `joinMulticast`，目录 runner 又会把启动错误折叠为空扫描。该证据排除了 Host 未监听、未注册服务和 TXT 缺字段，但尚不能单独区分具体 Android 网卡加入失败与网络设备对多播的限制，因此真机改用系统 NSD，原始 UDP 保留为回退。

## 验证方式

- Host mDNS 单测必须验证 A→B 同时替换 address/interface 与最新 `pair`，并保留 unregister ack、旧 resend 隔离、Announce 和失败清理。
- 真实 listener 测试必须验证 committed=A、pending=B 时，经 B authority 的 pairing exchange 已返回 B，且无 endpoint 时返回 retryable 503 而不消费 pairing。
- Tauri 测试必须覆盖 A→B 一致性；A→无地址时 listener/TLS/credential 不变并可恢复 B；IP/pair 竞态最终只留最新 generation；Announce 失败可重试；shutdown 后 resolver 调用停止。
- Remote 测试必须覆盖同一 `deviceId` 的 A→B 缓存替换、过期候选淘汰、B 优先于持久化 A、发现事件立即唤醒 retryable failure，以及非重试认证失败不被唤醒。
- Android NSD 测试必须覆盖：原生发现结果优先于原始 UDP；原生空结果或失败继续 UDP 回退；原生记录仍校验字段白名单、协议版本、`pair` 和 64 位小写十六进制证书指纹。真机 smoke 需要在 Wi-Fi 开启、蜂窝与 VPN 接口同时存在的常见环境下发现 Host。
- 模拟器回退测试必须覆盖：HTTPS metadata 与实际叶子证书指纹一致才形成候选；回退失败仍继续 mDNS；探测仅由调试 Android 模拟器身份启用。真实模拟器 smoke 必须验证 `10.0.2.2` 候选可见且首次配对仍经过双端确认。
- 配对测试必须覆盖 Host 返回与 QR、mDNS 或 `10.0.2.2` 不同 host/port 的 accepted `gatewayUrl`，确认 Remote 在身份链路完整时保存 Host 授权的 locator；非 WSS、错误 Gateway 路径或携带 userinfo/query/fragment 仍应失败关闭。
- Remote 重连测试必须覆盖：相同 `deviceId` 但 TXT `fp` 缺失或不同的发现候选不得连接；旧证书 session 不得写回新证书配对记录的 preferred endpoint。
- 发布 smoke 必须在当前真实 IPv4 上用 DNS-SD 看到 `_codepet._tcp.local.`，并让已安装 Remote 自动连接后只读列出数据。
