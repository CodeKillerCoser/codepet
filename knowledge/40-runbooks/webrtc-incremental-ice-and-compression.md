# 增量候选交换与大响应压缩

## 现象与证据

2026-11-07 用户要求修复此前日志中已确认的性能问题，并为未知异常保留少量诊断。原始日志及行号见 [代理诊断与重复日志排查](webrtc-proxy-diagnostics-and-log-noise.md)。当前 Mac 开 VPN 时 relay 的可用候选已出现，但 Remote 等完整 gathering 约 40 秒才发送 offer；中继中 680798/1385488 字节响应组包分别耗时 12.7/24.4 秒，心跳有 4–9 秒等待。关闭 VPN 后 680798 字节响应组包约 460 ms。

源码证据：Remote 在 `setLocalDescription` 后等待 `gathered.future`；双方 CPG1 分片均直接发送 JSON，且保持单个有序消息流，不允许交错分片。大响应占用流时小响应必须等待。VPS 当前 `max-bps=500000`、`bps-capacity=10000000`；coturn 的单位是字节/秒，不能把 55 KB/s 直接归因于 500 Kbit/s 限速，[官方参数说明](https://github.com/coturn/coturn/blob/master/README.turnserver)。本轮未调整 TURN 或 VPN 路由。

## 修复方案与兼容边界

### 提前交换握手，保留迟到候选

- Host 授权同步 `/v1/clients` 广告 `trickleIce=true`；服务 `/v1/ice` 只对已广告该能力的目标返回支持。旧服务或旧 Host 不返回支持时，Remote 保留完整 gathering 的原流程，无需强制重新配对。IPv6 host-only 路仍沿用已有快速完整收集。
- 新 Remote 的 IPv4/relay 路立即发出初始 offer，可包含零个候选；`onIceCandidate` 继续过滤并将迟到候选经 `/v1/ice-updates` 补交给 Host。Host 仍回完整 answer。本次实现的是客户端向 Host 的增量候选，不宣称双向完整 Trickle ICE。
- 每批最多 8 个候选，候选字符串最多 1024 字节，总候选最多 128。批次使用手机设备密钥签名，绑定 host/client/attempt、原始 offerHash、严格递增 sequence 与原 offer 到期时间。Host 验签、验绑定、检查撤销，再按路径策略应用候选。完成标记不会提前截掉尚未出现的候选。
- 服务只存转发的签名信封：鉴权按 host/client/attempt 隔离；同序同内容可幂等重试，不允许覆盖或跳序；每批信封最多 16000 字符，每 attempt 累计最多 128 KiB、最多 128 批，读取每次最多 4 批。沿用 60 秒邮箱过期、撤销和取消清理。
- Host 增量路径从接收 offer 开始保留 55 秒建连预算，避免提前发 SDP 后仍沿用旧的 15 秒 channel-open 等待，从而丢失较晚才出现的可用候选。任务在连接成功、关闭、撤销、到期或 receiver 丢弃后结束；没有增加串行通道尝试。

### 减少大消息传输字节

SDP 使用 `a=x-codepet-compression:gzip` 双方协商。没有双方确认则继续 CPG1；确认后，至少 16 KiB 且压缩节省达到 10% 的消息使用 CPGZ 魔数携带 gzip，原有分片 total/offset 表示压缩后大小。接收端要求已协商、禁止同一消息中途更换编码，关闭帧仍为 CPG1。压缩前大小和解压后大小都受原请求 256 KiB/响应 4 MiB 上限约束；解压采用有界输出，坏 gzip 拒绝。

压缩减少真实文本响应占据通道的时间，但不是独立心跳通道或分片多路复用。低可压缩数据、丢包/高延迟和有序流排队仍可能拖慢心跳，本轮不宣称已消除所有队头阻塞。保留压缩前/后字节数、大消息进度、背压与 RPC 时长，以实机数据判断后续是否需要单独控制通道。

## 未确认异常的诊断与边界修复

- LAN `ObservedSocket` 使用 yawc 的结果接口，记录暴露出的错误类别/OS 错误码、收发 close 帧的真实 code 和 reason 字节数；不打印帧内容。旧适配器丢弃错误并将所有收到的 close 都写成 1000，现已保留真实值。库内部自动关闭的部分原因仍可能只表现为 connectionClosed；1002 的具体来源尚未复现，不更改重试策略。
- Android NSD 失败时限频读取 phase、耗时、超时预算、是否开始/解析中、发现数量、队列和错误码，仍进入统一纯文本日志。原生方法无法及时响应时记录 snapshotUnavailable。源码确认晚到的 onDiscoveryStarted 可在 timeout 后留下扫描：现已在 finished 时停止晚启动的扫描；没有把这项边界修复宣称为此前所有 NSD 超时的根因。
- 信令服务诊断同步改为 UTC 日期时间、真实 PID/native thread、源码行号及键值文本，不再输出 JSON 日志；邮箱信封、凭证和查询参数不进入日志。
- VPS 验收发现 Host 每约 2.5 秒成功同步 `/v1/clients` 仍重复打印 HTTP 完成记录。现只在授权列表或增量能力变化时记录 `clients.changed`；相同同步静默，失败仍记录。测试覆盖重复同步、能力变化、撤销和鉴权失败。

## 涉及模块

- Remote `gateway/webrtc/{signaling,cloud_signaling,webrtc_gateway_transport,path_policy,framing}.dart`：可选能力、签名批次、早发握手、兼容及有界解压；新增测试覆盖晚候选、真实签名绑定、旧路径与压缩边界。
- Host `remote/channels/webrtc/{cloud,mod,path_policy,framing}.rs`：授权能力同步、验签和原生 add_ice_candidate、建连预算、压缩协商与发送。
- `server/server.py`：只承担能力广告与有界候选邮箱，仍不解析 ICE 地址/转发业务流量。
- Host `lan/websocket_diagnostics.rs` 与 Remote `MdnsBridge.kt`/discovery adapter：补足未知故障的关闭边界和原生阶段证据。

## 验证结果

- Remote gateway/discovery/diagnostics 166 项通过；新增 cloud trickle HTTP mock 单独 1 项通过，实际验证候选在 offer 发布后提交、签名、offerHash、sequence、完成标记和最终验签。相关 Dart analyze 无问题。
- Host 通道单测 36 项通过，包含真实 Rust WebRTC peer：空候选 offer 后经生产 worker 添加晚到候选，最终双端 DataChannel open。LAN/WSS/RTC 集成 3 项通过；两个需要外部环境的 probe 默认忽略，其中 Android probe 本轮单独执行。
- Android 原生代码编译通过；Android 模拟器 ↔ Rust Host 原生 RTC 测试通过，验证 Gateway 握手、大消息请求/响应、事件及凭据撤销。测试响应 72646 字节压缩到 847 字节，手机正确还原；该数字是测试样本，不是用户真实会话压缩率或公网速度承诺。使用独立 `.rtcprobe` Debug 包，临时 Host 和模拟器已关闭。
- 信令本地 18 项测试通过；VPS 使用当前部署的 registration.py 和生产 Python 环境，候选隔离、顺序、幂等、撤销和注册等 15 项通过。
- 两仓库 diff whitespace 检查通过；用户安装的手机/桌面 Release 未替换，真实 VPN 开关与公网中继速度仍待新包验收。

验证命令：Host 在 `crates/` 执行 `cargo test -p codepet-host --lib remote::channels -- --test-threads=1` 和 `cargo test -p codepet-host --test remote_lan_listener -- --test-threads=1`；Remote 执行 `flutter test test/gateway test/discovery test/diagnostics`、`flutter test test/gateway/cloud_trickle_test.dart` 和 Android 原生 `integration_test/rtc_gateway_test.dart`；信令执行 `uv run --with aiohttp --with cryptography python -m unittest discover`。

## 服务端部署

北京时间 2026-11-07 02:51，已将兼容旧客户端的新信令服务部署至 `172.96.254.12`，仅重启 `codepet-signal`。公网 HTTPS `/health` 返回 `client-trickle-ice-v1`；信令和 coturn 均为 active。部署前原始服务备份在 `/opt/codepet-signal/backups/trickle-20261106T184602Z/server.py`；最后一轮日志收敛前备份在 `trickle-20261106T185120Z/server.py`。更新脚本在健康检查失败时自动还原。未修改 TURN 限速或 VPN 配置；新性能路径须桌面和手机双方更新后才会启用。

## 回归防线与未知项

不以删除迟到候选、扩大无界缓冲、忽略证书/签名、关闭用户 VPN 换取速度。新增信令必须绑定原始协商及期限；压缩必须协商并限制解压输出。新增快路径保留旧端回退和取消释放测试。

引入历史未确认。用户手机上代理导致 IPv6 不可见的具体原因、LAN 1002 根因和真实中继性能尚未经新 Release 对照；保留 [代理采证步骤](webrtc-proxy-diagnostics-and-log-noise.md)。
