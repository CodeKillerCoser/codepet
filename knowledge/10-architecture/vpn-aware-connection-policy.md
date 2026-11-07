# VPN 与代理环境下的连接策略

状态：2026-11-07 调研建议，未实施。本轮仅查阅官方规范、平台 API、当前源码与已有实机证据；未修改网络设置、VPS 或连接逻辑。按 living-dev-doc-writer 技能记录证据、建议与待验证项。

后续范围收敛：用户已确认企业 VPN 可通过现有 LAN 通道连接，当前只补代理环境的地址采集/交换诊断及日志降噪。下文的完整策略属于调研备选，不是当前实施要求；以 [代理诊断与重复日志排查](../40-runbooks/webrtc-proxy-diagnostics-and-log-noise.md) 为准。

## 背景与证据

已有 VPN 开关对照中，同一手机构建到同一 Mac：VPN 开启时无可用 IPv6 候选，最终使用 TURN；关闭后采集公网 IPv6，约 2.44 秒业务上线，持续 host/host IPv6 直连。只能据此确认 VPN 环境影响候选与路径，不能认定手机上报了“错误地址”，也不能确定具体产品的地址、路由、过滤配置。40 秒等待全量 ICE 是另一项已定位问题，见 [重连诊断](../40-runbooks/webrtc-reconnect-ice-gathering-delay.md)。

源码核查：Remote `routed_rtc_transport.dart` 已并行竞速 WSS、RTC IPv6、IPv4、relay，但 transport.connect 后就取消其他候选，早于 Gateway 握手；`webrtc_gateway_transport.dart` 仍等待完整 gathering 才发 SDP。`path_policy.dart` 与 Host `path_policy.rs` 区分地址族/ICE 类型，没有统一的 VPN、代理和 DNS 策略。WSS resolver 依赖历史 endpoint 与 mDNS，没有企业 VPN 地址集合；Host WSS 默认监听 IPv4，纯 IPv6 endpoint 不能直接假设已支持。

当前 `flutter_webrtc 1.6.2+hotfix.1` 的 networkIgnoreMask 在 PeerConnectionFactory 初始化时设置。全局忽略 VPN 既不能等同于绕过系统路由，也不适合不同设备同时采用不同网络策略。

## 目标与非目标

目标是在系统允许的网络内，兼容公网、企业 VPN、Tailscale 类组网、系统 HTTP 代理、IPv4/IPv6 与中继；先连接可用路径，保留后续改善和网络变化恢复能力。保持身份校验、单一业务发送通道与日志可解释性。

不自行提供 VPN，不停用用户已有 VPN，不突破企业强制隧道，不因连接失败关闭证书校验；本阶段不承诺任意平台都能绕过 VPN。

## 推荐模型：网络策略与传输方式分开

VPN 是底层网络/路由，不是与 LAN、WebRTC 平行且互斥的第五种传输。HTTP/SOCKS 代理则是应用协议代理；关闭 HttpClient 代理并不意味着绕过 VPN，HTTP 能访问也不证明 UDP 能通。RFC 8828 的默认原则是遵循正常系统路由，同时允许配置更多接口参与；对原生客户端借鉴其路由与隐私原则，不假定浏览器策略开关适用于 Flutter。[WebRTC 地址处理](https://www.rfc-editor.org/rfc/rfc8828.html)

建议默认“跟随系统”，提供设备级覆盖：

| 策略 | 预期行为 |
| --- | --- |
| 跟随系统（默认） | 保留 VPN 和分流路由，在允许路径中竞速；不主动选择隧道外网络 |
| 允许额外尝试直连 | 用户为该设备启用且系统允许时，增加物理 Wi-Fi/蜂窝路径，VPN 路径仍保留 |
| 仅经 VPN | 必须能确认并约束到 VPN；VPN 不可用时停止该设备连接，不能悄悄直连。平台不能保证时不提供虚假的保证 |

应用代理设置独立于以上策略：跟随受支持的系统代理 / 指定代理 / 不使用应用代理。所有出口分支，包括 DNS、信令、TURN 和数据连接，都受当前策略约束；不要用“禁用代理”暗示禁用 VPN。

## 企业 VPN 与可信地址发现

将原 LAN/WSS 分支扩展为“可信地址直连”：尝试设备绑定的局域网地址、企业域名、VPN/组网地址。mDNS 只承担本地发现，不能默认穿过三层 VPN；RFC 6762 定义了其链路本地语义。设备可通过已有认证连接更新 endpoint 集合，或手动添加已配对设备地址；依然校验 deviceId、TLS pin，地址不能成为设备身份。[mDNS 规范](https://www.rfc-editor.org/rfc/rfc6762.html)

保留可能经 VPN 可达的私有 IPv4、ULA IPv6、组网地址，不能以“非公网”为由一律丢弃。分流 VPN 的企业 DNS 与路由必须配套，缓存按 network epoch 隔离，避免在家庭和公司重叠的 192.168.x.x 网段误连。VPN 内网没有互联网验证能力也可能可以访问目标，不能仅因缺少 INTERNET/VALIDATED 就排除企业目标路径。

Tailscale 提供 direct、peer relay、DERP 等不同底层通路；CodePet 只使用其可达地址时，不能凭应用 RTT 或 host ICE 候选推断它底层是否又经过 DERP。[Tailscale 连接类型](https://tailscale.com/docs/reference/connection-types)

## 连接与切换

1. 汇总网络、路由、DNS/代理和可信目标地址，生成有界候选；两端独立执行各自策略，不假设对称网络。
2. 并行启动允许的 WSS 与 RTC 路径，每路独立超时、取消和资源预算。按目标可达性决定资格，IPv6 不是必胜标签，也不把全部网络与协议做无界笛卡尔积。
3. 用身份验证及 Gateway 可用性确认胜出，不能只有 TCP/DTLS open。普通连接失败继续其他路径，明确的身份/凭据问题保留诊断，不循环尝试掩盖原因。
4. 信令增加 Trickle ICE，边收集边交换边检查；协议需支持 attemptId、ICE generation、候选去重顺序、结束标记、过期与认证。不要通过固定截断 gathering 丢掉迟到的唯一可用候选。[RFC 8838](https://www.rfc-editor.org/rfc/rfc8838.html)
5. 首条合规可用连接立即承载业务。后续在有限时间/预算内寻找更优直连，经过稳定性验证才切换；避免为了小幅延迟差异反复迁移。这借鉴 Tailscale 的先可用、再改善路径，不复制其 DERP 协议或强制所有连接先中继。
6. VPN 开关、路由/DNS 更新、Wi-Fi/蜂窝切换触发新 network epoch，取消旧尝试并重新验证。健康旧连接可保留至新连接就绪；若新策略禁止旧出口则立即停止使用旧出口。始终只有一个业务发送所有者，迁移不重发结果未知的 create/turn 等非幂等请求，沿用事件游标与 generation fence。

## 平台与代理边界

Android 使用 ConnectivityManager 的 network/capabilities/link properties 回调观察变化；按连接绑定 socket，并在相同 Network 上解析 DNS，避免 bindProcessToNetwork 改变整个 App 和其他已连接设备的出口。枚举到网络不等于有权限使用。[网络状态](https://developer.android.com/develop/connectivity/network-ops/reading-network-state)、[Network DNS/socket API](https://developer.android.com/reference/android/net/Network)、[ConnectivityManager](https://developer.android.com/reference/android/net/ConnectivityManager)

能否选择 VPN 外网络由 VPN 的 allowBypass、始终开启/阻止非 VPN 流量等策略控制，CodePet 不能替第三方 VPN 开启该能力。不要另起 VpnService：Android 每个用户/profile 只有一个活动 VPN 服务，会与现有企业 VPN 冲突。[Android VPN](https://developer.android.com/develop/connectivity/vpn)

macOS/iOS 的 Network.framework 有接口约束 API，但 includeAllNetworks/enforceRoutes 是 VPN 配置能力；不能把选择 Wi-Fi 当作能绕过所有系统 VPN 的承诺。Windows、macOS、Android 应分别返回能否观察/约束网络、代理能力和失败原因。当前 Dart、Rust、原生 libwebrtc 的 socket 所有权不同，需先做小规模平台验证，再决定是否扩展原生插件；不在业务层散落平台判断。[NWParameters](https://developer.apple.com/documentation/network/nwparameters)、[Apple VPN 路由](https://developer.apple.com/documentation/networkextension/routing-your-vpn-network-traffic)

信令 HTTPS、WSS 与 RTC/UDP 必须分别验证代理能力。Dart HttpClient 有 findProxy 接口，不能假设等同于完整接管系统 PAC、代理认证；RFC 8835 将 HTTP 代理支持列为可选能力，不能由“支持 WebRTC”推定已支持。[Dart 代理接口](https://api.dart.dev/dart-io/HttpClient/findProxy.html)、[WebRTC 传输规范](https://www.rfc-editor.org/rfc/rfc8835.html)

中继覆盖 TURN/UDP、TCP、TLS；TLS 可评估部署在 443，但 TURN/TLS 不等同 HTTPS，不能承诺穿过只接受 HTTP 的企业代理。现有 VPS 443 已承担其他服务，部署须单独安排，不直接抢占。若实际目标网络只允许受控 HTTPS/WSS，另评估端到端加密的 WSS 数据中继，不能把当前只交换信令的服务误当成业务中继。[TURN 传输](https://www.rfc-editor.org/rfc/rfc8656.html)

## 涉及模块与验证顺序

- Remote/Host 通道基础设施：增加统一 NetworkContext、目标策略、代理/DNS 适配与候选规划，原因是目前只按协议/地址族区分。
- discovery/可信 endpoint 与 Host listener：支持 VPN 地址及相应监听能力，原因是 mDNS 和一个历史 LAN 地址不足以覆盖企业组网。
- 信令服务及两端 RTC：Trickle ICE、网络 epoch、重协商；版本协商保留旧客户端兼容路径。
- route race / DeviceSession：把可用性确认、候选取消和安全切换连起来；保留业务请求不自动重放。
- UI/诊断：显示“IPv6 直连 · VPN”“中继 · 系统路径”等简短标签；底层信息不足不猜测。“WebRTC 直连”仅表示不经 CodePet TURN，不表示没有 VPN。

先落地统一策略/诊断、VPN endpoint 与分网络 DNS，再补 Trickle ICE/网络变化恢复；绕过 VPN、自动路径升级、HTTPS 专用中继分别验证后交付，避免一次重写整个网络栈。

## 风险与验收矩阵

无 VPN 双栈、VPN 无 IPv6、企业 split tunnel + 内部 DNS、VPN-only 且禁止绕过、Tailscale direct/DERP、UDP 封禁、认证 HTTP 代理、重叠私网、低 MTU/大消息、连接中开关 VPN、并发连接公司与家庭设备都需覆盖。分别验证首条业务握手耗时、实际选中网络/ICE pair、发包出口、上下载与 RTT、切换期间请求不重复、失败诊断明确；不能仅凭 UI 标签判定绕过成功。

沿用唯一纯文本日志格式，记录 network epoch、策略、attempt/offer/connection ID、候选筛除原因、解析/连通/认证阶段、选中路径与迁移原因；凭据和业务正文不记录。VPN 名称、网段等敏感环境信息默认摘要化。需要区分“未采集候选”“采集但连接失败”“系统禁止选网络”“DNS/代理失败”，不把所有情况都写成 IPv6 不可用。

## 未知项与实施门槛

具体手机 VPN 的 allowBypass、IPv6 与 DNS 行为未知；公司 VPN 是否提供目标可达路由、是否支持 UDP/内网 DNS未知。当前插件对每个 PeerConnection 独立网络约束、各平台 TURN 代理认证及切网能力尚未实测；不能承诺加一个 Dart 参数即可完成。本次仅形成方案，没有执行 VPN 切换或新的跨网验收。
