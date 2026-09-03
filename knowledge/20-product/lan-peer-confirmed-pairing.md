# 局域网双向发现与确认配对

## 背景

原有 LAN admission 只能由 Host 生成带一次性 secret 的二维码，再由 Remote 扫码交换 credential。该路径安全但强依赖相机和当前 Host 地址，也不能表达“Remote 发现 Host 后主动请求”或“Host 发起后 Remote 弹出确认”。已配对设备的地址恢复解决了 DHCP/VPN 切换后的重连，但首次建立信任仍缺少类似蓝牙数字比较的交互。

## 目标

- Remote 在添加设备页列出同一局域网内发现的 Host，并能主动发起配对请求。
- Host 收到 Remote 请求后，在桌面端弹出设备信息和 6 位配对码，由用户接受或拒绝。
- Host 打开“添加设备”后通过 `pair=1` 广告意图；未配对 Remote 弹出邀请，接受后进入相同的数字比较流程。
- 只有 Host 接受请求后才签发 bearer；Remote 收到 accepted 状态后安全保存凭据并自动连接。
- 保留二维码路径，作为相机/OOB 已验证的兼容配对方式。

## 非目标

- 不支持跨子网、互联网中继、NAT 穿透或后台常驻唤醒。
- 不把 mDNS 名称、地址、TXT 或候选证书指纹提升为已配对身份。
- 不改变 Gateway v1 业务方法、Provider 权限模型或已配对 credential 格式。
- 不在本阶段加入自动接受、无界请求队列或多 Host 批量配对。

## 现状理解

Host 已持久化稳定 `deviceId`、自签名 TLS identity 和仅保存 bearer hash 的 credential store。`RemoteAccessManager` 管理一次性二维码 pairing session；TLS listener 提供 exchange/WSS/credential revoke；Tauri 前端轮询 pairing 状态。Remote 已有持续 mDNS 目录、证书 pin、secure storage 和连接恢复。

新流程不能直接复用二维码 secret：将 secret 放入 mDNS 会使同网段旁观者获得准入能力。因此发现广告只增加 `fp` 候选指纹；Remote 以该候选 pin 建立 HTTPS，请求返回的 6 位码必须同时显示在 Remote 与真实 Host。用户比较并在 Host 接受后，credential 才出现在 accepted 响应中。发现信息被篡改时，请求会到达另一 TLS identity，真实 Host 不会显示相同请求与配对码。

## 实现路径

1. Host mDNS TXT 发布 `id/name/fp/vmin/vmax/pair`。`fp` 用于锁定本次候选 HTTPS 会话，不代表已建立持久信任。
2. Remote 生成 256-bit `clientNonce`，向 `POST /remote/v1/pairing-requests` 发送目标 Host id、稳定 client id 和设备 descriptor。
3. Host 以 `requestId + clientNonce + TLS fingerprint` 派生 6 位数字码，将 pending request 保留两分钟；相同 client id 与 nonce 幂等返回同一请求。Remote 必须用本地保存的 nonce 和本次实际 pin 住的证书指纹独立派生相同数字码，不能直接信任响应携带的码。
4. Remote 只有在响应码与本地派生码一致时才显示数字码，并轮询 `GET /remote/v1/pairing-requests/:requestId`。Host 桌面端每两秒查询 pending request，显示原生确认弹窗。
5. Host 拒绝时状态变为 rejected；接受时 credential store 持久化 bearer hash，并在 accepted 响应中返回 bearer 和同代 Gateway URL。
6. Remote 再次核对 Host `deviceId`、TLS fingerprint、request id、数字码和 Gateway authority，写入 secure storage，创建 session 并自动连接。
7. Host 主动打开添加设备时继续发布 `pair=1`；Remote 将其解释为邀请并弹窗，但仍走同一 pending/数字比较/Host 最终接受流程。Host 接受后原 pairing session 同步变为 succeeded。

## 涉及模块

- `protocol/channel/lan/v1/`：新增 pairing request/create/status DTO 与状态枚举，保持 admission 与 Gateway 分层。
- `crates/codepet-host/src/remote_access.rs`：pending request、幂等、TTL、接受/拒绝和 credential 签发的安全状态机。
- `crates/codepet-host/src/remote_listener.rs`：新增 TLS REST create/status 路由，只在 accepted 状态返回 bearer。
- `crates/codepet-host/src/remote_mdns.rs`：发布候选 TLS fingerprint 和 Host pairing intent。
- `src-tauri/src/runtime_gateway/remote_access.rs`、`frontend/`：向本机 UI 投影 pending request并要求用户确认。
- `codepet-remote/lib/discovery/`：持续投影地址变化和 `pair` 元数据变化，两类事件互不制造错误重连。
- `codepet-remote/lib/application/pairing/`、`lib/pairing/`：配对请求用例、严格响应校验和 secure credential commit。
- `codepet-remote/lib/features/connection/`、`lib/app/`：发现列表、数字码等待态、Host 邀请弹窗和成功后自动连接。

## 风险

- mDNS spoofing / TLS relay：`fp` 只作候选 pin；Remote 从请求 transcript 和实际 pin 住的证书独立计算数字码，不能采用中间方可改写的响应码。双方必须看到相同数字码，Host 未接受前不签发 credential。真实双机 smoke 验证错误码不接受。
- 请求洪泛：pending request 有 TTL 和数量上限；后续若真机日志出现滥用，再增加按源 IP 的 token bucket。
- accepted bearer 被重复读取：request id 使用 128-bit 随机值且只经 pinned TLS 返回；状态短期保留用于移动网络重试，不写日志或普通 UI。
- Host/Remote 竞态：create 按 nonce 幂等；terminal 状态不可逆；二维码 session 只有在关联请求被接受后才成功关闭。
- 时钟偏差：Host 只用单调时钟授权 TTL；Remote 保留 Host `expiresAt` 作为显示信息，但以收到 create 响应后的本地等待预算决定是否继续轮询，不拿两台设备的墙上时钟直接比较。
- 地址切换：响应 Gateway URL 按请求 Host authority 与 advertised generation 计算；Remote 仍在连接时重新经过 TLS pin 和 handshake。
- UI 重复弹窗：两端按 request/Host identity 去重，terminal/过期状态停止轮询；自动化测试覆盖重复广告。

## 测试计划

- `npm run protocol:check` 验证 schema、fixture、生成 SDK freshness 和 admission/Gateway 分层。
- Host core 单测覆盖 pending、nonce 幂等、接受签发、拒绝不签发和 Host-initiated session 收敛。
- `remote_lan_listener` 真实 TLS 测试覆盖 create→pending→本机接受→GET accepted→bearer 可认证。
- Tauri command/frontend 测试覆盖 pending request projection、数字码文案、接受/拒绝参数。
- Remote 测试覆盖 mDNS `fp` 严格解析、`pair` 元数据事件、请求身份核对、本地数字码派生与 relay 篡改拒绝、pending 不落凭据、accepted 才持久化，以及 Host 邀请弹窗。
- 完整运行 Host Rust/Tauri/前端检查与 Remote `flutter analyze/test/build apk`。
- 人工双机验证 Remote 发起、Host 发起、拒绝、超时、错误配对码和 Host 换 IP 后重连。

## 知识沉淀

更新 `gateway-v1-lan-generation-contract.md`、`remote-host-display-name-and-identity.md` 与 `remote-lan-runtime-address-generation.md`，明确候选 pin、数字比较和 credential 签发边界。本页作为首次配对功能事实入口；真机发现/系统权限问题继续记录到独立 runbook。

## 未知项

- Android 厂商对前台持续 mDNS 的功耗和丢包差异仍需真机数据。
- 当前请求限流是进程级数量上限，尚未按源 IP 限速。
- iOS 工程尚未进入当前 Remote 仓库，因此 Bonjour entitlement/本地网络权限未验证。
