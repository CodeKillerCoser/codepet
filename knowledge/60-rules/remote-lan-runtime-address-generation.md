# Remote LAN 运行期地址代际规约

## 规则

Remote LAN 的 publishable IP、mDNS A/SRV/interface、TXT `pair`、status/QR `httpsBaseUrl` 和 pairing exchange `gatewayUrl` 必须作为一个 advertised generation 管理。网卡或默认路由变化不得重启 wildcard listener、TLS identity、credential store、Gateway 或 Provider；新 generation 收到 mDNS Announce 后才提交，失败必须撤下过期 discovery 并自动重试。

## 适用场景

适用于 `codepet-host` 的 LAN listener/mDNS，以及 Tauri `RemoteAccessRuntime` 的地址解析、pairing watch、status、QR、retry 和 shutdown。凡是新增 advertised endpoint 字段、改变网卡选择、修改 mDNS TXT 或调整 pairing 生命周期，都必须检查 generation 是否仍由同一串行协调器收敛。

## 反例

Host 启动时把 `192.168.0.105` 分别复制进 listener handle、Axum pairing state、Tauri status 和 mDNS `ServiceInfo`，之后系统默认路由切到 `30.45.163.4`。`0.0.0.0:47622` listener 仍可从新 IP 访问，但 mDNS daemon 继续绑定旧 interface，QR 和 exchange 继续返回旧 URL；进程、5353 socket 和业务 Gateway 都存活，客户端仍无法发现或重连。另一个反例是先宣布 B、再更新 exchange state，使客户端在短窗口内发现 B 却收到 A 的 WSS URL。

## 推荐做法

- 跨 macOS/Windows/Linux 复用 `select_remote_lan_ipv4`，以有界低频轮询观察默认路由；连续两次相同结果再协调，shutdown watch 必须停止任务。
- listener 只绑定一次 `0.0.0.0:<actual-port>`；client-visible endpoint 使用共享的 committed/pending 状态，不把 URL 复制进各 handler。
- 网络变化和 pairing 变化共用一个 lifecycle mutex。替换顺序为：stage candidate、注销旧 service、启动新 daemon、等待完整新 service Announce、刷新 active QR、commit；Announce 窗口内，匹配 pending `Host` authority 的 exchange 返回 pending gateway URL。
- 无 publishable 地址时有界注销旧广告并清空 committed endpoint；status 保持 listener `available`，URL 为空并暴露 retryable discovery diagnostic。地址恢复或 Announce 失败后由同一低频检查重新发布。
- 失败 generation 不得放回 advertiser slot，也不得恢复旧 endpoint；listener/TLS/session/credential/Gateway 保持原对象。

## 来源

2026-09-01 现场证据：Host PID 59555 于 05:50 启动时 `en0=192.168.0.105`，09:59 切换为 `30.45.163.4`；新 IP 的 47622 可直连、旧 IP 不可达，daemon 与 5353 socket 仍在，但 DNS-SD 无实例。Git 证据显示 immutable listener endpoint 由 `6fa1c6b6` 引入，Tauri 启动期一次性解析与持有由 `daee063c` 引入；此前没有运行期网络监听。

## 验证方式

- Host mDNS 单测必须验证 A→B 同时替换 address/interface 与最新 `pair`，并保留 unregister ack、旧 resend 隔离、Announce 和失败清理。
- 真实 listener 测试必须验证 committed=A、pending=B 时，经 B authority 的 pairing exchange 已返回 B，且无 endpoint 时返回 retryable 503 而不消费 pairing。
- Tauri 测试必须覆盖 A→B 一致性；A→无地址时 listener/TLS/credential 不变并可恢复 B；IP/pair 竞态最终只留最新 generation；Announce 失败可重试；shutdown 后 resolver 调用停止。
- 发布 smoke 必须在当前真实 IPv4 上用 DNS-SD 看到 `_codepet._tcp.local.`，并让已安装 Remote 自动连接后只读列出数据。
