# Gateway v1 LAN 生成协议契约

## 背景

`codepet-host::RemoteAccessManager` 已具备持久 TLS leaf identity、五分钟内存 pairing session 和只落盘 bearer SHA-256 的 credential core，Gateway SDK 已生成 Host identity、QR 与 REST body 契约。Phase 2A2 在这些既有事实上增加可复用的 LAN TLS/HTTP/WSS listener；transport 不再手写第二套 DTO，也不生成第二份设备或证书身份。

## 目标

- 从 `protocol/gateway/v1/schema.json` 同时生成 Rust 与 TypeScript 的 LAN DTO。
- 保留 `HandshakeRequest.clientId` 作为 remote client 唯一协议身份，并要求 transport 将其与 pairing credential 绑定。
- 让 handshake 和 pairing response 使用同一 `RemoteHostIdentity`，其中证书指纹语义唯一。
- 以单个 TLS listener 落实固定 REST/WSS wire 边界，并保持 mDNS、Tauri/UI 为下一阶段接线。

## 非目标

- 不实现 mDNS publisher、Tauri/UI 启停接线或 Pet/Desktop IPC。
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

`RemoteLanServerConfig::default()` 绑定 `0.0.0.0:0`，测试使用显式 loopback；handle 返回实际地址、端口、`httpsBaseUrl` 与 `gatewayUrl`。每条 socket 独立持有 credential clientId、握手状态、订阅和有界 outbound queue。`DELETE current` 只撤销发起 bearer，并按非敏感 credentialId 取消对应 listener-local sessions；其他 credential 的 socket 不受影响。显式 shutdown 先停止新 session、取消现有 socket，再有界关闭 server task；TLS handshake 自身也有小于 server shutdown 窗口的上限。坏 JSON、binary frame、超限 frame 和协议次序错误只产生固定 Gateway error 或 WebSocket close，不进入 panic 路径。

mDNS service type 为 `_codepet._tcp.local.`，TXT 仅允许 `id/name/vmin/vmax/pair`。SRV/A/AAAA 负责 endpoint 发现，TXT 不携带 certificate fingerprint、secret、credential、Provider、项目或会话。

## 涉及模块

- `protocol/gateway/v1/schema.json` 与 `fixtures/`：LAN DTO 与可验证 wire 样例的唯一事实来源。
- `tools/protocol-codegen/`：支持 standalone type fixture 与 fingerprint pattern 校验，保证 Rust/TypeScript freshness。
- `sdk/rust/codepet-gateway-sdk`、`sdk/typescript/codepet-gateway-sdk`：生成 DTO 与语言侧编译测试。
- `crates/codepet-host/src/remote_access.rs`：消费生成 `PairingExchangeRequest`，不保留手写同义 DTO。
- `crates/codepet-host/src/gateway.rs`：构造必需 handshake `device`；认证仍留在 transport 外层。
- `crates/codepet-host/src/remote_listener.rs`：单 TLS listener、固定 REST/WSS route、per-socket 状态、撤销取消和有界 shutdown。
- `crates/codepet-host/tests/remote_lan_listener.rs`：真实 loopback TLS pin 与完整 WSS/撤销/多客户端证据。

## 风险

- 指纹格式漂移：schema pattern、fixture、Rust SDK 解码与 Host TLS 定向测试共同验证 64 位小写 hex。
- `clientId` 出现同义字段：生成器测试断言 request 只有 `clientId`，无 `remoteClientId`；真实 WSS 负例验证 credential 绑定。
- 无 identity 的进程内服务被误用于 LAN：Gateway v1 handshake fail-closed，listener start 拒绝与 manager identity 不一致的 service；loopback 测试核对真实 TLS leaf 指纹、pairing response 与 handshake identity 三者相同。
- 慢客户端阻塞或消费其他客户端事件：每 socket 使用独立 subscription 与有界 send loop；双客户端测试验证未订阅连接保持静默、订阅连接各自收到同 cursor event，业务 response 不串 socket。
- 撤销后旧 socket 继续使用：DELETE 先持久撤销 bearer，再取消对应 credentialId 的本地 sessions；测试验证有界断线、拒绝重连且其他 bearer 仍可请求。
- REST DTO 被误加成 Gateway method：生成器测试断言 pairing/credential 不出现在 method manifest。
- compat 或桌宠链机械漂移：`protocol:check`、Tauri `cargo check --lib --locked` 与最终边界 diff 审计确认无改动。

## 测试计划

- `npm run protocol:check`：schema、standalone fixtures、生成目标与 freshness。
- `cargo test --manifest-path sdk/rust/Cargo.toml -p codepet-gateway-sdk`：handshake identity 与 LAN DTO fixture 解码。
- `npm test --prefix sdk/typescript/codepet-gateway-sdk`：Gateway v1 TypeScript 生成类型编译。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test manager_gateway`：Host 注入 identity 的 handshake 构造。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test remote_lan_listener`：真实 loopback TLS、证书 pin、pairing、WSS dispatch、replay/live、多客户端、撤销和 shutdown。
- `cargo check --manifest-path src-tauri/Cargo.toml --lib --locked`：兼容调用方与 Tauri 机械生成边界。

## 知识沉淀

本页记录 Phase 2A1 生成契约与 Phase 2A2 listener 运行边界。mDNS/Tauri/UI 落地后只补接线与平台证据；若证书轮换或 credential 权限进入范围，另立架构决策，不在此 DTO 上追加隐式字段。

## 未知项

- mDNS 在 macOS/Windows 的具体库和接口权限尚未选择。
- Tauri 尚未创建、持有或 shutdown `RemoteLanServerHandle`，也未把实际 LAN endpoint 交给 pairing UI；当前只验证 Host 内真实 loopback，不代表 App 已开放 LAN 服务。
- 手机等真实设备跨 LAN 的证书 pin、系统防火墙与网络切换行为仍待 mDNS/Tauri 阶段验证。
