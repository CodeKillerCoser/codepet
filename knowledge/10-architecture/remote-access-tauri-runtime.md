# Remote LAN Access 的 Tauri 生命周期与命令边界

## 背景

Host 已具备稳定设备身份、LAN TLS identity、一次性 pairing、credential store、HTTPS/WSS listener 与 mDNS advertiser，但此前 Code Pet App 不创建或持有这些对象。仅有 Host API 不等于产品运行时已开放远程访问：App 还需要复用 Provider Host 的唯一事实源、选择可发布 LAN IP、把 pairing 可用性同步到 mDNS，并在退出时有界释放网络与监控任务。

本页记录 Phase 2B1 的实际后端接线，以及后续连接页对 pairing 命令的接入边界。事实依据是 `crates/codepet-host/src/remote_access.rs`、`remote_listener.rs`、`remote_network.rs`，以及 `src-tauri/src/runtime_gateway/remote_access.rs`、`tauri_bridge.rs`、`frontend/App.svelte` 和 `frontend/lib/PairDeviceDialog.svelte`。

## 目标

- 让 Tauri 管理单个 `RemoteAccessRuntime`，后台启动 LAN listener 与 mDNS，网络失败不阻断窗口和 App core。
- 复用 Provider Host 的同一个 `DeviceRegistry`、`PluginManager` 和 `ProviderGatewayService`，不创建第二套 Provider 进程、registry、event bus 或设备身份。
- 通过显式命令提供状态、retry、配对、client 列表和 credential 撤销；普通状态、start 和轮询响应不返回 secret、bearer 或原始 QR JSON，用户主动复制时也只把成功或安全失败返回 WebView，原始 JSON 仅在原生侧写入系统剪贴板。
- pairing start/cancel/consume/expire 自动驱动 mDNS `pair=1/0`，不依赖 UI 轮询。
- App 退出时先有界停止 pairing monitor、mDNS、listener，再关闭 Provider Host。

## 非目标

- 不改变 Gateway wire、Remote 客户端解析方式或 pairing credential 语义；连接页 UI 只消费 Tauri 命令。
- 不修改 Gateway wire、Provider Protocol、Pet/activity、Desktop Companion IPC 或其事件投影。
- 不实现 IPv6、多网卡选择 UI、relay、E2EE、RBAC、refresh token 或 credential scope。
- 不把 pairing secret、pairing outcome 或在线 session 持久化。

## 现状理解

`ProviderHostState` 是 App 内 Provider 生命周期入口。配置阶段先打开唯一的 `DeviceRegistry`，再把同一个 `Arc<DeviceRegistry>` 同时交给 `PluginManager::with_device_registry` 与 `RemoteAccessManager::open`。随后只构造一个 `ProviderGatewayService::with_remote_identity`；现有 compat `RuntimeGatewayState` 和新 LAN listener 都引用这个 service，所以 Manager 的单一 update receiver、Gateway replay/event sequence 和 Provider 进程事实不会分叉。

`RemoteAccessRuntime` 由 Tauri `manage`。它持有 `RemoteAccessManager`、同一个 Gateway、运行中的 `RemoteLanServerHandle`、当前 mDNS generation、安全诊断、pairing monitor 和 network monitor。它还只为当前 active pairing 暂存一份 `{ pairingId, exactSerializedJson, qrSvg }`：同一次 `serde_json` 序列化的字节既输入 QR encoder，也供显式复制命令读取，不通过二维码反解。setup 只调用 `start_in_background`；首次 IP/bind 失败不会从 setup 返回错误。listener 启动后，地址消失或 mDNS Announce 失败只撤下 discovery generation 并保留 listener、TLS identity、credential、Gateway 与 Provider；显式 retry 和后台 monitor 都可重新发布。

## 实现路径

### 生命周期与状态

runtime 状态为 `starting / available / unavailable / stopping / stopped`。`available` 表示业务 listener 仍在运行，不等同于 discovery 一定可用；无 publishable 地址或 Announce 失败时，status 保持 `available`，但 `advertisedHost/httpsBaseUrl` 为 `null`、`pairingAvailable=false`，并返回可重试 discovery 诊断。status 其余字段仍只有 device id/display name、总在线 session 数和不含 Host error details 的 `code/message/retryable`。启动中调用需要 listener 的命令返回可重试 `remote_access_starting`；不可用、停止中和已停止都有稳定错误，不 panic。

启动顺序是：读取环境覆盖或执行 route probe；产品 runtime 优先以 `0.0.0.0:47622` bind listener；用 listener 的实际端口和选中 IPv4 构造第一代 client endpoint；最后从同一个 handle-derived source 启动 mDNS。只有明确的 `AddrInUse` 才记录 `remote_lan_stable_port_unavailable` 告警并回退 `0.0.0.0:0`；其他 bind/config 错误继续 fail closed。`codepet-host` 的默认 config 和并行测试仍使用 port 0。退出顺序是：通知并有界等待 pairing/network monitor；注销和关闭 mDNS；取消所有 session 并关闭 listener；然后 `handle_run_event` 才调用 Provider Host shutdown。ExitRequested、Exit 和托盘退出仍汇入同一条退出路径。

### IPv4 选择

v1 只发布 IPv4。若设置 `CODEPET_REMOTE_ADVERTISED_HOST`，值必须是具体 IPv4，且必须属于 `if-addrs` 枚举出的一个 active 本机接口；unspecified、multicast、documentation 地址和同一 IP 同时落在多个 active 接口上的歧义都会 fail-closed。

没有覆盖时，Host 创建 IPv4 UDP socket，对 documentation target 执行 `connect` 以让操作系统选择路由，然后只读取 `local_addr`；代码不发送 datagram。route probe 失败、结果不是可发布 IPv4、结果不属于 active 接口或匹配多个 active 接口时都返回可诊断错误，不从“第一个网卡”或地址排序猜测。loopback 仅用于确定性自动化和显式本机调试；跨设备可达性仍需真机验证。

运行期复用同一个 resolver，每两秒检查一次；只有连续两次得到相同地址状态才进入串行 generation 协调器，避免 DHCP/VPN 切换瞬间反复重发。轮询任务由 shutdown watch 停止，不创建平台专属常驻线程。无地址时先有界注销旧 mDNS，再把 committed endpoint 置空；同一 wildcard listener 和已有 WSS session 不变。地址恢复后自动重建 advertiser。相同地址但 advertiser 缺失时也会继续低频重试，因此一次 Announce 失败不会永久杀死 runtime。

### Pairing 状态与 mDNS

同一时间只允许一个 active pairing。`RemoteAccessManager` 在单一 mutex 内保留 active session，以及最多 64 条、最长约 15 分钟的 `succeeded / expired / cancelled` 内存结果。Host 重启后这些状态消失；secret 和 outcome 都不落盘。cancel 对已经 cancelled 的 pairing 幂等；对 succeeded/expired 返回稳定的 not-active 错误。

Manager 的最小 `watch` 值只有 pairing id、是否可用和单调 deadline。begin、cancel、成功 consume 与 expire 都更新 watch。Tauri monitor 等待 watch 变化或精确 deadline，不做高频轮询；变化后让 generation 协调器以当前 endpoint 和最新 `pair` 替换 advertiser。因此 listener 的真实 POST exchange 成功消费 secret 后，即使 UI 没有查询状态，mDNS 也会回到 `pair=0`，查询同一 pairing id 得到 `succeeded`。

网络地址和 pairing 变化共用一个 lifecycle mutex。每次协调都读取最新 pairing watch，并用完整 `{ endpoint, pair }` 替换 mDNS generation；旧 advertiser 必须完成 unregister/restart，新 generation 必须收到 Announce 才能提交。listener 内部同时保留 committed endpoint 和至多一个 pending endpoint：status/QR 只读 committed；新 Announce 已发出但尚未提交的极短窗口内，`Host` authority 匹配 pending IP 的 pairing exchange 已返回 pending generation 的 WSS URL，因此不会出现“发现新 IP、exchange 返回旧 IP”。Announce 失败会清除 pending 和已过期 committed endpoint，不复活旧 service。

active pairing 的缓存 JSON/QR 也属于 advertised generation。地址提交后，Host 用原 pairing secret 重新序列化同一 pairing id 的 `httpsBaseUrl` 并生成新 QR；`get_remote_pairing_status` 在 active 状态返回当前 `qrSvgDataUrl`，连接弹窗的既有轮询据此替换图像。无地址时 QR/复制入口暂不可用，地址恢复后同一 active pairing 自动得到新 endpoint；这只扩展本地 Tauri command view，不修改 Gateway v1 wire。

### Tauri commands

命令均已加入 `tauri::generate_handler!`：

- `remote_access_status`：读取安全运行状态与诊断。
- `retry_remote_access`：串行重试不可用的 LAN runtime；listener available 但 discovery unavailable 时立即重新解析并发布。
- `list_remote_clients`：按稳定 `remoteClientId` 把同一客户端的 credential 记录投影为一个逻辑设备，返回代表 credential 的 `credentialId` 以及聚合后的 `descriptor/createdAt/lastSeenAt/revokedAt/onlineSessionCount`；同名但 `remoteClientId` 不同的设备不得合并。
- `start_remote_pairing`：只返回 `pairingId/expiresAt/qrSvgDataUrl`。
- `get_remote_pairing_status`：按 pairing id 查询 `active/succeeded/expired/cancelled`；active 时附带当前 advertised generation 的 `qrSvgDataUrl`。
- `copy_remote_pairing_json`：只接受主窗口的用户显式复制；在 Host pairing mutex 内确认匹配 pairing 为 `active` 并同步写系统剪贴板，只返回成功或安全错误，终态、旧 id 和非主窗口均 fail closed。
- `cancel_remote_pairing`：取消 active pairing；重复 cancelled 调用幂等。
- `revoke_remote_credential`：用列表代表 credential id 解析逻辑 client，原子撤销该 client 的全部 active credential；listener 先向整组 credential 广播取消，再用一个共享期限等待并统一返回总断开数或剩余组数错误。

`start_remote_pairing` 直接构造生成 SDK 的 `PairingQrPayload`，对它做一次 `serde_json` 编码，再让同一字符串进入 `qrcode` 并生成 base64 SVG。start 响应、状态响应、日志和 Debug 不包含 `pairingSecret`、bearer 或原始 JSON。原始 JSON 的第二个出口仅是用户点击“复制配对 JSON”后的原生命令：命令拒绝 `main` 之外的 WebView，`RemoteAccessManager::run_while_pairing_active` 在同一 pairing mutex 内完成 active 校验和同步剪贴板写入，使成功 consume、cancel 和 expire 必须在线性顺序上发生在复制之前或之后，不能插入校验与写入之间。clipboard plugin 只供 Rust extension 使用，默认 WebView capability 不授予它的直接命令。WebView 只得到成功或固定错误，JSON 不进入 JavaScript、DOM、ARIA、错误消息或持久化状态。

暂存 payload 在 cancel、consume、expire、runtime fail-closed 和 shutdown 时清除；暂时无 advertised endpoint 时保留内存 JSON 以便恢复，但清除可显示 QR 并拒绝复制。清理按 pairing id 匹配；旧 pairing 的延迟 cancel 不得删除新 pairing 的 payload。pairing monitor 以 manager watch 的最新值收敛缓存，即使 UI 没有继续轮询也会在终态清除。前端在 success/cancelled/expired、retry、close 和新 generation 开始时重置复制反馈；本地倒计时为 0 时立即禁用复制，而 Host 命令仍做最终权威校验。

client online 数只读取 listener 的实际 session registry。同一 `clientId` 的所有 credential session 求和，因此重复配对不会产生“旧记录离线、新记录在线”的重复设备；不同 `clientId` 即使 descriptor 同名也分别展示。credential store 仍保留逐 credential 的授权与审计记录，pairing/WSS wire 不变。设备撤销先按 `clientId` 原子撤销全部 active credential，再对全部 session group 广播取消，最后用共享有界期限等待归零；某个顽固 session 超时不得阻止后续 credential 收到取消。其他 client 不受影响，重复撤销仍返回代表记录原有的 `revokedAt`。

## 涉及模块

- `crates/codepet-host/src/manager.rs`：让 Provider Manager 接受共享 `Arc<DeviceRegistry>`，避免复制 App 身份事实。
- `crates/codepet-host/src/remote_access.rs`：增加 pairing watch、短期 outcome 查询、单 active 约束和按 credential id 幂等撤销。
- `crates/codepet-host/src/remote_network.rs`：执行 IPv4 override/route probe/active-interface fail-closed 选择。
- `crates/codepet-host/src/remote_listener.rs`：持有 committed/pending advertised endpoint、为 pairing exchange 选择同代 gateway URL，并继续暴露 session 计数和有界定向断开。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs`：组合共享 device/manager、唯一带 remote identity 的 Gateway 与 RemoteAccessManager。
- `src-tauri/src/runtime_gateway/remote_access.rs`：Tauri lifecycle、状态、commands、QR 和 pairing monitor。
- `crates/codepet-host/src/remote_mdns.rs`：以同一 listener source 替换 IP/interface/pair 的完整 mDNS generation，并保留 Announce/旧 resend 屏障。
- `frontend/App.svelte`、`frontend/lib/remoteAccess.ts`：只让既有 pairing status 轮询替换当前 active QR，不改变 Pet UI。
- `src-tauri/src/lib.rs`：manage、后台启动、invoke 注册和退出顺序。

## 风险

- 多网卡或 VPN 路由选择错误：route result 必须唯一匹配 active 接口；纯函数负例验证失败、非本机和重复接口，真机仍需覆盖 VPN/热点切换。
- 稳定端口被其他进程占用：runtime 测试占用首选端口并验证明确诊断、ephemeral 回退和实际 endpoint；非 `AddrInUse` 错误不得静默回退。
- pairing secret 泄露：命令序列化测试继续断言 start 响应只有三个字段，解码 SVG 也不携带 JSON 文本；active-only 测试只解析字段并检查非敏感元数据，不 snapshot 或打印 payload；前端源码测试断言弹窗没有明文 JSON 容器，截图使用脱敏预览。
- 终态或旧 generation 仍可复制：Tauri 测试覆盖 cancel、成功 consume、旧 cancel 不影响新 pairing，以及复制与 cancel 竞争时由 Host mutex 线性化；前端纯状态测试覆盖倒计时为 0、success、expired、cancelled 均不可复制，并用 request token 与 pairing id 防止旧请求反馈污染新弹窗。
- consume 后 discovery 仍为 pair=1：真实 HTTPS exchange 测试验证 watch 从 1 到 0 且 outcome 为 succeeded；mDNS 单元测试验证相同 service 的 pair 更新。
- client 聚合或撤销越界：Tauri 定向测试覆盖同 client 多 credential 聚合、在线数求和、同名不同 client 不合并，并验证设备撤销覆盖同 client 全部 bearer、保留其他 client；listener 可控 session 测试让首组超时并断言后续组仍先收到取消。
- shutdown 遗留 daemon/socket/task：runtime 测试验证 stopped 状态，Host listener/mDNS 测试验证有界关闭与重复 shutdown；Exit 测试固定 Remote 在 Provider 前关闭。
- IP/pair 竞态复活旧 service：单一 lifecycle mutex 串行替换，fake publisher 测试断言第一代新 IP 之后只出现新 IP，最终 pair 为 manager 最新值。
- 新 IP 已 Announce 但 exchange 仍返回旧 URL：listener pending endpoint 以请求 `Host` authority 选择 gateway URL；真实 TLS listener 测试在 committed 仍为 A 时从 pending B exchange 并断言返回 B。
- 地址消失或 Announce 失败误杀业务面：runtime 测试固定 listener port、TLS certificate 和 bearer 可用性，验证 discovery 置空后 B 恢复以及失败 generation 可重试。
- 网络启动拖垮 App：startup failure/retry 测试证明 Remote unavailable 时同一个 Host core 与 credential store 仍可访问，setup 源码只后台启动。

## 测试计划

- Host `remote_access` 单元测试：单 active、watch、四种状态、一次性 consume、重启丢失和幂等 credential revoke。
- Host `remote_network` 单元测试：显式 loopback、route-selected 地址、probe failure、非本机、documentation 与歧义。
- Host `remote_lan_listener` 真实 loopback 测试：HTTPS pairing exchange、WSS、在线计数、同 credential 多 socket 撤销、其他 client 隔离与 shutdown。
- Tauri `remote_access` 单元测试：后台等价启动失败不影响 core、stable port 成功与占用回退、retry、QR 响应字段、原生 active-only copy、copy/cancel 线性化、watch 驱动清理、真实 HTTPS consume、同 client 多 credential 的逻辑设备聚合与 client 级 revoke，以及 shutdown。
- Tauri `remote_access` generation 测试：A→B 的 mDNS/status/QR/exchange 一致；A→无地址只撤 discovery 且 listener/TLS/credential 不变→B 恢复；IP/pair 并发最终一致；Announce 失败后同 IP 重试；shutdown 后 resolver 不再运行。
- Tauri `tray_tests`：Remote state 参与退出，并验证八个命令全部进入 invoke handler。
- Frontend `remoteAccess/connections/remoteDevices`：bridge 参数、原生复制成功/失败反馈、active-only 显示、终态禁用和普通 UI 不渲染 payload；`npm run build` 验证 Svelte 编译。
- 验收运行 Host all-targets、Gateway protocol、Tauri 定向测试与 `cargo check --lib --locked`，再执行 frontend 未改、schema 漂移和 `git diff --check` 检查。

## 知识沉淀

本页是 Phase 2B1 的当前事实入口。Gateway wire 与 mDNS 代际契约仍由 `gateway-v1-lan-generation-contract.md` 维护；运行期地址变更的长期约束见 `remote-lan-runtime-address-generation.md`；Provider/companion 事件隔离仍由 `provider-host-device-and-plugin-runtime.md` 和 `codex-provider-channel-isolation.md` 维护。若真机验证暴露防火墙、睡眠唤醒或权限问题，应新增对应 runbook。

## 未知项

- 连接页已经消费 remote client 列表并展示持久 descriptor；手机侧对称消费 Host descriptor、真实跨 LAN pairing 与握手仍需独立验证。
- 手机等真实设备的跨 LAN TLS pin、系统防火墙提示和睡眠唤醒尚未完整验证；macOS 实机网卡切换需继续作为发布 smoke。
- 原生系统剪贴板在不同桌面平台的失败提示已 fail closed，但仍需各平台真机 smoke。
- v1 只支持 IPv4；IPv6 地址选择与双栈 mDNS 发布未设计。
