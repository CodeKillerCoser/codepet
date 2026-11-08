# VPS 多用户容量排查：TURN 身份配额与未及时释放的分配

## 现象

2026-11-08 用户担心此前连接容量扩容后，4 或 40 个用户、每人 1–2 部手机及 1–2 台电脑会再次耗尽 VPS 资源。本轮只读检查 `172.96.254.12`，另用本地临时数据库验证信令准入边界；未改 VPS 配置、未重启服务、未在生产申请测试 TURN 分配。

## 复现路径

1. SSH 检查 coturn 的白名单配置项、systemd 资源、relay 端口占用，不输出密钥或 token。
2. 按时间解析 `/var/log/coturn/turnserver.log-20261107` 和当前日志；以 session ID 跟踪 `new`、`refreshed lifetime=0`、`closed (2nd stage)`、全局 allocation counter。
3. 用 `journalctl -u codepet-signal --since '24 hours ago'` 汇总 HTTP 状态与接口，不计入被日志策略省略的成功空轮询。
4. 本地 aiohttp TestServer 使用生产 `create_app`、临时 SQLite 和虚构身份，直接验证信箱与登记上限；不模拟 WebRTC 或公网吞吐。

## 证据

### 当前配置及资源

检查时间约北京时间 2026-11-08 03:06–03:11。coturn 4.17.2：

| 项目 | 当前值 | 范围 |
| --- | --- | --- |
| user-quota | 8 | 每个 TURN 身份的分配数 |
| total-quota | 20 | 当前 realm 总分配数 |
| max-bps | 500,000 B/s | 单分配上限，约 4 Mbit/s |
| bps-capacity | 10,000,000 B/s | 总预留预算，约 80 Mbit/s；不代表购买的真实带宽 |
| relay 端口 | 49160–49259 | 100 个端口 |
| 实时 relay 端口占用 | 0 | 单次 ss 快照，日志最后 counter 也为 0 |

codepet-signal、coturn、nginx、xray 均 active；前两者 NRestarts=0。信令约 27 MB、coturn 约 30 MB，整机可用内存约 619 MB，无 swap 使用，磁盘约 19%。当前证据不支持 CPU/内存耗尽。

此前并非按设备数从 4 改到 16：旧单分配 2,000,000 B/s、总预算 10,000,000 B/s，实际只够约 5 条；后来改为单分配 500,000 B/s、总数 20，详见 [前次排查](webrtc-turn-reservation-capacity.md)。

### 扩容后仍有明确拒绝

北京时间 2026-11-08 02:15:26–02:17:04（coturn 原始时区为 -0500，需加 13 小时），出现 **10 次 ALLOCATE 486**，涉及 6 个失败 session；每次还伴随一行 `message processed`，不能将 20 行重复记录视为 20 次独立分配。

错误为 `Allocation Quota Reached`，与前次 `Allocation Bandwidth Quota Reached` 不同。每次拒绝附近全局 counter 为 16；按 session 重建，该身份已占 8 条。因此这次命中的是身份配额，不是全局 20 或带宽预算。

首次拒绝时，8 条中的 4 条较早分配完全没有 peer 流量（rp/rb/sp/sb 均 0），未记录主动 lifetime=0，分别从 02:12:16、02:13:00 保留至 02:22:16、02:23:00，由 allocation timeout 回收，持续约 600 秒。其余 4 条新申请稍后有 lifetime=0 释放记录。这证明失败尝试会短时滞留占额；不能据此确定是客户端关闭遗漏、网络变化导致释放包丢失还是原生 SDK 行为。

解析窗口（约 UTC 2026-11-06 19:10 起）记录 281 条新建分配，均观察到结束；239 条先主动 lifetime=0、27 条靠到期回收、15 条由 TCP 关闭/错误回收。全局峰值 17，最后为 0。长连接期间可能正常续租，不能把持续时间长或业务空闲本身判为泄漏。

### 信令也有共享限制

线上 `server.py` 与仓库 SHA-256 均为 `5a912d45786d1d1aec4e06394c53899182a4efe246dad546b2590f18695ba9d9`；`registration.py` 均为 `a5cc7cebd61dbf31d3baaaec41ace6d9c4283805d6db5c133dfe029327845993`。

- 最近 24 小时记录 `/v1/ice` 3 次 429；源码按 bearer 限制 12 次/分钟。IPv4 与 relay 两种路径各自获取 ICE 配置，快速重连会加倍消耗这一额度。
- 待协商信箱全局 128，单 host/client 对 12，TTL 60 秒；回答完成后仍可能保留到取消或到期。该容量是握手尝试数，不是在线设备数。
- 登记按 `request.remote` 限 30 次/分钟；当前 nginx 回源为 loopback，应用未使用可信代理来源，因此所有用户共享这一后端来源额度。1024 个登记设备的存储上限不等于能同时登记或连接 1024 台设备。
- nginx 对来源公网 IP 为 10 r/s、burst 30。当前日志未见该层限流证据，但多个设备处于同一公司 NAT/VPN 出口时会共享额度。Host 常态约每 2 秒进行三类信令请求，不能只按活跃握手估算 HTTP 请求量。
- 当前没有应用账号维度。TURN REST 用户名为 `expiry:host:client-or-host`；coturn 去掉时间戳后按身份计数，不是“每个真实用户 8 条”。

### 客户端清理边界的代码证据

Remote `lib/gateway/webrtc/webrtc_gateway_transport.dart::_close` 对 channel.close、peer.close 和 peer.dispose 的 2 秒超时/异常均使用空 catch，最后仍正常返回。`engine_gateway_transport.dart` 依赖 transport.close 抛错才能报告 `candidate_cleanup_failed`。因此原生关闭失败可能被当作成功清理，上层无法隔离未释放的候选许可。这是可复核的代码缺口，尚不能证明上述 4 条分配就是由该路径造成；也可能是切网后 UDP 释放未到服务器。

已有连接模块确实实现了每进程候选预算、退避和清理失败隔离；需验证原生适配器正确传递结果，而不是再叠加一套连接调度。候选预算计数的是通道候选，不等于 TURN allocation 数，更不是跨 40 个用户的 VPS 总预算。

## 初步影响范围

两端 relay 策略都强制 TURN。按每条连接两端各保留至少一条分配估算：

| 用户规模 | 每人 1–4 条同时手机↔电脑连接 | 全部走 relay 时至少分配数 |
| --- | --- | --- |
| 4 用户 | 4–16 | 8–32 |
| 40 用户 | 40–160 | 80–320 |

此为下界，不包括 UDP/TCP/TLS、多接口候选、失败重试及网络切换重叠。LAN/IPv4/IPv6 直连稳定后不需要保留 TURN 分配，但并行探测阶段仍会暂时申请。仅提高 total-quota 而保持另外两项带宽参数不变，仍只能预算约 20 条。

本地准入验证结果（`/tmp/codepet-signaling-capacity-probe.py`，使用 `/private/tmp/codepet-registration-venv/bin/python`，aiohttp 3.14.3 / cryptography 48.0.1）：

| 场景 | 每连接 3 个并行信令 attempt | 接受 | 503 |
| --- | --- | --- | --- |
| 4 用户 × 2 手机 × 2 电脑 | 48 | 48 | 0 |
| 40 用户 × 1 手机 × 1 电脑 | 120 | 120 | 0 |
| 40 用户 × 2 手机 × 2 电脑 | 480 | 128 | 352 |

删除 pending 后可再次准入；同一代理回源下 40 次有效设备登记（各给不同 X-Forwarded-For）为 30 成功、10 次 429。该验证说明固定阈值的行为，不是 VPS 吞吐压测，不代表所有人实际会同时走 relay。

## 当前判断与已排除方向

已确认：单身份 8 条被短时滞留分配占满、共享信令限额会限制多用户并发，当前配置不能承诺 40 用户的全中继及重连峰值。这是当前配置与实现的准入边界，不是 VPS 的实测用户容量；不能据此得出单机只能服务约 100 人，或需要按每几十个用户增购 VPS 的结论。

已排除当前持续耗尽或永久不释放：此刻占用已归零，服务资源正常；失败记录不是“信令没有在线”。尚未确定 4 条分配为何未收到主动释放，不能仅凭服务端日志归因 VPN 或某段 close 实现。

## 容量模型复核：配置限额不等于单机能力

用户提出“单机应能承载大量连接”后，再次核对 coturn 4.17.2 源码：`ns_turn_server.c` 在普通客户端未指定带宽请求时，以 `max-bps` 作为分配的带宽值；`netengine.c::allocate_bps` 从 `bps_capacity_allocated` 累计预留，在最终释放时扣回。这个准入计数不取决于当时有没有业务包。当前 10,000,000 / 500,000 = 20，即使空闲分配也能耗尽预算。

仓库 `server/deploy_turn_capacity.py` 还显式断言 `total-quota * max-bps <= bps-capacity`。这是上一次避免配额与预留预算冲突的保守修补策略，不是 TCP/WebRTC 的容量定律，也不是依据真实业务流量制定的多用户方案。对于大量空闲连接、间歇文本事件和偶发文件传输，不能把所有连接都当成持续使用峰值带宽。

整体保留 LAN / IPv6 / IPv4 并行直连探测、TURN 兜底的架构。Remote 当前按 `DeviceSession` 创建 `ProtocolGatewayClient` / `createDeviceConnection`，同一设备会话中的业务请求复用 transport，并非每开一段对话就新建一组中继。扩容方向应落在以下边界，而非重写业务或先增加服务器：

| 模块 | 修订方向与原因 | 验证路径 |
| --- | --- | --- |
| 信令服务及两端适配器 | 评估每设备复用 WSS 事件推送，或长轮询替代固定 2 秒短轮询；只在变更时同步配对列表；信箱按目标索引并独立清理过期项，避免每请求扫描全局状态 | 对比空闲请求量、CPU、内存、握手 p95；覆盖重连补偿、消息去重、旧客户端兼容、同 NAT 多设备 |
| TURN 部署与资源控制 | 分开设置“允许保留多少分配”和“实际允许传多少字节”；不再用每条峰值预留直接推导在线用户数。评估取消或提高静态预留门槛，同时保留经过压测的数量上限、单身份公平限额和单会话限速，并以实际出口吞吐进行总限速/监控；同步规划 relay 端口与 FD | 多空闲分配 + 少量突发大流量；确认空闲不会因峰值预留被拒绝、单用户不能挤占全部资源、饱和时握手和控制消息可用 |
| 客户端连接生命周期 | 复用已有连接引擎；并行探测仍保留，明确取消失败分支、报告原生关闭失败、控制重试重叠；不以零业务流量回收健康连接 | 切网、VPN 开关、进程强杀、关闭超时及释放包丢失；观察存活连接、悬挂分配数和恢复时延 |
| 可观测性与压测 | 分别记录在线设备、活动设备对、待握手、TURN 分配、实际出口流量和资源水位，不把它们统称“连接数” | 隔离环境逐级验证 1,000 / 10,000 空闲信令连接，再分别测试稳定中继吞吐、并发握手和重连风暴；这些是待验证档位，不是当前 VPS 的性能承诺 |

`bps-capacity=0` 在 coturn 的带宽预留函数中表示不限制总预留；它本身不会提供实际出口的总带宽控制，因此不能仅把该值清零就视为完成容量修复。取消静态预留门槛时，应同时验证真实吞吐控制、资源准入与过载行为。

估算时使用实际同时连接的设备对，而不是已登记用户数或每人手机×电脑的全部组合：稳定中继分配约为每对所需分配数 × 实际中继连接数，再加并行探测与异常滞留；网络容量则看这些连接的实际传输量与突发峰值。成千上万条低流量连接与同数量持续大流量中继，是不同工作负载。本轮没有新增线上压测、配置调整或重启。

## 下一步排查与实施建议

### 实施范围约束：VPS 尽量少开发

用户明确不希望增加大量 VPS 开发。保留现有 Python HTTP 信令、coturn、nginx；不新增自研中继、分布式组件或独立连接调度服务。上述 WSS 是后续性能选项，不是本次修复的前置条件。

设备名称、操作系统、系统版本等展示信息属于两端业务协议，不纳入 VPS 登记表。已核对 Remote `gateway_client.dart` 在通道建立后的 `protocol.handshake.device` 发送这些字段，Host `remote/channels/session.rs` 校验握手后通过 `update_credential_descriptor` 持久化到 PC 本地，桌面展示读取本地 descriptor。当前 `protocol.ping` 维护存活并返回 Provider 摘要，没有重复发送手机名称/系统；后续若补充设备信息变更同步，应走现有业务通道上的心跳摘要/增量更新。用户可见表格使用“PC / 手机”等设备类型；持久化 `host / remote` 表示协议角色，不能普遍等同于硬件类型。

- VPS 配置：调整 coturn 数量、端口、带宽预留策略及 nginx 限流；总出口吞吐控制使用系统/现成组件能力，不自写转发限速器。配置变化仍需验证过载行为，不等于无限放开。
- 现有信令的小范围修补：修正可信反向代理后的登记来源识别；将固定配额参数化；以已认证设备身份限流，保留合理的来源 IP 防滥用保护；优化过期信箱回收并提供明确的限流重试时间。匿名登记仍需来源限流，不能把任意自报设备 ID 当作可信身份。
- APP 侧承担连接管理：复用有效 ICE 凭据、按变更同步授权状态、修复候选关闭异常、重连合并及退避。减少重复请求时须保留首次同步、撤销传播和失败重试，不能靠拉长收信轮询牺牲连接延迟。
- 先压测这组最小改动。如果固定收信轮询确实成为主要瓶颈，再局部增加兼容的长轮询或评估 WSS，不预先把信令整体重写作为必需工作。

1. 客户端：修正原生 close/dispose 失败被吞掉的边界，沿 race/attempt、peer 生命周期记录释放开始、完成和超时；复现 VPN/切网/强杀后对齐 VPS lifetime=0 或超时。保持并行通道选择，同时限制同设备重连重叠并加入退避抖动。验证健康空闲连接不被误停。
2. 信令：登记限流识别可信代理来源，认证后按设备/账号设置公平准入；对 429 携带可重试时间并复用有效 ICE 配置。验证同一公网出口的多用户不会互相阻塞。
3. TURN：统一规划单身份、总分配、带宽预算和 relay 端口，依据实际带宽与并发探测峰值压测定值；评估较短异常分配租期前，先验证原生 SDK 的续租与断网恢复。不要只删除配额。
4. 隔离测试环境：覆盖 4/40 用户、每人 1/2 部手机和电脑、全 relay/混合直连、重复切网、断开及重启。验收分配峰值、释放时延、重连成功率、429/486/503、网络吞吐及资源用量。
5. 运维：增加按身份的活动分配数/年龄/流量、拒绝原因、清理超时指标；当前 no-cli、无已启用的 Prometheus 指标，日志分析不应成为唯一容量预警方式。

## 未知项与来源

VPS 套餐真实上下行带宽、共享带宽政策、用户同时在线比例、实际 relay 比例和单连接峰值均未实测；无法据此给出有保证的最大用户数。当前终端是否已运行最新 Release 也未核验。

协议语义依据 [coturn 4.17.2 配置](https://github.com/coturn/coturn/blob/4.17.2/examples/etc/turnserver.conf)、[身份配额实现](https://github.com/coturn/coturn/blob/4.17.2/src/apps/relay/userdb.c)、[分配与带宽准入](https://github.com/coturn/coturn/blob/4.17.2/src/server/ns_turn_server.c)、[带宽预留计数实现](https://github.com/coturn/coturn/blob/4.17.2/src/apps/relay/netengine.c)。统计只含服务元数据，未输出认证密钥；未读取业务消息或用户文件内容。

## 2026-11-08 实施与验收

用户授权迁入仓库 `server/` 并调整 VPS。源码现位于 `server/`；部署仍使用 `/opt/codepet-signal/`，数据库和协议兼容旧客户端。配置及运行说明见 [server/README.md](../../server/README.md)。

- 信令：固定额度配置化；只信任 loopback nginx 覆盖的 X-Real-IP；身份请求分别限流，返回 Retry-After；5 秒后台回收临时信箱/限流窗口；全局 2,048 待握手、1,024 邀请共享 32 MiB 字符串预算；同一授权列表重复同步不提交数据库写入。默认配对并发 12、握手 TTL 60 秒不变。
- 部署：nginx 来源门槛调整为 200 r/s、burst 400；登记上限 10,000。TURN 每身份 32 / 总计 1,024、端口 49160–51199；单分配仍 500,000 B/s，取消静态总预留，改用系统 nftables 对 coturn 的实际非 loopback 出口流量设置 10,000,000 B/s、2,000,000 B 突发保护。独立表和 systemd oneshot 不修改 xray，coturn 启动依赖规则加载。
- 备份：`/opt/codepet-signal/backups/management-20261107T195219Z`，包含代码、配置、数据库快照及原文件 manifest。部署成功时间约北京时间 03:52。更新前 relay socket 为 0；没有主动断开正在传输的中继连接。

本地和 VPS Python 测试均为 **29 项通过**。包括 480 个同时保留的握手、40 个经同一代理回源但不同来源地址的登记、伪造来源拒绝、共享内存限额、清理恢复以及配对授权隔离。VPS 上的独立网络命名空间探针使用真实 nftables，100 个 coturn UID 数据包有 99 个命中缩小后的测试限额，其他 UID 不受影响；测试结束随 namespace 自动销毁。

线上有限探针：**48 个真实 TURN 分配同时存在**（每身份 16），全部收到 lifetime=0 释放确认。随后运行 Host 现有 `rtc_public_relay_probe`：填写地址自动登记、重复配置、配对授权同步、真实 relay candidate 双端验证、Gateway 握手、约 72 KB RPC 往返和撤销关闭均通过，测试用时 14.46 秒。测试使用独立安装身份，测试后只清理该身份的登记/授权记录。

执行命令：`make -C server test`；VPS `probe_turn_budget.py`、`probe_turn_capacity.py --allocations 48 --per-identity 16`；本地设置 `CODEPET_SIGNAL_REGISTRATION_URL` 和唯一 `CODEPET_RTC_PROBE_HOST_ID` 后，运行 `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test remote_lan_listener rtc_public_relay_probe --locked -- --ignored --nocapture`。另通过 nginx/nft 配置校验、git diff --check；线上 server.py / registration.py / policy.py 与本地 SHA-256 一致。新增 server-quality 工作流尚未提交运行。

边界：此次没有修改 APP 原生候选关闭、心跳协议或轮询节奏，也没有进行万连接/饱和吞吐压测。1,024 是当前配置的分配门槛，不是硬件能力或用户数结论。nftables 是超额丢包保护，未验证饱和下的业务延迟/账号公平带宽；需要更多吞吐时应据实际带宽与负载调整，而非继续按静态预留推算人数。

### VPS 配置归档到仓库

用户随后要求 VPS 相关配置也放入 `server/`。新增 `server/config/`，包括公开部署参数、非敏感信令参数、nginx/coturn/nftables 模板、systemd unit、证书续期 hook 和日志轮转。首次安装和增量维护改为读取同一来源，旧自动登记入口也转入统一维护包。配置源及线上路径映射见 [配置说明](../../server/README.md)。

本地与 VPS staging 的 **34 项测试通过**。VPS 只读校验 `nginx -t`、`nft --check`、`systemd-analyze verify`、`logrotate --debug` 通过；当前 TURN/信令 nginx 规则及四个 unit 与仓库生成结果一致。systemd 校验同时提示既有 xray 使用 nobody，此项属于独立代理服务，未修改。临时校验目录与上传包已清理，没有应用新配置或重启线上服务。

新增回归覆盖 nginx 原生变量保留、模板秘密注入、初始/增量策略一致、现有服务器身份与数据库路径保留、无效端口及限额拒绝。密钥、设备 token、数据库及含密钥渲染结果均未归档进仓库。常用非敏感管理参数以仓库配置为准；已有安装的地址/密钥/数据库路径仍以服务器原值为准，避免配置整理造成身份替换或数据库迁移。

用户随后明确：保留多个 Python 模块，将 `server/` 当作独立项目，减少目录层级。现将应用/维护模块放在项目根，配置统一平铺到 `config/`，测试统一放在 `tests/`；nginx 的三个片段收敛为一份模板。增加项目 Makefile 和独立忽略规则，同步 CI 与部署测试入口，不合并业务模块、不修改线上运行位置。将项目完整复制到仓库外的临时目录后，使用 `make test VENV_PYTHON=<已有虚拟环境解释器>` 运行 **34 项测试通过**；同时通过 `git diff --check`。本次目录整理未部署或重启 VPS。

### 提交与客户端资源边界补齐

服务端已以 `c07e048` 推送 main，[Server quality #42411103073](https://github.com/CodeKillerCoser/codepet/actions/runs/42411103073) 通过。再次本地运行 34 项服务端测试通过；线上五个相关 unit（含独立 xray）均 active，最近半小时信令 journal 无 ERROR/Traceback。此轮只归档代码，不重复重启线上服务。

Remote 原生清理已补齐：关闭等待创建中的 Peer/Channel；逐步记录关闭失败并继续 dispose，最终成功释放可恢复前面关闭错误；无法确认释放时向连接引擎传播错误并隔离名额。共享 ICE 凭据按设备控制器和授权隔离，仅内存保存，最长 60 秒，提前 60 秒到期；IPv4/relay 并行请求合并，HTTP 生命周期不归属某个候选，避免取消一条连带取消其他分支。401/403 失效，失败不缓存。未变更轮询和心跳业务协议。

本地 Remote `flutter analyze`、465 项全量测试、独立引擎测试通过；10,000 轮 / 16 设备模拟压力结束时 liveConnections、permits、waiting 均为 0。100 次模拟原生创建/关闭以及失败隔离回归通过。这些结果不替代 Android 原生、真实切网或实际 TURN 吞吐测试；Android 专项脚本位于 Remote `integration_test/rtc_cleanup_test.dart`，手机验收关注取消释放、VPN/Wi-Fi 切换后连接恢复与 429/486/503，不把配置上限当成并发用户承诺。

随后在本机 ARM64 Android 模拟器运行原生专项：40 个 Peer 并行创建/取消/关闭收到 40 个 cleanup.completed，没有 cleanup.failed，集成测试通过。使用 Debug 测试包；交付另由 GitHub 正式签名构建 Release。模拟器原生测试不替代用户手机 VPN/蜂窝网验收。

Remote 修复已以 `ee7aaea` 推送 main，[Quality #42411878400](https://github.com/CodeKillerCoser/codepet-remote/actions/runs/42411878400)（分析、全量测试、引擎压测及 Debug 编译）与 [Android Release #42411879013](https://github.com/CodeKillerCoser/codepet-remote/actions/runs/42411879013) 均通过。下载正式产物后，apksigner 验证通过，签名与上一版 `18f03e0` 相同；aapt2 确认 `com.codepet.remote`、`0.1.1 (2)` 且非 debuggable，ARM64 AOT 包含完整修复提交号。APK SHA-256 为 `f1931cb30bc8818e099f3f115764f2d98ca926037e6b011dae37a2eac00dd587`。模拟器测试结束后已覆盖回同提交的正常 Debug 应用，未删除应用数据。
