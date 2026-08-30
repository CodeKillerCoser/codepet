# Gateway v1 LAN 生成协议契约

## 背景

`codepet-host::RemoteAccessManager` 已具备持久 TLS leaf identity、五分钟内存 pairing session 和只落盘 bearer SHA-256 的 credential core，Gateway SDK 已生成 Host identity、QR 与 REST body 契约。Phase 2A2 在这些既有事实上增加可复用的 LAN TLS/HTTP/WSS listener；Phase 2A3 增加只消费同一 identity 与 listener 实际 endpoint 的 LAN mDNS advertiser。两者都不生成第二份设备、证书身份或端口。

## 目标

- 从 `protocol/gateway/v1/schema.json` 同时生成 Rust 与 TypeScript 的 LAN DTO。
- 保留 `HandshakeRequest.clientId` 作为 remote client 唯一协议身份，并要求 transport 将其与 pairing credential 绑定。
- 让 handshake 和 pairing response 使用同一 `RemoteHostIdentity`，其中证书指纹语义唯一。
- 以单个 TLS listener 落实固定 REST/WSS wire 边界，并通过 `_codepet._tcp.local.` 发布同一 listener 的实际 LAN IP 与 TLS port。
- 保持 mDNS 只负责发现；Tauri/UI 启停与 pairing 状态接线仍留给下一阶段。

## 非目标

- 不实现 Tauri/UI 启停接线或 Pet/Desktop IPC。
- 不修改 `gateway/v1/compat-v0.*`、Desktop IPC companion、Pet、activity projection 或 Provider 业务 DTO。
- 不增加 RBAC、credential scope、refresh token、证书轮换或权限模型。

## 现状理解

`DeviceRegistry` 仍是 `deviceId/displayName` 唯一事实来源；`RemoteAccessManager` 持有 leaf certificate DER 与 fingerprint、pairing session 和 credential store。`ProviderGatewayService` 实现 Gateway v1 业务 dispatch，listener 只在 transport 层验证 bearer、绑定 socket clientId 和维护撤销取消；服务层不接收 bearer 或 TLS 状态。现有 `devices/providers` handshake 集合保持不变。

## 实现路径

固定网络入口如下：

```text
POST   /remote/v1/pairings/{pairingId}/exchange
GET    /remote/v1/gateway                    # WSS Upgrade
DELETE /remote/v1/credentials/current
```

`PairingExchangeRequest` 只包含 `pairingSecret/clientId/clientName/platform`；`pairingId` 来自 path。响应只包含 `device/gatewayUrl/credential`，credential 对客户端 opaque。DELETE 返回最小 `{ revoked }` DTO。以上类型属于同一 Gateway schema 的独立生成类型，不进入 envelope method manifest。

QR 只编码 `PairingQrPayload`：`version/hostDeviceId/displayName/httpsBaseUrl/certSha256/pairingId/pairingSecret/expiresAt`。PairingOffer 的状态、倒计时等只留在 Host/UI 内存。`pairingSecret` 明文只能进入 QR encoder，不在普通 UI 文本、日志或持久文档中展示。

`RemoteHostIdentity` 固定为 `deviceId/displayName/identityFingerprint`。`identityFingerprint` 与 QR `certSha256` 都是 leaf certificate DER SHA-256 的 64 位小写 hex。`HandshakeResponse.device` 必填，同时保留既有 `devices/providers`。普通 `ProviderGatewayService::new` 不携带 remote identity，因此 Gateway v1 `protocol.handshake` 继续以 `remote_host_identity_unavailable` fail-closed。LAN listener 启动前要求调用方通过 `ProviderGatewayService::with_remote_identity` 显式注入 `RemoteAccessManager::remote_host_identity()`，并再次核对完整 identity；TLS 配置直接读取同一个 manager 已持久化的 certificate/private key DER。

连接次序固定：客户端先 pin TLS peer leaf DER；WSS Upgrade 必须携带有效 bearer；第一条业务请求必须是 `protocol.handshake`；listener 校验 handshake `clientId` 等于 credential 绑定的 `clientId`；客户端再核对 response `device.deviceId` 与 `device.identityFingerprint`。后续请求统一交给生成 SDK 的 envelope dispatcher，不在 listener 复制 conversation/turn/approval 路由。

`event.subscribe` 是每条 socket 的显式推送门。listener 在成功响应前先从现有 EventPublisher 建立 replay/live subscription，响应入队后才启动该 socket 的 event send loop；`GatewayEventSubscription` 以 cursor 去除 receiver 与 replay 窗口交叠，因此顺序是 replay 后 live 且不重复。每条 socket 最多成功订阅一次，未订阅 socket 不收到 server event，但仍可执行普通请求。

`RemoteLanServerConfig::default()` 的 bind address 是 `0.0.0.0:0`，但 bind address 与客户端可见 authority 分离：wildcard bind 必须由调用方显式提供具体 `advertised_host`，否则 start fail-closed；listener 只把实际端口附到该 IP/DNS host，handle 与 pairing response 共用由此得到的 `httpsBaseUrl/gatewayUrl`，绝不发布 `0.0.0.0`。真实网络测试显式绑定 loopback 并传入 `127.0.0.1`；后续 mDNS/Tauri 负责提供真实 LAN host。

每条 socket 独立持有 credential clientId、握手状态、订阅和容量为 64 的 outbound queue；writer 单次发送有一秒上限，response/event 入队有两秒上限，任一 transport backpressure 只取消该 session。listener 以全局 32 permit semaphore 限制并发 WSS session，超限 upgrade 返回稳定 503。`DELETE current` 只撤销发起 bearer，并按非敏感 credentialId 取消对应 listener-local sessions；同一 credential 的所有 socket 都关闭，其他 credential 不受影响。显式 shutdown 先停止新 session、取消现有 socket，再有界关闭 server task；TLS handshake 自身也有小于 server shutdown 窗口的上限。坏 JSON、binary frame、超限 frame 和协议次序错误只产生固定 Gateway error、WebSocket close 或安全断开，不进入 panic 路径。

mDNS 使用仍在维护且跨 macOS/Linux/Windows 的纯 Rust `mdns-sd 0.21`；Host 不需要异步或 logging feature。service type 精确为 `_codepet._tcp.local.`，TXT 精确且仅有 `id/name/vmin/vmax/pair`：`id/name` 只读取 `RemoteLanServerHandle` 在 listener 启动时已核对并保存的 `RemoteHostIdentity`，advertiser API 不再接受另一份 manager，因此不能拼接 manager A 的 identity 与 listener B 的 endpoint；`vmin/vmax` 都是 Gateway `PROTOCOL_VERSION=1`，`pair=1` 只表示 Host 当前主动开放一次性 pairing，否则为 `0`。SRV/A/AAAA 使用同一 handle 的 advertised IP 和实际 TLS port；DNS host、unspecified/multicast/documentation IP、与具体 bind 不一致的 IP 或地址族均 fail-closed。显式 IP 还必须存在于 `if-addrs` 返回的 active 本机接口集合，接口枚举只做归属校验，不代替调用方猜公网、LAN 或 loopback 地址；测试可显式使用匹配的 loopback。

instance name 由可读 display name 加 device id 的短哈希冲突后缀组成，hostname 使用相同冲突后缀；`mdns-sd` 默认 probe 继续处理极小概率的局域网名称冲突。instance、hostname 与 IP 都只是发现元数据，稳定身份只能读取 TXT `id`，TLS 信任仍只能来自 pairing 后的证书 pin 与 handshake 核对。TXT 不携带 certificate fingerprint、secret、credential、Provider、项目或会话。

`mdns-sd::ServiceDaemon::register` 只保证命令入队，因此 advertiser 在 start 和 pair 值变化后通过 daemon monitor 等待固定短窗口内目标 fullname 的 `DaemonEvent::Announce`；只有实际发送产生该事件才返回成功。`DaemonEvent::Error`、monitor 断开或超时都会注销 service 并停止 daemon；idle 期间积累的 error/断开由下一次值变化 update 或 shutdown 读取并执行同样的 fail-closed 清理。同值 update 是严格 no-op，不读取 backend。

fullname 本身不能区分注册代际，旧 daemon 的 `RegisterResend` 可能在普通 drain 后产生同名 Announce。`RemoteLanMdnsAdvertiser::update_pairing_available` 因此在值变化时先等待旧 service 的 `UnregisterStatus::OK/NotFound` 终态 ack，再 drain ack 前的 monitor 事件、停止旧 daemon并创建新 daemon，最后以同一个 fullname 注册新 TXT 并等待新 monitor 的 Announce。endpoint 和除 `pair` 外的 TXT 不变；该顺序可证明不会把旧代 Announce 当成新代确认，但 unregister 到新 Announce 之间存在短暂发现空窗，不是原子或无缝更新。下一阶段调用者必须在 pairing start、cancel、consume 时显式更新，不能由 advertiser 创建 listener 或轮询 pairing。`shutdown` 检查 monitor 后注销 service 并停止 daemon，成功停止后的重复调用是 no-op，Drop 只做同样的 best-effort 收尾。

## 涉及模块

- `protocol/gateway/v1/schema.json` 与 `fixtures/`：LAN DTO 与可验证 wire 样例的唯一事实来源。
- `tools/protocol-codegen/`：支持 standalone type fixture 与 fingerprint pattern 校验，保证 Rust/TypeScript freshness。
- `sdk/rust/codepet-gateway-sdk`、`sdk/typescript/codepet-gateway-sdk`：生成 DTO 与语言侧编译测试。
- `crates/codepet-host/src/remote_access.rs`：消费生成 `PairingExchangeRequest`，不保留手写同义 DTO。
- `crates/codepet-host/src/gateway.rs`：构造必需 handshake `device`；认证仍留在 transport 外层。
- `crates/codepet-host/src/remote_listener.rs`：单 TLS listener、固定 REST/WSS route、per-socket 状态、撤销取消和有界 shutdown。
- `crates/codepet-host/src/remote_mdns.rs`：只从 listener handle 的同源 identity/endpoint 构造唯一 service，以本机接口与 Announce 证明发布，并通过 unregister ack 与新 daemon 代际更新 pairing availability、持有生命周期。
- `crates/codepet-host/tests/remote_lan_listener.rs`：真实 loopback TLS pin 与完整 WSS/撤销/多客户端证据。

## 风险

- 指纹格式漂移：schema pattern、fixture、Rust SDK 解码与 Host TLS 定向测试共同验证 64 位小写 hex。
- `clientId` 出现同义字段：生成器测试断言 request 只有 `clientId`，无 `remoteClientId`；真实 WSS 负例验证 credential 绑定。
- 无 identity 的进程内服务被误用于 LAN：Gateway v1 handshake fail-closed，listener start 拒绝与 manager identity 不一致的 service；loopback 测试核对真实 TLS leaf 指纹、pairing response 与 handshake identity 三者相同。
- wildcard bind 被误当作可访问 endpoint：start 在缺少 advertised host 时 fail-closed；测试分别验证 `0.0.0.0:0` bind 与 `listener.local:<actual-port>` handle URL，并用显式 `127.0.0.1` 完成真实网络闭环。
- 发现数据被误当身份或泄露敏感上下文：构造测试断言 service type、实际 port 与 TXT exact set，并使用 sentinel fingerprint 证明不进入 service；文档和 review 继续要求客户端只信任 pairing/TLS/handshake。
- identity 与 endpoint provenance 被错误拼接：公开 start API 只接受 listener handle，编译形状测试固定该边界；真实 listener 测试核对 handle identity 等于启动它的 manager identity。
- 广告不可达或串到错误接口：advertiser 只接受与 listener bind 匹配、存在于 active 本机接口集合的显式 unicast IP，并将 service 限定到该地址；确定性负例覆盖 DNS host、unspecified、bind mismatch 与非本机 IP，macOS smoke 对 start 和 `pair=0→1` 各等待目标 fullname 的真实 Announce。实际跨设备发现、网卡切换和防火墙仍需下一阶段真机验证。
- daemon 入队或旧代 resend 被误报为新发布成功：fake backend 把旧 Announce 精确注入到 update 的首次 drain 与 unregister ack 之间，断言 ack 后 drain 和新 daemon 会丢弃它，且没有新 Announce 时必须超时；另覆盖 unregister error/timeout 清理、同值 no-op、Announce、idle 断开与重复 shutdown。真实 backend 只有新 daemon monitor 收到目标 fullname Announce 才让变更 update 返回成功。
- 慢客户端阻塞或消费其他客户端事件：每 socket 使用独立 subscription、send task 与有界 queue；真实 WSS 测试让一个客户端在握手后停止读取并持续发送合法的大响应请求，验证该 session 因 backpressure 有界退出，同时健康 socket 仍得到正常 response。双客户端订阅测试另行验证未订阅连接保持静默、订阅连接各自收到同 cursor event，业务 response 不串 socket。
- 撤销后旧 socket 继续使用：DELETE 先持久撤销 bearer，再取消对应 credentialId 的本地 sessions；真实 WSS 测试以同一 credential 建立两个 socket，验证两者均有界断线、旧 bearer 拒绝重连且其他 bearer 仍可请求。
- REST DTO 被误加成 Gateway method：生成器测试断言 pairing/credential 不出现在 method manifest。
- compat 或桌宠链机械漂移：`protocol:check`、Tauri `cargo check --lib --locked` 与最终边界 diff 审计确认无改动。

## 测试计划

- `npm run protocol:check`：schema、standalone fixtures、生成目标与 freshness。
- `cargo test --manifest-path sdk/rust/Cargo.toml -p codepet-gateway-sdk`：handshake identity 与 LAN DTO fixture 解码。
- `npm test --prefix sdk/typescript/codepet-gateway-sdk`：Gateway v1 TypeScript 生成类型编译。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test manager_gateway`：Host 注入 identity 的 handshake 构造。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test remote_lan_listener`：真实 loopback TLS、证书 pin、advertised authority、pairing、WSS dispatch、binary/超限 frame、安全 backpressure 关闭、replay/live、多客户端、同 credential 双 socket 撤销和 shutdown。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --lib remote_mdns`：纯构造/校验、TXT exact set、实际 port、非本机 endpoint fail-closed、同源 API、同值 no-op、旧 resend 竞态屏障、unregister error/timeout 与 Announce/idle 失败生命周期；macOS 同一测试集另以唯一 instance、不启动 browse，验证 start Announce、`pair=0→1` 的 unregister/new-daemon/new-Announce 顺序和 shutdown。
- `cargo check --manifest-path src-tauri/Cargo.toml --lib --locked`：兼容调用方与 Tauri 机械生成边界。

## 知识沉淀

本页记录 Phase 2A1 生成契约、Phase 2A2 listener 运行边界与 Phase 2A3 mDNS 发现边界。Tauri/UI 落地后只补接线与平台证据；若证书轮换或 credential 权限进入范围，另立架构决策，不在此 DTO 或 TXT 上追加隐式字段。

## 未知项

- Tauri 尚未创建、持有或按序 shutdown `RemoteLanServerHandle` 与 `RemoteLanMdnsAdvertiser`，也未在 pairing start/cancel/consume 时更新 `pair`；当前 Host 能力不代表 App 已开放 LAN 服务。
- 手机等真实设备跨 LAN 的证书 pin、系统防火墙与网络切换行为仍待 mDNS/Tauri 阶段验证。
