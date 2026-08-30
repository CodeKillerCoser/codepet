# Gateway v1 LAN 生成协议契约

## 背景

`codepet-host::RemoteAccessManager` 已具备持久 TLS leaf identity、五分钟内存 pairing session 和只落盘 bearer SHA-256 的 credential core，但 Gateway v1 尚未给 Remote Client 一份可生成的 Host identity、QR 与 REST body 契约。后续 listener 若自行手写 DTO，会让 Rust Host、TypeScript/移动客户端和协议文档产生多个事实来源。

## 目标

- 从 `protocol/gateway/v1/schema.json` 同时生成 Rust 与 TypeScript 的 LAN DTO。
- 保留 `HandshakeRequest.clientId` 作为 remote client 唯一协议身份，并要求 transport 将其与 pairing credential 绑定。
- 让 handshake 和 pairing response 使用同一 `RemoteHostIdentity`，其中证书指纹语义唯一。
- 固定 REST、WSS、mDNS 与 QR wire 边界，供下一阶段 listener/UI 直接消费。

## 非目标

- 不实现 TLS listener、HTTP route、WebSocket session、mDNS publisher 或 Tauri/UI 接线。
- 不修改 `gateway/v1/compat-v0.*`、Desktop IPC companion、Pet、activity projection 或 Provider 业务 DTO。
- 不增加 RBAC、credential scope、refresh token、证书轮换或权限模型。

## 现状理解

`DeviceRegistry` 仍是 `deviceId/displayName` 唯一事实来源；`RemoteAccessManager` 持有 leaf certificate DER 与 fingerprint、pairing session 和 credential store。`ProviderGatewayService` 实现 Gateway v1 业务 dispatch，但认证属于未来 LAN transport，服务层不能接收 bearer。现有 `devices/providers` handshake 集合仍有调用方，不能因增加本机身份而重构或删除。

## 实现路径

固定网络入口如下：

```text
POST   /remote/v1/pairings/{pairingId}/exchange
GET    /remote/v1/gateway                    # WSS Upgrade
DELETE /remote/v1/credentials/current
```

`PairingExchangeRequest` 只包含 `pairingSecret/clientId/clientName/platform`；`pairingId` 来自 path。响应只包含 `device/gatewayUrl/credential`，credential 对客户端 opaque。DELETE 返回最小 `{ revoked }` DTO。以上类型属于同一 Gateway schema 的独立生成类型，不进入 envelope method manifest。

QR 只编码 `PairingQrPayload`：`version/hostDeviceId/displayName/httpsBaseUrl/certSha256/pairingId/pairingSecret/expiresAt`。PairingOffer 的状态、倒计时等只留在 Host/UI 内存。`pairingSecret` 明文只能进入 QR encoder，不在普通 UI 文本、日志或持久文档中展示。

`RemoteHostIdentity` 固定为 `deviceId/displayName/identityFingerprint`。`identityFingerprint` 与 QR `certSha256` 都是 leaf certificate DER SHA-256 的 64 位小写 hex。`HandshakeResponse.device` 必填，同时保留既有 `devices/providers`。当前进程内 Host 构造使用明确的全零 transport-neutral placeholder；该值不代表网络身份。未来 LAN listener 必须通过专用构造入口注入 `RemoteAccessManager` 的真实 fingerprint，再开放网络连接。

连接次序固定：先验证 TLS peer leaf DER fingerprint；WSS Upgrade 校验 bearer；第一条业务请求必须是 `protocol.handshake`；listener 校验 handshake `clientId` 等于 credential 绑定的 `clientId`；客户端再核对 response `device.deviceId` 与 `device.identityFingerprint`。`ProviderGatewayService` 只接收已经构造好的 `RemoteHostIdentity`，不感知 bearer 或 Authorization header。

mDNS service type 为 `_codepet._tcp.local.`，TXT 仅允许 `id/name/vmin/vmax/pair`。SRV/A/AAAA 负责 endpoint 发现，TXT 不携带 certificate fingerprint、secret、credential、Provider、项目或会话。

## 涉及模块

- `protocol/gateway/v1/schema.json` 与 `fixtures/`：LAN DTO 与可验证 wire 样例的唯一事实来源。
- `tools/protocol-codegen/`：支持 standalone type fixture 与 fingerprint pattern 校验，保证 Rust/TypeScript freshness。
- `sdk/rust/codepet-gateway-sdk`、`sdk/typescript/codepet-gateway-sdk`：生成 DTO 与语言侧编译测试。
- `crates/codepet-host/src/remote_access.rs`：消费生成 `PairingExchangeRequest`，不保留手写同义 DTO。
- `crates/codepet-host/src/gateway.rs`：构造必需 handshake `device`；认证仍留在 transport 外层。

## 风险

- 指纹格式漂移：schema pattern、fixture、Rust SDK 解码与 Host TLS 定向测试共同验证 64 位小写 hex。
- `clientId` 出现同义字段：生成器测试断言 request 只有 `clientId`，无 `remoteClientId`；listener 后续需验证 credential 绑定。
- 进程内 placeholder 被误用于 LAN：构造入口注释和 manager_gateway 测试区分 placeholder 与注入 identity；listener 验收必须核对真实 TLS fingerprint。
- REST DTO 被误加成 Gateway method：生成器测试断言 pairing/credential 不出现在 method manifest。
- compat 或桌宠链机械漂移：`protocol:check`、Tauri `cargo check --lib --locked` 与最终边界 diff 审计确认无改动。

## 测试计划

- `npm run protocol:check`：schema、standalone fixtures、生成目标与 freshness。
- `cargo test --manifest-path sdk/rust/Cargo.toml -p codepet-gateway-sdk`：handshake identity 与 LAN DTO fixture 解码。
- `npm test --prefix sdk/typescript/codepet-gateway-sdk`：Gateway v1 TypeScript 生成类型编译。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test manager_gateway`：Host 注入 identity 的 handshake 构造。
- `cargo check --manifest-path src-tauri/Cargo.toml --lib --locked`：兼容调用方与 Tauri 机械生成边界。

## 知识沉淀

本页是 Phase 2A1 的可执行功能设计。listener/mDNS/Tauri/UI 落地后应在本页补真实 route 与端到端证据；若证书轮换或 credential 权限进入范围，另立架构决策，不在此 DTO 上追加隐式字段。

## 未知项

- listener 的 session 状态机、错误到 HTTP/WSS close code 的映射尚未实现。
- mDNS 在 macOS/Windows 的具体库和接口权限尚未选择。
- 真实设备上的 TLS pin、pairing exchange 与撤销端到端行为仍待下一阶段验证。
