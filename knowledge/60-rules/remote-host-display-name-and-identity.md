# Remote Host 显示名与稳定身份规约

## 规则

macOS Host 的可见名称必须读取系统 ComputerName，Windows Host 必须读取原生计算机名称；设备名称和系统描述只是可刷新的展示元数据，不能参与稳定身份判断。Remote Host 的稳定身份仍由持久化 `deviceId` 与配对证书指纹共同确定。

## 适用场景

本规则适用于 Tauri 启动时创建设备 registry，以及 Remote pairing QR、pairing exchange、`protocol.handshake` 和 mDNS TXT `name` 的 Host 信息投影。Gateway v1 的 exchange/handshake 使用 `DeviceDescriptor.deviceName/operatingSystem/systemVersion`，QR 契约只携带 `displayName`；不得绕过 Schema 或把平台信息拼进名称。

## 反例

- 把 `This Device`、`This Mac` 或其他固定文案写入生产设备 registry，会让所有 Mac 在 Remote 中显示成同名 Host。
- 使用 hostname、网络地址或 `displayName` 作为设备身份，会让改名、DHCP、VPN 或网络切换造成错误的新设备或身份漂移。
- QR、exchange、handshake 和 mDNS 分别读取不同名称来源，会让配对前后的同一 Host 显示不一致。

## 推荐做法

- macOS 使用 SystemConfiguration 的 `SCDynamicStoreCopyComputerName`，不启动 shell；空值或 API 不可用时使用无身份语义的安全 fallback。
- Windows 在 `platform/host_identity.rs` 内使用 `GetComputerNameW`，按 UTF-16 解码有效长度；原生调用失败时统一回退为 `CodePet Host`。业务调用方继续只调用 `computer_name()`。
- `DeviceRegistry::open` 可刷新持久化 `displayName`，但必须保留原 `deviceId/createdAt`；名称变化不得触发 TLS identity 或 credential 重建。
- Tauri 用 ComputerName 和系统信息构造一个 `DeviceDescriptor` 并注入 `RemoteAccessManager`。QR 使用其中的设备名，exchange/handshake 返回完整 descriptor；mDNS 可发布设备名与候选 TLS 指纹，但二者都不能单独建立信任。首次发现配对必须通过双方数字比较和 Host 接受，已配对连接仍核对持久化证书指纹。

## 来源

- 用户真机观察到 CodePet Remote 把 Host 显示为通用名称。
- Windows 的 `native_computer_name()` 曾落入非 macOS 分支直接返回 `None`，导致手机收到固定的 `This Device`；修复点位于 Host 平台层，Remote 已在握手后刷新 descriptor，无需修改手机端显示逻辑。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs` 曾在生产启动路径硬编码名称；`crates/codepet-host/src/device.rs` 又在后续启动忽略新名称，因此旧通用值会持续存在。
- pairing、listener handshake 与 mDNS 的现有实现均源自同一个 `RemoteHostIdentity`，无需第二套身份存储。

## 验证方式

- 名称解析单元测试覆盖可注入的原生值、空值与 fallback。
- Windows 原生测试确认系统名称非空、不含 NUL，并与本机 `COMPUTERNAME` 一致。可独立运行 `rustc --edition 2021 --test src-tauri/src/platform/host_identity.rs -o <临时测试程序>` 后执行测试程序。
- 真机验收需运行重新构建的 Windows Host，再让手机重连，核对设备详情与电脑系统名称一致，且原配对仍可使用。
- `device_catalog` 测试验证名称刷新后 `deviceId/createdAt` 不变且文件已更新。
- 真实 loopback listener 测试验证 pairing exchange 与 handshake 返回完整相同的 Host identity，且 descriptor 的名称、OS、系统版本保持一致；mDNS 单元测试继续核对 TXT `name`。
- QR copy 测试验证只复制当前 active payload，普通 Tauri view、UI 文本和日志不出现 pairing secret。
- 运行 `npm run protocol:check`，证明 Gateway Schema 和生成代码保持同步。
