# 公网连接失败：先核对当前 Host 身份与信令配置

## 现象与证据

2026-11-06 用户在 Android 开启 WebRTC 后报告「没有公网信令」，当时无法分享手机日志。以下只基于本机、VPS 和源码，不把用户转述当成精确错误码。

- Mac 的 `~/Library/Application Support/code-pet/logs/code-pet.log` 在 02:21:27（UTC+8）记录新 Release 启动，无 RTC 记录。
- 当前 `connections/device-identity.json` 的 Host ID 为 `device-8d247c27-08d0-488a-a6eb-9d66942eb81c`；凭据文件有 02:43:51 新签发的手机凭据。`connections/rtc-cloud.json` 不存在。
- VPS 的 codepet-signal、coturn、nginx 均 active，SQLite hosts 表没有上述当前 Mac ID。
- VPS journal 在 02:41:49—02:50:19 收到 10 次 `offer.accepted`，均指向 `device-544a159c-5b27-45f2-8a0a-53ae6b1bae77`。同窗口该身份有 21 次 `/v1/ice` 200、10 次 `/v1/offers` POST 200，没有 answer 事件，也没有 Host 同步请求。另有独立测试身份的过期信箱清理，不能算用户请求。

## 结论与涉及模块

当前 Mac 未配置或注册公网信令，这是阻断该设备跨网连接的确定条件；不能由此判断 IPv4 NAT、IPv6 或 TURN 协议失败。VPS 接受另一个 Host 的请求也不代表当前 Mac 收到了请求。该身份究竟属于旧 Mac 配对还是另一台电脑，尚未确认。

- `src-tauri/src/runtime_gateway/tauri_bridge.rs`：从当前数据目录的 `connections/` 读取身份和凭据，不能只查旧 `remote-access/`。
- `crates/codepet-host/src/remote/channels/webrtc/cloud.rs::Cloud::load`：配置不存在返回 `None`。
- `crates/codepet-host/src/remote/channels/lan/listener.rs`：缺失或无效 cloud 配置均保留 LAN；`channel-bootstrap` 在未配置时返回 503 / `rtc_cloud_unavailable`。当前 load 错误被降为 None，缺失启动诊断，故「没有 RTC 错误日志」不能证明配置正常。
- Remote `routed_rtc_transport.dart`：安全存储无公网 token 时不会发起公网协商；LAN 成功后的授权 bootstrap 失败不影响 LAN。该失败只记录简略消息，尚需手机日志确认具体错误。

## 排查与验证路径

1. 确认运行进程、构建、实际数据目录及 Host ID，检查 cloud 配置是否存在；只输出 URL、身份、计数，不输出 token、seed、私钥。
2. 对照 VPS hosts/clients 元数据，并按 Host ID 和 attempt 查询同一时间窗口的 offer、answer、HTTP 状态，排除独立测试和其他电脑。
3. 当前身份需使用匹配的服务端授权和本地 cloud 配置，不能直接搬用绑定其他 Host ID 的旧 token。本次仅诊断，未修改授权或现有配对。
4. 修复配置后验证 Host 同步、手机在 LAN 获取授权或扫新版邀请，再测跨网；以双方 answer、选中 candidate pair 和 Gateway 握手为准。
5. 原生 Rust 探针通过不能替代已安装 App 的配置检查；Release 验收须包含实际 Host 注册和手机公网授权状态。手机日志恢复后再核对这次请求的设备选择及精确错误。

## 自动登记修复后的排查（2026-11-06）

上述证据是修复前的现场记录。新版无需手工创建 `rtc-cloud.json`：在设置填写服务 IP 后，App 创建 `connections/rtc-device-key.json`、`connections/rtc-services/<地址哈希>.json`，执行自动登记。先查 `signal.configuration`，再查 `signal.registration.start/complete/failed/rejected` 及 VPS `/v1/devices/register` 的状态码；确认 hosts 表包含当前 Mac ID。只检查 ID、role、时间和计数，禁止打印 seed、token 或完整配置。失败优先核对 TLS/地址、403/404 服务版本、409 身份冲突、429 限流；不要删除身份文件强行重置。

完成登记后继续检查手机配对授权和 offer/answer。已有手机可在 LAN 重连获取当前 Host 的公网授权；新设备扫当前邀请并经电脑确认。不同服务的旧凭据不能直接迁移，登记成功也不能作为跨网握手成功的证据。
