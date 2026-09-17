# LAN 与公网统一二维码邀请配对

## 背景

旧二维码只包含 Host HTTPS 地址和 TLS pin；Remote 必须先通过 LAN 交换 bearer，再访问 channel-bootstrap 获得公网信令授权。因此手机使用移动数据时，即使 VPS 信令和 TURN 正常，也无法首次配对。

## 目标

- 同一次扫码同时发起 LAN 与 VPS 申请，首个完成身份及结果验证的成功者胜出。
- Host 只确认一次、签发一份 credential；响应丢失后，相同申请可重新取回结果。
- VPS 不可达时，已配置公网能力的 Host 仍可完成 LAN 配对。

## 非目标

不引入账号、文件业务或新的 WebRTC 实现；不并发创建两套 PeerConnection，不改变 ICE 默认选路策略。

## 现状理解

`remote/access.rs` 的旧 QR exchange 会立即消费邀请；发现配对已有 pending/accepted/rejected/expired 状态和桌面确认入口。`cloud.rs` 使用每 credential 独立 token，并以已绑定 Ed25519 公钥验证 SDP。公网传输为 HTTPS 轮询，不是 WebSocket；签名 SDP 本身不是加密内容。

## 实现路径

### 二维码和兼容

LAN admission schema 允许 QR payload version 1 或 2。Host 配置 cloud 时生成 v2，在原字段上增加 `serviceUrl`、`hostPublicKey`；不具备 cloud 配置时仍生成原 v1。Remote 保留 v1 TLS exchange，v2 要求两项新字段、HTTPS 服务入口和合法 Ed25519 公钥。旧版严格 decoder 会拒绝 v2，不会静默降级。

生成 v2 时将本次邀请标记为必须确认，禁止通过旧 `/remote/v1/pairings/:id/exchange` 绕过确认。发现配对保留独立流程，不能消费已标为 v2 的邀请。

已加载 cloud 后如果邀请准备失败（例如资源上限），取消本次邀请并返回错误，不能吞掉错误回退为 v1 自动接受流程。

### 申请与加密

Remote 在 secure storage 中先保存每 Host 当前邀请的申请状态，再发送：随机 Ed25519 seed、256-bit requestId、请求 hash 和加密后的完整请求。重新扫描同一邀请使用完全相同的请求字节；新邀请替换该槽位。业务层只收到一个最终 registration。

请求 payload 绑定 `v=2, host, invitationId, requestId, expires, clientId, publicKey, device`，手机以对应私钥签名原始 UTF-8 JSON 字节。签名 envelope 再以 AES-256-GCM 加密。方向密钥为 SHA-256(`codepet-invite-v2:request:<secret>`) 或 SHA-256(`codepet-invite-v2:result:<secret>`)，AAD 是 UTF-8 `host:invitationId:requestId`。wire `sealed` 为标准 Base64(nonce12 || ciphertext || tag16)。secret 是 QR 中已有的 256-bit 随机十六进制串；该设计不提供前向保密。

VPS mailbox bearer 为 SHA-256(`codepet-invite-v2:mailbox:<secret>`) 的小写 hex；服务端只保存其 SHA-256。它只允许调用邀请 exchange，不能获得 TURN 凭据、同步 clients 或发送 SDP。VPS 不获得邀请 secret、手机 seed 或明文设备 bearer。

### 状态收敛

Host 解密并验证手机签名、身份和有效期后，以原始签名 payload 的 SHA-256 绑定邀请。锁内创建一条待确认申请；不同绑定被拒绝，同绑定返回现有状态。确认接受后签发一次 bearer，并绑定同一手机公钥到独立 cloud token。LAN 结果构造只需本地持久化，公网 clients 同步由既有后台循环完成，因此 VPS 失败不能阻塞 LAN 成功。

双方六位码沿用已有 SHA-256 数字比较算法；v2 的 requestId 参数使用手机 nonce，clientNonce 参数也使用同一 nonce，第三项为 QR TLS pin。Host 内部用于状态读取的随机 request ID 不向 VPS 暴露。Remote 通过配对层的确认码回调显示数字，业务层不感知 LAN/VPS。

结果再次绑定 Host、邀请、requestId、requestHash、有效期和终态；accepted 附 pairing 和 cloud bootstrap。Host 签名后加密，并缓存一份不可变密文。Remote 验证 AEAD、QR 中的 Host 公钥签名、全部绑定和 endpoint/pin 后才认为该路径成功。错误、pending、被拒绝或无效签名都不能赢得竞速。胜出后关闭两路 HTTP client，不发送撤销凭据请求。

VPS 邮箱按 Host 和邀请 ID 隔离，保留至邀请到期；相同密文幂等，不同 requestId/密文冲突。Host 拉取直到终态，丢失轮询响应不会使请求失联；终态写入也幂等且不可变。邀请定期重新发布，服务重启不要求更换二维码。邮箱数量、消息大小、TTL 和请求速率均有上限。

### 后续连接

配对结果中已包含 cloud 授权，移动数据首次配对后可直接进行公网信令。当前 RoutedRtcTransport 保留 LAN 尝试后 cloud 回退：两路现有 connect 都会创建 PeerConnection，直接竞速会产生重复 Host session，因此本次不并行建连。CloudRtcSignaling 默认不设置 relay-only，公网信令仍可通过 ICE 选择 LAN 直连。

## 涉及模块

- `protocol/channel/lan/v1/schema.json` 和生成 SDK：定义新 QR 元数据及兼容解码。
- `remote/access.rs`：统一申请绑定、单次签发、过期/取消后的确认约束。
- `remote/channels/webrtc/cloud/invitation.rs`：加密签名验证、结果封装、邀请同步。
- `remote/channels/lan/listener.rs`、Tauri remote_access：生成邀请并暴露 LAN 同协议入口。
- `services/signaling/server.py`：未配对客户端的临时、隔离、受限邮箱。
- Remote `pairing/`、`cloud_signaling.dart`：竞速、校验、重试状态与授权保存。

## 风险

- 请求重放或双重签发：Host 状态机和 TLS 并发集成测试验证一条 pending、一份 credential、相同终态。
- 取消后旧弹窗授权：状态机测试要求 expired 且不签发。
- 中间人替换结果：Remote 测试篡改签名、Host、邀请、请求、hash、过期和 bootstrap 公钥，均应拒绝。
- VPS 重启或响应丢失：邀请重复发布、重复拉取及结果幂等测试；实机需补故障注入。
- 新版本旧 QR/新 QR 兼容：保留 v1 fixtures 和严格解码测试；旧 Remote 遇到 v2 需升级。

## 测试计划与当前证据

2026-09-16～17 开发验证：Host access 单测 11 项、协议生成器测试 22 项、信令 Python 单测 5 项、Remote 配对/信令/页面测试 22 项通过。新增 `invitation_tls_retries_share_authorization_without_vps` 在真实本机 TLS 上验证 VPS 不可达、并发重复提交、一次确认、阻止旧接口降级、签名结果与 bearer 有效性。页面测试覆盖 360×640 和 640×360 下确认码、等待状态和失败提示。

- `npm run build`、`npm run build:bin -- --debug -- --locked --jobs 4` 成功，桌面运行隔离工作树 debug 产物，没有替换安装目录。
- Remote `flutter analyze --no-pub` 无问题。Android 构建从 `android/` 运行 `./gradlew.bat :app:assembleDebug --no-daemon --max-workers=2 '-Pkotlin.incremental=false' '-Pkotlin.compiler.execution.strategy=in-process'` 成功。首次构建因 C 盘工作树与 D 盘 pub cache 的 Kotlin 增量缓存跨根路径失败；关闭本次构建的增量编译后通过。APK 与已安装应用签名一致，使用 `adb install -r` 覆盖安装，保留数据。
- 收尾增加缓存结果的撤销检查回归，`cargo test --manifest-path crates/Cargo.toml -p codepet-host --test remote_lan_listener invitation_tls_retries_share_authorization_without_vps --locked --jobs 4` 通过。邀请码准备错误不降级的最后修改通过 `cargo check --manifest-path src-tauri/Cargo.toml --locked --jobs 4`，环境 `TAURI_CONFIG` 使用仓库 `tauri.bin.conf.json` 内容；直接使用默认配置会因隔离工作树缺少打包用 `resources/provider-sdk` 而失败。最后两仓库 `git diff --check`、协议生成新鲜度及 Remote 两套 `cp-sdk-gen --check` 均通过。现场 debug 二进制的真机验证早于这项错误处理收尾修改，未再次替换运行进程。
- VPS 已部署本次邮箱服务；发布前在 VPS 暂存目录跑 5 项单测，发布后 health 成功，codepet-signal/nginx/coturn/xray 均 active。代码、配置和 SQLite 快照备份位于 `/opt/codepet-signal/backups/invitation-20260916-154106`；部署仅替换 Python 服务代码，没有修改既有代理/TURN 服务。
- 2026-09-17 00:19（北京时间）Windows ↔ Android 真机成功：仅将 v2 邀请的 `httpsBaseUrl` 改为不可达的 `https://192.0.2.1:47622`，其余 QR 身份、pin、公钥和 secret 保持原值。手机分段输入完整 JSON 并比对字节一致后提交；双方显示相同六位码，Host 确认一次后显示“设备已连接”，手机加载 Windows 的在线 Claude/Codex。VPS journal 同一邀请的 `role=invite` POST 返回 200；Host cloud peers 从 0 变为 1，凭据总数从 1 变为 2，原授权保留。
- 该真机结果证明无法使用邀请 LAN 地址时，首次授权可经公网邮箱完成，也验证 Dart 与 Rust 加密协议互通。之后日志出现 `Cloud pairing upgrade completed`，对应 LAN RTC 成功分支；不能把后续业务连接记作 TURN 中继成功。本次没有采集其 selected candidate pair。

现场另发现旧配置目录问题：当前 `tauri_bridge.rs` 从 `connections/` 加载 Host 身份和凭据，旧 `remote-access/rtc-cloud.json` 对应另一个 Host ID。不能直接复制旧 token 冒充当前身份。本次为当前 Host 独立登记并放置 `connections/rtc-cloud.json`，保留旧身份；配置缺失时 v1 QR 是兼容行为，不能据此声称 v2 已启用。

## 知识沉淀

长期边界记录到 `knowledge/60-rules/pairing-route-convergence.md`；旧公网首次配对排查文档继续作为历史故障证据。

## 未知项

实测手机 SIM 状态为 `ABSENT,ABSENT`，使用 Wi-Fi；尚未完成真实 4G/5G 网络、切网、VPS 重启故障注入或本次 TURN candidate 验收。Mac 仍需独立公网身份配置和本轮构建验证。当前成果位于隔离工作树，未提交、合并或推送。
