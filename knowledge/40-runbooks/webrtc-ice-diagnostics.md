# WebRTC 直连与分片诊断日志

## 现象与范围

2026-09-10 的手机日志只能看到 `path=relay` 和 `RTC fragment timed out`，缺少候选及检查过程。
本次补充诊断，保留 LAN/WSS、RTC 选路、五秒组包期限和业务接口；不代表直连或超时已修复。
桌面及手机必须使用含本次源码改动的构建，旧版本不会自动出现新日志。

## 证据入口

- 手机：正常诊断导出的 `codepet.log`，logger 为 `gateway.rtc.diagnostic`，message 内为
  `codepet.rtc.v1` JSON。记录连接、ICE、信令、RPC 大小、组包及背压。
- Host：`code-pet.log` 的 `rtc` target，内容为 `codepet.rtc.v1` JSON。桌面初始化注册
  `set_rtc_diagnostic_sink`；有界队列在后台写入既有文件日志，避免 RTC 线程同步写磁盘。
  独立 Host 未注册 sink 时输出 stderr，不能把独立进程输出当作安装版文件日志。
- VPS 信令：`journalctl -u codepet-signal` 中 `codepet.signal.v1` JSON。
- VPS TURN：`/var/log/coturn/turnserver.log`，含 endpoint、session、BINDING、ALLOCATE、
  CREATE_PERMISSION、CHANNEL_BIND 及错误；原始地址和 TURN 用户标识只在服务器本地保留。

## 关联一次连接

两端使用完整 offer SDP 的 SHA-256 前 24 个十六进制字符作为 `offerId`，不保存 SDP。
手机收集阶段用本地 `connectionId`；生成 offer 后同一 connectionId 带上 offerId。
手机 `signal.attempt` 和 Host `signal.offer.accepted` 把 offerId 映射到信令 `attempt`。
VPS 通过 attempt 关联 offer.accepted、offer.delivered、answer.accepted、answer.delivered。
TURN 不理解 attempt，需结合时间、session 和本地 endpoint 日志辅助关联，不能假装三端
所有底层包都有同一个关联 ID。

手机时间为 UTC；Host 有 timestampUnixMs；VPS journal 和 coturn 当前显示 EDT（UTC-4）。
排查前统一换算，不能直接按显示小时比较。

## 可见字段与判断路径

1. `ice.configuration` / `connect.start`：策略、ICE 服务数量；手机记录候选错误回调不可用的能力标识。
2. `ice.localCandidate` 与 `ice.description`：候选类型、协议、优先级、端口、地址类别及脱敏 ID。
   description 记录收发 SDP 字节数和候选数量，最多 128 条，超出会显式标记 truncated。
   手机还保留候选提供的 network-id、network-cost；这些不是操作系统网卡名称。
3. `ice.gathering`、`ice.state`、`peer.state`、`signaling.state`：观察收集、检查、建连状态及耗时。
4. `ice.stats`：候选对状态、提名、候选关联 ID、transport 的 selectedCandidatePairId，以及
   原生库实际提供的请求/响应计数、字节数和 RTT。先检查候选是否收集，再比较两端是否收到，
   最后检查候选对状态和选中路径。不能只凭 relay 断言直连完全没有被尝试。
5. `rpc.queued` / `rpc.response`：手机记录请求方法、哈希后的请求 ID 和字节数；不记录参数或结果。
6. `message.receive.progress`、`message.receive.complete`、`channel.abort`：消息长度、分片数、
   已收字节、组包年龄、最后一片距超时的时间。手机进度日志最多约每秒一次；Host 接收记录
   首片及每 64 片。`send.backpressure.start/end` 和 send.complete 记录发送等待/总耗时。

例如 partialAgeMs 接近 5000 而 lastFragmentAgeMs 很小，支持“固定总期限打断有进展传输”；
二者都接近 5000 才支持“首片后没有进展”。这仍不能单独确定某一中间路由器是否丢包。

## 当前库的真实限制

- webrtc-ice 0.14 的 candidate pair stats 仅填充 ID、state、nominated，其他计数和 RTT 是默认值。
  Host 日志过滤这些默认指标，并记录 `countersAvailable=false`。零不能作为未发探测包的证据。
  依据本地依赖 `webrtc-ice-0.14.0/src/agent/agent_stats.rs::get_candidate_pairs_stats`。
- flutter_webrtc 当前接口没有暴露 onIceCandidateError；本次没有修改第三方原生插件。
  缺少 srflx 时可以结合收集状态、手机 stats 和 VPS STUN 记录缩小范围，不能保证得到具体 STUN 错误码。
- 候选对为定期快照，可能遗漏两次采样间的短暂状态。Host/手机前一分钟约每两秒采样，稳定后
  约每三十秒采样；手机尚未连通时继续两秒采样。关闭/失败补采快照，单次统计等待最多一秒。
- 候选地址使用 offerId 加盐哈希，保留 private/public/cgnat/benchmarkTun 等类别，不保留原始地址。
  同一 offer 的双方描述可关联；跨 offer 不应按 addressId 关联。日志不是抓包的替代品。

## VPS 部署与回滚

增量脚本 `services/signaling/deploy_diagnostics.py` 接收新版 server.py 路径，备份并更新
信令源码、TURN 日志选项及轮转配置。失败自动恢复三个文件并重启；不覆盖身份数据库、密钥或防火墙。
备份保存在 `/opt/codepet-signal/backups/diagnostics-<UTC时间>/`，禁止放在配置 include 目录；校验完整 `/etc/logrotate.conf`。见 [日志轮转备份故障](logrotate-include-backups.md)。

coturn 4.17.2 必须同时设置 `verbose` 和 `log-binding` 才记录 binding 方法，已核对
[官方源码](https://github.com/coturn/coturn/blob/4.17.2/src/server/ns_turn_server.c)。不启用逐包正文输出。
logrotate 配置 daily、maxsize 10M、rotate 3、copytruncate；10M 是轮转运行时检查的阈值，不是实时硬上限。

2026-09-09 17:49:51 UTC 已完成 VPS 更新；健康检查和本机 STUN 请求成功，日志实际出现
`incoming packet BINDING processed, success`。这只验证部署和日志，不代表手机 4G 路径已验收。

## 验证结果与下一轮测试

- `flutter analyze --no-pub lib/gateway/webrtc test/gateway/rtc_diagnostics_test.dart test/gateway/webrtc_transport_test.dart`：通过。
- RTC/信令/诊断三个 Dart 测试文件：12 项通过，覆盖超时进度、关闭后停止采样、脱敏、原有背压与 RPC。
- `cargo test -p codepet-host remote::channels::webrtc --lib --manifest-path crates/Cargo.toml`：5 项通过。
- `cargo test -p codepet-host --test remote_lan_listener --manifest-path crates/Cargo.toml`：2 项通过，
  2 项需要真机/私有配置的探针维持 ignored。
- `cargo check --manifest-path src-tauri/Cargo.toml --lib`：通过，有既有未使用代码告警。
- `uv run --with aiohttp==3.14.3 python -m unittest discover -s services/signaling -p test_server.py`：3 项通过，
  覆盖完整信令阶段关联、鉴权失败与日志不泄露 token/信封/query。

下一轮用新版桌面和手机分别复现 LAN、4G、TUN 开关后的全新连接及大列表加载，按 offerId
收集两端日志，按 attempt 查信令，按时间查 TURN。客户端安装包本轮未构建、未安装；macOS
编译和真实跨网新日志尚未验证。


## 2026-11-06：并行竞速实机采证

### 证据与补充

此前竞速日志只有路径名称和异常类型，三个 RTC peer 与三个信令对象各有独立 connectionId，缺少统一关联；Host 的 accept 错误被调用方丢弃，尚未进入 ICE 的失败没有可靠记录。本轮补充而非宣称旧安装包已具备以下字段。

- Remote 每次竞速生成 `raceId`；每条 RTC 路径的 transport 与 signaling 共享同一诊断对象，带相同 `connectionId`、`pathPolicy`，生成 offer 后追加 `offerId`。所有记录新增 `timestampUnixMs`。
- `race.start/path.start/selected/path.result/failed/close` 记录候选、预算、耗时、赢家和结果；正常落败标记 `cancelled`、`anotherPathSelected`，不冒充网络失败。路径开始记录 `rtcConnectionId`。
- `race.cloudLoad.*` 区分尚无公网授权、读取失败、超时和加载成功。`race.target` 仅记录目标 deviceId 的哈希。
- `connect.stage` 依次记录 ICE 配置、native peer 创建、候选收集、信令交换、应用 answer、等待 DataChannel 打开与 connected；`connect.failed` 记录失败阶段和该阶段耗时。
- 错误采用受控分类：timeout、socket 与 OS 错误码、TLS、HTTP status、noCandidates、签名/绑定拒绝等；不将原生异常正文、完整 URL、token 或 SDP写入这些新增字段。
- Host 新增 `signal.offer.verified/failed`、`signal.answer.published` 和 `signal.http.failed`。错误记录 attempt、阶段、code、retryable；HTTP 错误记录不含 query 的路径、状态或超时/连接失败类别。早期验证失败没有可信 offerId，使用 attempt 与服务器记录关联。
- Host 候选明确带 IPv4/IPv6 family；日志队列满时累计 `diagnosticDroppedTotal`，后续记录可以看出诊断是否有缺口。

上述模块都影响排查边界：Remote 负责竞速与连接阶段，Host 负责签名/admission/协商与 answer 发布，现有 VPS 日志负责证明请求是否收到及何时转交。信令和 TURN 日志已开启，本轮无需重启或修改 VPS 服务。

### 测试与限制

新增测试真实初始化 LocalLogStore，模拟竞速并导出 ZIP，解压验证同一 raceId、赢家、耗时字段及取消原因完整进入导出的 `codepet.log`；另测异常分类不复制敏感正文。RTC/竞速/信令/诊断 25 项测试通过，Host RTC 模块 8 项通过。手机真实原生日志、不同运营商跨网与系统级网络错误仍需实机复现，不能依据模拟导出测试宣称实机已经验收。

当前 flutter_webrtc 不暴露完整 ICE candidate error 回调，Host 部分 ICE 统计无真实计数；保留上文限制。日志可定位在哪一阶段、哪一路径失败，不承诺直接确定 NAT 型号或哪台中间路由器丢包。若 `diagnosticDroppedTotal` 增长，需缩短复现窗口并收集原生/服务器证据，不把缺少日志当作事件没有发生。

### 用户实测时怎样采集

1. 两端使用包含并行竞速和本轮诊断补充的构建；记录版本、开始时间、手机 Wi-Fi/4G/5G、电脑网络，以及 VPN/TUN 是否开启。旧安装版不会因改了源码而增加这些日志。
2. 每个场景单独操作：同 Wi-Fi 连接；手机关闭 Wi-Fi 后新连接；切网重连；连接后创建会话和请求大列表。记录点击与报错的大致时间，避免把多次重连混成一次。
3. 出错后尽快在手机“设置”导出日志 ZIP，再保留电脑日志。Remote 最多 5 个文件、每个约 2 MiB，另有 7 天过期；容量先达到时会提前覆盖，不能只依据 7 天判断还在。
4. 排查从 `raceId` 找四路结果，再按 `connectionId -> offerId -> attempt` 对齐 Host 与 VPS，查 `journalctl -u codepet-signal --utc --since ... --until ...`，结合 coturn 日志。不要先重装、清空应用数据或连续重试很久后才导出。

验证回归应包括：落败取消不触发业务断开；共享诊断对象不改变 SDP 签名输入；新增日志不含敏感正文；日志写盘失败不影响建连。

## 2026-11-06 统一文本日志（取代 JSON/JSONL 导出）

用户明确要求仅保留一种格式：`年月日 时分秒.毫秒+时区 pid thread 级别 文件:行号 [模块] 内容`。当前源码已将 Remote 落盘和 ZIP 导出统一为 UTF-8 `.log`，不再生成 JSONL、副本或连接摘要。ICE 统计、trace 和关联 ID 以键值文本保留；嵌套信息展开为 `stats[0].state=...`。旧版本 `.log` 中的 JSON 在打开时原子转换为文本，保留修改时间以维持过期策略，缺失来源标为 `-`。上文 JSON 入口描述对应旧版本。

Remote `log_context.dart` 在生产日志时同步读取 PID、系统线程 ID、Dart isolate 和调用栈来源；FFI 封装在诊断适配器内，不能通过 MethodChannel 取 handler 线程充当调用线程。Rust `LogOrigin` 同样在事件发生时记录时间和来源，RTC 后台写盘保留生产线程的元数据。两端均为 UTC `+00:00`，Rust `#[track_caller]` 透传调用位置。第三方库无源码符号或未来启用拆分符号时，位置不可获取则明确缺失。

验证：Remote diagnostics/gateway/settings 144 项测试通过，相关 analyze 通过；Host 来源跨线程/RTC 格式 2 项通过，Tauri app_log 9 项通过。Tauri 检查需沿用本仓库测试设置 `TAURI_CONFIG='{"app":{"macOSPrivateApi":false}}'`，裸命令被已有配置/feature 检查拦截。独立 Dart AOT 可执行探针确认真实线程 ID 与文件行号；这不等同于 Android 新安装包实机验收。本轮尚未构建/替换桌面和手机 Release。

规约见 [文本日志格式与来源上下文](../60-rules/text-log-origin.md)。VPS 自有信令与第三方 coturn 的既有日志配置未在此客户端格式修改中部署更新；排查 486 仍须结合 coturn 证据。
