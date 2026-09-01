# Gateway SDK、通道与准入边界

## 结论

Remote 接入被拆成四个单向依赖层：discovery 只发现候选地址；channel 只建立经过 TLS pin 的 HTTPS/WSS 字节通道；LAN admission 通过一次性 pairing secret 换取设备级 credential；Gateway v2 在已准入通道上运行 JSON-RPC 2.0 业务协议。准入成功后可以访问 Host 上全部 Provider，不存在 Provider ACL；每个 Provider 资源始终携带 `deviceId + providerPluginId + providerInstanceId`，资源再增加 `nativeResourceId`。

Gateway v2 的 schema/manifest 是业务协议唯一事实来源。`cp-sdk-gen` 从同一输入生成 Dart client 或 Rust client/server：client 生成 typed method wrapper、请求信封和响应解码；server 生成 trait、dispatcher、严格 JSON-RPC 分类和错误映射，业务实现仍由 Host 手写。通道实现不得枚举 Gateway method，Gateway handler 不得读取 bearer、证书或 mDNS 状态。

## 证据

- `protocol/gateway/v2/{schema,manifest}.json`：Gateway 业务 DTO、11 个方法、7 个 replayable event 与 WebSocket text framing。
- `protocol/channel/lan/v1/{schema,manifest}.json`：QR、pairing exchange、LAN Host identity 与 current credential revoke DTO；不声明 Gateway method。
- `crates/codepet-host/src/remote_listener.rs`：pairing/credential 保持 `/remote/v1/...`，Gateway WSS 使用 `/remote/v2/gateway`；首个业务请求必须是 handshake。
- `sdk/rust/codepet-gateway-sdk`：Host 实现生成的 server trait，事件与响应使用 JSON-RPC 2.0。
- `sdk/dart/codepet-gateway-sdk`：生成 transport-neutral `ProtocolClient`；Remote 的 pinned WebSocket 只移动请求信封、响应信封与 notification。
- `tools/cp-sdk-gen/cp-sdk-gen.mjs`：支持 Provider server、Gateway client/server/both、LAN models，并记录 schema digest 与 role。

## Remote 分层

```text
UI / device session
        |
Gateway domain adapter ---- generated Gateway client SDK
        |                              |
        +-------- channel contract ----+
                       |
            pinned WSS / localhost / future WebRTC

LAN admission ---- generated LAN models ---- pinned HTTPS
discovery ---------------------------------- mDNS
```

新的接入端只需实现 channel contract，并按所选环境实现 discovery/admission；Gateway 的请求包装、JSON-RPC method、DTO、事件解码和 server dispatcher 均由 SDK 提供。WebRTC 或 localhost 可以替换通道，但不能改变 Gateway schema，也不能把 pairing secret 放进 Gateway handshake。

## 身份与信任约束

- `certSha256`/`identityFingerprint` 属于 LAN channel/admission，只用于校验实际 TLS peer；Gateway handshake 不重复携带证书指纹。
- credential 绑定逻辑 client identity。WSS 首次 `protocol.handshake.clientId` 必须与 bearer 绑定的 client 一致。
- Gateway Host identity 只包含 `deviceId + DeviceDescriptor`。
- Provider route 从业务资源获得。`turn.send` 不再同时携带一份可冲突的 route 和 conversation route。
- 旧 Remote 存储的 `/remote/v1/gateway` endpoint 在读取时迁移到 `/remote/v2/gateway`；pairing 与 credential REST 路径仍为 v1。

## 风险与验证

- 风险：channel 再次手写 method 或业务 DTO。验证：Remote Gateway client 只调用生成 `ProtocolClient`，WebSocket transport 只接收/返回 JSON object envelope。
- 风险：Gateway server 重新依赖 LAN 凭据。验证：bearer 校验发生在 listener upgrade，`ProviderGatewayService` 只接收 `GatewayHostIdentity`。
- 风险：Provider 身份在 Remote 聚合时丢失。验证：Host/Remote 测试覆盖多实例 route、事件、分页、conversation get 与 turn send。
- 风险：生成 client/server 实际输出相同。验证：`cp-sdk-gen` 测试断言 server role 有 trait 且没有 `ProtocolClient`，Dart client 有 typed client。
- 风险：配对或升级破坏已有安装。验证：LAN listener 纵向 TLS/WSS 测试、Remote pairing 测试与 stored endpoint migration。

## 验证命令

```sh
npm run protocol:check
npm run sdkgen:test
npm run providers:test
cargo test --manifest-path crates/codepet-host/Cargo.toml
flutter analyze
flutter test
```

Remote 的后两条命令在 `codepet-remote` 仓库执行。App 构建还必须从最终 bundle 的 `provider-sdk/` 资源运行一次 `cp-sdk-gen` 并编译导出结果，防止 staging 与源码检查脱节。
