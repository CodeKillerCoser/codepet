# Gateway v1 与 LAN Channel 生成协议契约

## 背景

`codepet-host::RemoteAccessManager` 已具备持久 TLS leaf identity、五分钟内存 pairing session 和只落盘 bearer SHA-256 的 credential core，Gateway SDK 已生成 Host identity、QR 与 REST body 契约。Phase 2A2 在这些既有事实上增加可复用的 LAN TLS/HTTP/WSS listener；Phase 2A3 增加只消费同一 identity 与 listener 实际 endpoint 的 LAN mDNS advertiser。两者都不生成第二份设备、证书身份或端口。

## 目标

- 从 `protocol/gateway/v1` 生成 Rust/Dart Gateway v1 SDK，从 `protocol/channel/lan/v1` 生成 Rust/Dart LAN admission DTO；两层只通过 Core 类型共享稳定身份字段。
- 保留 `HandshakeRequest.clientId` 作为 remote client 唯一协议身份，并要求 transport 将其与 pairing credential 绑定；设备展示信息统一使用 `DeviceDescriptor`，不重复造 ID。
- 让 pairing exchange 与 handshake 双向都使用同一最小设备展示信息；证书指纹和稳定 device ID 只存在于 LAN admission/transport，不进入 Gateway 握手响应。
- 以单个 TLS listener 落实固定 REST/WSS wire 边界，并通过 `_codepet._tcp.local.` 发布同一 listener 的实际 LAN IP 与 TLS port。
- 保持 mDNS 只负责发现；Tauri 生命周期与 pairing 状态接线见 `remote-access-tauri-runtime.md`，frontend UI 仍留给后续阶段。

## 非目标

- 不实现 frontend UI 或 Pet/Desktop IPC；Tauri 后端生命周期由 Phase 2B1 配套文档补充。
- 不修改 `desktop/v0`、Desktop IPC companion、Pet、activity projection 或 Provider 业务 DTO。
- 不增加 RBAC、credential scope、refresh token、证书轮换或权限模型。

## 现状理解

`DeviceRegistry` 仍是稳定 `deviceId/displayName` 唯一事实来源；Tauri 启动时把该 display name 与一次 OS/system version 探测组合为进程内稳定 `DeviceDescriptor`，再注入 `RemoteAccessManager`。Manager 持有 leaf certificate DER 与 fingerprint、pairing session、descriptor 和 credential store。`ProviderGatewayService` 实现 Gateway v1 业务 dispatch，listener 只在 transport 层验证 bearer、绑定 socket clientId 和维护撤销取消；服务层不接收 bearer 或 TLS 状态。Gateway 握手只返回单个 `device` 展示对象和 `ProviderSummary[]`，不再返回 `devices` 集合。

## 实现路径

固定网络入口如下：

```text
POST   /remote/v1/pairings/{pairingId}/exchange
GET    /remote/v2/gateway                    # WSS Upgrade
DELETE /remote/v1/credentials/current
```

`PairingExchangeRequest` 只包含 `pairingSecret/clientId/device`；`device` 精确为 `DeviceDescriptor { deviceName, operatingSystem, systemVersion }`，`pairingId` 来自 path。响应只包含 `device/gatewayUrl/credential`，其中 Host identity 内嵌相同 descriptor，credential 对客户端 opaque。DELETE 返回最小 `{ revoked }` DTO。以上类型属于同一 Gateway schema 的独立生成类型，不进入 envelope method manifest。

QR 只编码 `PairingQrPayload`：`version/hostDeviceId/displayName/httpsBaseUrl/certSha256/pairingId/pairingSecret/expiresAt`。PairingOffer 的状态、倒计时等只留在 Host/UI 内存。`pairingSecret` 明文只能进入 QR encoder，不在普通 UI 文本、日志或持久文档中展示。

`LanHostIdentity` 固定为 `deviceId/descriptor/identityFingerprint`。`identityFingerprint` 与 QR `certSha256` 都是 leaf certificate DER SHA-256 的 64 位小写 hex。`HandshakeRequest.device` 与 pairing request 复用 Core `DeviceDescriptor`；Gateway `HandshakeResponse.device` 只投影 `name/operatingSystem/systemVersion`。LAN listener 启动前要求调用方通过 `ProviderGatewayService::with_remote_identity` 显式注入经 admission 校验的 Host identity；TLS 配置直接读取同一个 manager 已持久化的 certificate/private key DER。

连接次序固定：客户端先 pin TLS peer leaf DER；WSS Upgrade 必须携带有效 bearer；第一条业务请求必须是 `protocol.handshake`；listener 校验 handshake `clientId` 等于 credential 绑定的 `clientId`，并在成功响应发送前原子刷新该 credential 的 descriptor；客户端以 pairing/TLS admission 保存稳定 Host 身份，并用 handshake `device` 更新展示信息。后续请求统一交给生成 SDK 的 envelope dispatcher，不在 listener 复制 conversation/turn/approval 路由。

`event.subscribe` 是每条 socket 的显式推送门。listener 在成功响应前先从现有 EventPublisher 建立 replay/live subscription，响应入队后才启动该 socket 的 event send loop；`GatewayEventSubscription` 以 cursor 去除 receiver 与 replay 窗口交叠，因此顺序是 replay 后 live 且不重复。每条 socket 最多成功订阅一次，未订阅 socket 不收到 server event，但仍可执行普通请求。

`RemoteLanServerConfig::default()` 的 bind address 是 `0.0.0.0:0`，但 bind address 与客户端可见 authority 分离：wildcard bind 必须由调用方显式提供第一代具体 `advertised_host`，否则 start fail-closed；listener 把实际端口与客户端可见 IP/DNS host 组成 `RemoteLanAdvertisedEndpoint`，绝不发布 `0.0.0.0`。运行期 handle 可在同一 listener 上 stage/commit/withdraw endpoint；pairing response、status 和 QR 都读取该共享 generation。真实网络测试显式绑定 loopback 并传入 `127.0.0.1`；Tauri v1 通过环境覆盖或 no-send route probe 选择 active 本机 IPv4，细节见 `remote-access-tauri-runtime.md`。

每条 socket 独立持有 credential clientId、握手状态、订阅和容量为 64 的 outbound queue；writer 单次发送有一秒上限，response/event 入队有两秒上限，任一 transport backpressure 只取消该 session。listener 以全局 32 permit semaphore 限制并发 WSS session，超限 upgrade 返回稳定 503。`DELETE current` 只撤销发起 bearer，并按非敏感 credentialId 取消对应 listener-local sessions；同一 credential 的所有 socket 都关闭，其他 credential 不受影响。显式 shutdown 先停止新 session、取消现有 socket，再有界关闭 server task；TLS handshake 自身也有小于 server shutdown 窗口的上限。坏 JSON、binary frame、超限 frame 和协议次序错误只产生固定 Gateway error、WebSocket close 或安全断开，不进入 panic 路径。

mDNS 使用仍在维护且跨 macOS/Linux/Windows 的纯 Rust `mdns-sd 0.21`；Host 不需要异步或 logging feature。service type 精确为 `_codepet._tcp.local.`，TXT 精确且仅有 `id/name/vmin/vmax/pair`：`id/name` 只读取 `RemoteLanServerHandle` 导出的 opaque advertisement source，candidate endpoint 也必须属于同一 listener id，因此不能拼接 manager A 的 identity、listener B 的端口与 endpoint C；`vmin/vmax` 都是 Gateway `PROTOCOL_VERSION=1`，`pair=1` 只表示 Host 当前主动开放一次性 pairing，否则为 `0`。SRV/A/AAAA 使用同一 source 的实际 TLS port 和 candidate IPv4；DNS host、unspecified/multicast/documentation IP、与具体 bind 不一致的 IP 或地址族均 fail-closed。显式 IP 还必须存在于 `if-addrs` 返回的 active 本机接口集合，接口枚举只做归属校验，不代替调用方猜公网、LAN 或 loopback 地址；测试可显式使用匹配的 loopback。

instance name 由 Host descriptor 的可读 `deviceName` 加 device id 的短哈希冲突后缀组成，hostname 使用相同冲突后缀；`mdns-sd` 默认 probe 继续处理极小概率的局域网名称冲突。instance、hostname 与 IP 都只是发现元数据，稳定身份只能读取 TXT `id`，TLS 信任仍只能来自 pairing 后的证书 pin 与 handshake 核对。TXT 不携带 OS/version、certificate fingerprint、secret、credential、Provider、项目或会话。

`mdns-sd::ServiceDaemon::register` 只保证命令入队，因此 advertiser 在 start 和 pair 值变化后通过 daemon monitor 等待固定短窗口内目标 fullname 的 `DaemonEvent::Announce`；只有实际发送产生该事件才返回成功。`DaemonEvent::Error`、monitor 断开或超时都会注销 service 并停止 daemon；idle 期间积累的 error/断开由下一次值变化 update 或 shutdown 读取并执行同样的 fail-closed 清理。同值 update 是严格 no-op，不读取 backend。

fullname 本身不能区分注册代际，旧 daemon 的 `RegisterResend` 可能在普通 drain 后产生同名 Announce。`RemoteLanMdnsAdvertiser` 因此把 IP/interface 与 `pair` 都视为一个完整 generation：任一字段变化时先等待旧 service 的 `UnregisterStatus::OK/NotFound` 终态 ack，再 drain ack 前的 monitor 事件、停止旧 daemon并创建新 daemon，最后以同一个 fullname 注册完整新 service 并等待新 monitor 的 Announce。该顺序可证明不会把旧代 Announce 当成新代确认；unregister 到新 Announce 之间允许短暂发现空窗。

Tauri 在一个 lifecycle mutex 下串行网络与 pairing 变化。地址切换先在 listener stage pending endpoint，再执行上述 mDNS 替换；Announce 成功后才把 endpoint、status、active QR 和 advertiser 一起提交。Announce 已发送而提交尚未完成时，请求 `Host` authority 匹配 pending IP 的 pairing exchange 已使用 pending `gatewayUrl`，所以不存在发现 B 却返回 A 的窗口。失败时 advertiser fail-closed、pending 清除且 obsolete committed endpoint 撤下，listener/TLS/session/credential/Gateway 不停止；下一次低频网络检查可从干净 daemon 重试。`shutdown` 检查 monitor 后注销 service 并停止 daemon，成功停止后的重复调用是 no-op，Drop 只做同样的 best-effort 收尾。

## 涉及模块

- `protocol/gateway/v1/{schema.json,manifest.json,fixtures/}`：Gateway v1 业务 RPC 与可验证 wire 样例的唯一事实来源。
- `protocol/channel/lan/v1/{schema.json,manifest.json,fixtures/}`：LAN admission DTO 与样例的唯一事实来源。
- `tools/protocol-codegen/`：支持 standalone type fixture、fingerprint pattern、normalized IR 与判别联合校验，保证 Rust/TypeScript/Dart freshness。
- `sdk/rust/codepet-gateway-sdk`、`sdk/dart/codepet-gateway-sdk`：Gateway v1 server/client DTO、trait 与 fixture 测试。
- `sdk/rust/codepet-lan-channel-sdk`、`sdk/dart/codepet-lan-channel-sdk`：LAN admission DTO 与 fixture 测试。
- `crates/codepet-host/src/remote_access.rs`：消费生成 `PairingExchangeRequest`，不保留手写同义 DTO。
- `crates/codepet-host/src/gateway.rs`：构造必需 handshake `device`；认证仍留在 transport 外层。
- `crates/codepet-host/src/remote_listener.rs`：单 TLS listener、固定 REST/WSS route、per-socket 状态、撤销取消和有界 shutdown。
- `crates/codepet-host/src/remote_mdns.rs`：只从 listener-derived source/candidate endpoint 构造唯一 service，以本机接口与 Announce 证明发布，并通过 unregister ack 与新 daemon 代际更新地址和 pairing availability。
- `crates/codepet-host/src/remote_listener.rs`：持有 committed/pending endpoint，并让新 generation Announce 窗口内的 pairing exchange 返回同代 gateway URL。
- `crates/codepet-host/tests/remote_lan_listener.rs`：真实 loopback TLS pin 与完整 WSS/撤销/多客户端证据。

## 风险

- 指纹格式漂移：schema pattern、fixture、Rust SDK 解码与 Host TLS 定向测试共同验证 64 位小写 hex。
- `clientId` 出现同义字段：生成器测试断言 request 只有 `clientId`，无 `remoteClientId`；真实 WSS 负例验证 credential 绑定。
- 无 identity 的进程内服务被误用于 LAN：Gateway v1 handshake fail-closed，listener start 拒绝与 manager identity 不一致的 service；loopback 测试核对真实 TLS leaf 指纹、pairing response 与 handshake identity 三者相同。
- wildcard bind 被误当作可访问 endpoint：start 在缺少 advertised host 时 fail-closed；测试分别验证 `0.0.0.0:0` bind 与 `listener.local:<actual-port>` handle URL，并用显式 `127.0.0.1` 完成真实网络闭环。
- 发现数据被误当身份或泄露敏感上下文：构造测试断言 service type、实际 port 与 TXT exact set；`fp` 只能作为首次 HTTPS 的候选 pin，必须经双方数字比较和 Host 接受才签发 credential。secret、bearer、Provider、项目和会话上下文不得进入 service；已配对客户端仍只信任持久化 pin 与 handshake。
- identity 与 endpoint provenance 被错误拼接：公开 start API 只接受 listener handle，编译形状测试固定该边界；真实 listener 测试核对 handle identity 等于启动它的 manager identity。
- 广告不可达或串到错误接口：advertiser 只接受与 listener bind 匹配、存在于 active 本机接口集合的显式 unicast IP，并将 service 限定到该地址；确定性负例覆盖 DNS host、unspecified、bind mismatch 与非本机 IP，macOS smoke 对 start 和 `pair=0→1` 各等待目标 fullname 的真实 Announce；runtime fake publisher 另覆盖 A→B、无地址撤销、失败重试与 IP/pair 竞态。实际跨设备发现和防火墙仍需真机验证。
- daemon 入队或旧代 resend 被误报为新发布成功：fake backend 把旧 Announce 精确注入到 update 的首次 drain 与 unregister ack 之间，断言 ack 后 drain 和新 daemon 会丢弃它，且没有新 Announce 时必须超时；另覆盖 unregister error/timeout 清理、同值 no-op、Announce、idle 断开与重复 shutdown。真实 backend 只有新 daemon monitor 收到目标 fullname Announce 才让变更 update 返回成功。
- 慢 Provider 或客户端阻塞 control plane：每 socket 使用持续 poll 控制帧的 reader、握手后有界并发 request dispatcher、独立 subscription/send task 与有界 queue；真实 WSS 测试让 `conversation.get` 保持 pending 并验证同 socket 仍及时 Pong，再让一个客户端停止读取并持续发送合法的大响应请求，验证该 session 因 backpressure 有界退出，同时健康 socket 仍得到正常 response。双客户端订阅测试另行验证未订阅连接保持静默、订阅连接各自收到同 cursor event，业务 response 不串 socket。
- 撤销后旧 socket 继续使用：DELETE 先持久撤销 bearer，再取消对应 credentialId 的本地 sessions；真实 WSS 测试以同一 credential 建立两个 socket，验证两者均有界断线、旧 bearer 拒绝重连且其他 bearer 仍可请求。
- REST DTO 被误加成 Gateway method：生成器测试断言 pairing/credential 不出现在 method manifest。
- compat 或桌宠链机械漂移：`protocol:check`、Tauri `cargo check --lib --locked` 与最终边界 diff 审计确认无改动。

## 测试计划

- `npm run protocol:check`：schema、standalone fixtures、生成目标与 freshness。
- `cargo test --manifest-path sdk/rust/Cargo.toml -p codepet-gateway-sdk -p codepet-lan-channel-sdk`：Gateway handshake 与 LAN DTO fixture 解码。
- `npm test --prefix sdk/typescript/codepet-desktop-sdk`：desktop-only v0 TypeScript 契约编译，确认没有重新引入旧 Gateway 协议。
- `dart analyze sdk/dart/codepet-core-sdk sdk/dart/codepet-gateway-sdk` 与 Gateway package 下的 `dart test`：null safety、canonical fixture round-trip、closed-object/constraint、判别 union、敏感字段与 typed client。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test manager_gateway`：Host 注入 identity 的 handshake 构造。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test remote_lan_listener`：真实 loopback TLS、证书 pin、advertised authority、pairing、WSS dispatch、binary/超限 frame、安全 backpressure 关闭、replay/live、多客户端、同 credential 双 socket 撤销和 shutdown。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --lib remote_mdns`：纯构造/校验、TXT exact set、完整 IP+pair generation、实际 port、非本机 endpoint fail-closed、同源 API、同值 no-op、旧 resend 竞态屏障、unregister error/timeout 与 Announce/idle 失败生命周期；macOS 同一测试集另以唯一 instance、不启动 browse，验证 start Announce、`pair=0→1` 的 unregister/new-daemon/new-Announce 顺序和 shutdown。
- `cargo check --manifest-path src-tauri/Cargo.toml --lib --locked`：兼容调用方与 Tauri 机械生成边界。

## 知识沉淀

本页记录 Phase 2A1 生成契约、Phase 2A2 listener 运行边界与 Phase 2A3 mDNS 发现边界。Phase 2B1 Tauri 后端接线见 `remote-access-tauri-runtime.md`；若证书轮换或 credential 权限进入范围，另立架构决策，不在此 DTO 或 TXT 上追加隐式字段。

## 未知项

- frontend 尚未调用 Tauri Remote commands；后端已创建、持有并按序 shutdown listener/mDNS，也已同步 pairing availability。
- 手机等真实设备跨 LAN 的证书 pin、系统防火墙与网络切换行为仍待 frontend/真机阶段验证。
