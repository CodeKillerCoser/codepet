# WebRTC 公网信令与 TURN 部署

## 范围与当前证据

2026-09-09，用户授权在既有 VPS 部署公网连接。主工作树保持不动，实施位于
`codex/webrtc-channel`。LAN/WSS、LAN 发现和配对保留，文件挂载与预览不在范围。
用户已安装并确认 M2 Windows/Android LAN RTC 可连接。

服务器 `172.96.254.12` 实际为 AlmaLinux 9.7，1 GB 内存。SSH 已验证；TCP 443
由既有 xray 占用，因此保留 443，HTTPS 信令使用 **8443**，没有要求用户购买域名。
Let's Encrypt IP 证书已申请成功；Certbot 5.8.0，自动续期模拟通过。

## 身份与连接设计

不增加账号登录。管理员在 VPS 运行 `services/signaling/provision_host.py`，
为指定 Host deviceId 签发独立信令 token，并将配置写入权限 600 的文件。
Host 数据目录 `remote-access/rtc-cloud.json` 保存 serviceUrl、hostToken、Ed25519
seed 与已绑定 peer。配置不得进入源码仓库、APK 或日志；普通安装包不内置管理员凭据。

Remote 在原 pinned LAN HTTPS + bearer 下调用 `/remote/v1/channel-bootstrap`：
先在安全存储持久化自己的 seed，再提交公钥；Host 首次绑定到当前 credentialId，
之后拒绝无确认换 key。返回 Host 公钥、信令地址、独立 client token。
客户端 token 通过已认证 LAN 直接交付（本阶段省略额外的一次性兑换券往返）。
服务只保存 token hash。首次使用公网前需在同 LAN 成功升级一次。

与旧长线提案的具体差别：公网 offer 的有效 Ed25519 签名证明持有新绑定的设备私钥，
Host 将其解析为原有活跃 credentialId，进入共用 SessionRegistry/握手；不再经云端传
业务 bearer，也不增加未实现的 control DataChannel。撤销检查在资源分配前后执行，
既有统一断开路径继续关闭 LAN 与 RTC。云端权限最多有轮询同步延迟，不能越过 Host 撤销。

签名对象是 **原始 UTF-8 JSON bytes**，信封携带 base64 payload 与 Ed25519 signature；
验证方不重新序列化。签名覆盖 v/kind/host/client/attempt/expires/description，
description 包含完整 SDP、候选及 DTLS 指纹；answer 额外绑定 offerHash。
有效期 60 秒，Host 校验时间窗和有界重放集合；固定 Remote offer、Host answer。
云端能看到连接元数据及 SDP，不能冒充已经由 LAN 绑定的对端。

Remote RTC 路由先尝试本地 resolver（整体预算 8 秒），失败关闭本地 peer 后使用
安全存储里的公网配置。失败不重放业务请求、不自动改为 WSS；WSS 仍由设置关闭 RTC
后选择。云端地址不会覆盖已有 preferredEndpoint。云端中断不主动关闭健康 DataChannel。

## 服务与端口

| 组件 | 配置 |
| --- | --- |
| Nginx | TLS 8443，反代 127.0.0.1:8787，100 KB body 上限、IP 限流、关闭 access log |
| 信令 | Python 3.12 / aiohttp 3.14.3；独立 codepet-signal 用户，SQLite，systemd MemoryMax=192M |
| coturn | 4.17.2；3478 UDP/TCP、5349 TCP TLS；UDP relay 49160–49259 |
| 凭据 | TURN REST HMAC 临时凭据 24 小时，secret 只在服务器；客户端到期前 5 分钟受控重连 |
| 配额 | 每个 TURN 用户 8 allocations，全局 64；每用户/总带宽限制；拒绝私网和回环 relay 目标 |
| 证书 | IP SAN，约 6 天有效；systemd 每日两次检查；deploy hook 重载 Nginx/coturn |
| 保留服务 | SSH 22、既有 xray 443 不改动 |

普通 Nginx 默认 80 listener 改为 loopback 8080，原配置保存在
`/etc/nginx/nginx.conf.before-codepet`。公网 80 专供 Certbot standalone 验证/续期。
部署脚本见 `services/signaling/deploy.py`，不会关闭防火墙或改动 xray。

## 服务协议与资源上限

`GET /health` 无认证；其他入口一律独立 bearer：Host 可 PUT `/v1/clients` 同步授权，
GET `/v1/offers` 拉取，POST `/v1/answers` 回答；Remote 可 POST `/v1/offers`、
GET `/v1/answer?attempt=...`。两者经 GET `/v1/ice` 领取临时 ICE 配置。

每 Host 最多 32 个 Remote，全局最多 128 个待连接信箱，每 Remote 只保留一个 attempt，
信箱 60 秒到期；旧 answer、跨 Host 写入、匿名 TURN、超限请求均拒绝。
服务重启使未完成协商重试，不影响已经建立的数据链路。

## 检查与验收

- `services/signaling/test_server.py`：两个异步用例通过，覆盖角色隔离、跨 Host 信箱隔离、
  撤销、重复 offer/answer 拒绝、TURN 限流。测试使用临时 SQLite，不修改生产授权。
- Host 原 61 单测与 2 个 LAN/RTC 集成测试通过；新增签名绑定定向测试通过。
- Remote 原 RTC/设置/App/分层 21 项通过；新增 answer 签名、身份、attempt、offerHash、
  过期时间与篡改测试通过。
- 公网 HTTPS health 实际可达；nginx/coturn/codepet-signal 均 active；
  `certbot renew --dry-run` 成功。不跳过 TLS 证书验证。
- Windows Rust↔Rust `rtc_public_relay_probe`，强制 TURN/UDP：实际选中 relay，
  公网信令、签名 SDP、Gateway 握手、72 KiB 级请求 ID 双向分片及撤销全部通过，约 30 秒。
  两个 peer 位于同一电脑但策略强制中继，证明经过 VPS；不代替手机移动网络验收。
- 最终 Remote Gateway/设置/App/分层回归 107 项通过；固定 Debug APK 构建通过。
- 最终 Host 全量单测首轮 61 项通过、原 event journal 测试临时文件操作出现 Windows
  PermissionDenied；该项独立重跑通过，未修改该模块。随后 remote 模块 31 项及独立
  LAN/RTC 集成 2 项均通过，不能把全量首轮记成全绿。
- 真机强制 relay 探针扩展 `RTC_CLOUD_PROBE=true`，复用既有 native probe 并使用
  独立测试 Host 授权。手机锁定，ADB 安装等待约 8 分钟后停止；仅构建成功，测试未运行。
  已关闭临时 Host、删除临时配对导出文件并移除 ADB 转发。待用户解锁再测试。

## 风险与尚待验证

大陆运营商 UDP/非 443 TLS 的可达性、TURN UDP/TCP/TLS 分支、Wi-Fi 与移动数据切换、
长连接临时凭据到期重连需实测。现阶段不提供 TURN/TLS 443，不能保证所有企业网络可达。
中继经美国 VPS 时会绕行美国；需记录 selected candidate pair，不能仅看是否配置 TURN。

已确认的 SDK 限制：webrtc-ice 0.14.0 的 `agent/agent_gather.rs::gather_candidates_relay`
只实现 TURN/UDP；TCP 与 TLS 分支标为 TODO。强制 TCP 的 Rust 探针未获得 relay candidate，
因此本轮不声称桌面端具备 TURN/TCP/TLS 兜底。Android 原生库与服务器配置支持不等于两端
路径已验证；Host 所在网络若完全禁用 UDP，需后续升级或补齐 ICE 实现。

公网测试另暴露两项修复：后台授权同步与 LAN bootstrap 共用异步锁，避免旧快照覆盖新
client token；会话退出的所有清理任务共用 250 ms 预算，小于外层 500 ms 撤销等待，
避免 TURN 关闭帧等待 ACK 导致撤销接口误报超时。云端大消息测试使用生产 RPC 的 15 秒
预算，保留原 LAN fixture 5 秒预算，不通过放大生产超时掩盖问题。

Host 配置/私钥丢失需要可信 LAN 重新配对，不能通过信令服务悄悄替换身份。
配置不正确时应保留 LAN 可用；当前公网服务属于单 VPS，无高可用承诺。
