# 连接引擎：选路、健康观测与传输统计

状态：2026-11-07 已实现核心、Remote 接入与 Host 交接协议，已通过本地自动化验收；待真机与发布验收。沿用 living-dev-doc-writer。设计与实现依据见下文；原始现象和源码证据见 [网络变化恢复调研](network-change-connection-recovery.md)。不代表新客户端已发布或完成手机切网实测。

## 目标与边界

统一提供通道选择建立、持续健康观测、传输统计。首次连接、网络变化、前台恢复、用户重连都只是外层触发源，不进入引擎策略分支。每个目标/授权绑定一个控制器，所有入口调用同一个幂等协调接口。

优先级固定为 LAN（包括企业 VPN、Tailscale 等可达地址）→ WebRTC IPv6 直连 → IPv4 直连 → TURN 中继。四路并行，先恢复服务，再收敛到最高健康优先级。VPN 的底层转发方式不是 App 的通道类别。不是带宽测速器，不做业务 RPC 自动重放，不改用户路由，不承诺零中断或系统挂起时持续工作。

## 改造前需要改变的边界

`RoutedRtcTransport` 在 transport.connect 成功时取消全部候选，尚未验证 Gateway 会话；`GatewayClient.connect` 又把握手、订阅与心跳合为一次操作。必须拆分候选验证和业务激活，不能简单把旧 transport 的 socket 指针换掉。Host 当前握手成功即注册 presence，缺少 prepared/active 区分。`DeviceSession` 的重试定时器须迁入引擎，不能保留两套重试所有者。

## 分层与所有权

```text
设备操作 / 网络观察器 / 前台生命周期
             │ 更新输入、请求协调
             ▼
 ConnectionEngine（纯状态机、策略、预算）
   ├─ CandidateAdapter：LAN / IPv6 / IPv4 / Relay
   ├─ HealthMonitor：适配器健康 + 会话就绪结果
   └─ TrafficLedger：每连接计数、窗口速率
             │ Prepared / Active / Failed
             ▼
应用层 SessionBridge（验证、业务恢复、交接屏障）
             │
Host 通道归属控制（代次确认、拒绝旧代请求）
```

引擎不 import UI、Android、Gateway method/Provider DTO；会话桥通过应用层端口返回 ready/failed 和交接完成状态。平台观察器处理 NetworkCallback 与前后台语义，向引擎更新可用候选环境；不是引擎反向读取系统 API。UI 仅订阅快照，关闭页面不停止探测/统计。应用 shutdown/显式断开才释放目标控制器。Flutter/Dart 实现客户端状态机，Rust Host 实现对端归属，不为形式上的复用强行跨语言共享运行时。

## 公共契约（概念接口）

| 接口 | 语义 |
| --- | --- |
| `reconcile(TargetSpec, Policy)` | 接受“保持可用并择优”的期望；返回操作代次，不等待全部候选耗尽 |
| `watch(targetKey)` | 最新连接阶段、active 通道、健康、统计及有限错误摘要 |
| `disconnect(targetKey)` | 撤销在线期望，取消正在建立/升级/重试；重复调用安全 |
| `dispose()` | 取消并等待有界资源清理，不接受新请求 |

TargetSpec 包含稳定设备 ID、授权引用/版本、候选地址、环境版本；不携带“网络变化”等场景枚举。Policy 包含允许的通道、固定优先级、超时和资源预算。相同输入重复 reconcile 合并为一轮；新地址/授权/环境版本更新才使相关候选失效。调用方取消等待不等于断开全局共享连接。认证撤销不得被普通 reconcile 自动恢复，须新的授权版本。

Snapshot 区分 `desiredState`、`phase`、`activeConnectionId`、`actualChannel`、`health`、`optimizationInProgress`、`retryAfter`、`traffic`。活动连接仍可用时保留 online，并单独标记优化；不能为了后台探测把页面闪成离线。每条快照附 revision 和连接代次，避免 UI 将旧通道与新计数拼接。

## 状态机与并发约束

控制器用串行事件队列修改状态，网络操作异步执行，结果带 `(lifecycleEpoch, authorizationRevision, environmentRevision, attemptId)` 回来；过期结果只清理资源，不覆盖新状态。

| 当前状态 | 输入/结果 | 动作 |
| --- | --- | --- |
| stopped | reconcile | 更新在线期望，进入 selecting |
| selecting | 首个 ready | 激活，进入 online，继续更优候选 |
| online | 相同 reconcile | 保持或合并已安排的核查，不重复建连 |
| online | 更优候选稳定 ready | handover；旧连接继续服务至交接屏障 |
| online | suspect | 有界探测并准备替代，保留可用连接 |
| online | 确认失效 | 隔离旧代，立即 selecting |
| selecting | 暂时全部失败 | waiting，按预算退避；新有效输入可提前唤醒 |
| 任意 | 授权拒绝/撤销 | blocked，释放候选，等待新授权 |
| 任意 | disconnect/dispose | 先撤销期望与代次，再异步清理 |

强约束：每个逻辑会话至多一个业务 active；每通道族每轮最多一个候选；重复事件不能累计定时器；关闭事件和错误事件都能终结 active；清理失败不能阻塞下一轮；业务超时不自动使通道失效。

## 选路与性能预算

所有允许分支同轮启动，LAN 不等待信令初始化。先完成者可服务；同一决策批次多个 ready 选优先级最高者，不为了等待 LAN 故意拖住已可用的中继。保留更高优先级候选，取消更低候选；同级健康连接不按瞬时 RTT 来回替换。

初始参数为待压测配置，不是体验承诺：环境事件合并 300ms；升级候选稳定确认 2 秒；升级冷却 15 秒（故障降级豁免）；LAN 建立预算沿用 8 秒、RTC 沿用 55 秒；优先路径后台复查从 30 秒开始指数退避，失败恢复从 2 秒开始；均加 ±20% 抖动并限制到 60 秒。后台输入关闭健康路径择优，但仍保留心跳和故障恢复。无候选时等待输入，不不停开启 socket。

每目标最多 1 active + 4 候选；全局候选预算初设 8，按目标公平调度，恢复断线优先于健康优化，不关闭其他目标的 active 来腾位置。并行表示分支不串行依赖，不代表多设备情况下无限制同时创建 peer。取消立即撤销逻辑候选，原生 open/close 未完成前仍占候选许可；清理积压达到阈值后停止新增，close 抛错则隔离该许可并记录 `cleanup_failed_quarantined`，防止反复超时无限分配。恢复在排队许可中优先于健康优化，同级 FIFO；已经启动的原生 IO 不抢占释放。

采样不重建会话列表；复用现有流量计数与 RTC stats，空闲时不主动产生大包测速。状态变化即时发布，速率快照最多每秒一次。控制器的计算/内存开销与固定候选数和固定窗口桶数有关，不随运行时长增长。

## 健康度与故障判断

健康状态为 unknown/healthy/suspect/unusable；分别保留传输存活与应用就绪，TCP 可写不代表 Host 可处理业务。硬关闭、认证撤销立即处理；短暂 RTC disconnected、一次慢响应先进入 suspect。速率为零是正常空闲，单个 RPC 超时不是网络断线。仍有持续收发但 RPC 堵塞时记录背压/服务响应异常，不能以“有流量”无限延长故障窗口。

复用 SDK 协议心跳作为应用健康输入，引擎不启动第二套 protocol.ping。网络事件要求尽快核查时由适配器执行有界探测；重叠探测合并，避免前一次未结束又发一次。RTC 可靠有序流仍可能被大消息阻塞：首版用传输进度、缓冲和应用心跳共同诊断，不把压缩当作消除了队头阻塞。是否增加独立控制流由慢速大响应验收决定，不以缩短所有超时解决。

## 安全交接与 Host 归属

候选经历 transportReady → peerValidated → prepared → active。prepared 只允许验证/探测，不发送用户写请求、不向 UI 投递业务事件。SessionBridge 从现有 connect 拆出验证和激活阶段，负责 Gateway 握手、快照与订阅；引擎不认识 conversation.create 等方法。

健康升级：准备新连接 → 暂停接纳新业务请求 → 有界等待旧在途调用 → 提交 active 归属 → 恢复事件/快照并开放新请求 → 旧连接 draining/关闭。新请求由上层短暂等待或明确提示重连，不形成无界队列。旧请求不能排空则延期健康升级。硬故障时无需等待旧连接，但已发送操作保留结果未知，不在新连接自动重发。Host 已接纳执行的操作不能靠切换撤销；长任务可继续运行，重新订阅后恢复显示，不等任务结束才迁移。

Host 维护以已认证安装身份与逻辑会话为范围的 active 归属，使用 Host 发放的 epoch/token、expectedActiveToken 与幂等 switchId 比较交换。确认丢失时查询同一归属或重试同一 switchId；不能盲目回滚并让两端各自使用不同 active。进程重启后重新取得 Host epoch，不拿本地重置计数覆盖新会话。新连接提交前旧代继续可用，提交后旧代不得再接纳新业务请求；队列中的未执行请求在调度入口再次校验代次，已接纳执行的请求可排空，结果不污染新 UI。

这是通道控制能力，不把证书、secret、切换命令塞进 Gateway 业务 schema。设计独立、传输无关的 channel control 契约，两端明确协商后使用；LAN 与 RTC 采用各自适配器承载同一语义，不能把 transport control 直接伪装成 Gateway RPC。具体帧编码和能力公告字段在协议阶段形成 schema/fixtures，再生成 SDK。未完成双方能力协商前不发送未知控制帧。旧端继续单路选中/故障重连，禁止宣称支持安全主动升级；回退由明确能力缺失决定，不能把身份校验失败当作回退理由。

## 统计与展示

原始统计归每条物理连接，活动快照只选 active 的通道/速度/本次连接累计，新物理连接归零；准备/废弃候选消耗单列诊断，不混入主界面。同一 peer 内 ICE pair 变化更新通道而不清零。底层采用现有压缩后 RTC payload、WSS 文本 payload 口径，不含 IP/TLS 重传；不能用于核对运营商账单。若需要跨切换累计，后续新增独立逻辑会话字段，不悄悄改“本次连接”语义。

Desktop 不再遍历同一逻辑会话的全部物理连接作为“当前”。它消费 Host 确认的 active 与同代统计；Remote 在确认前保留旧通道或显示正在重连。真实多个独立逻辑会话单独标识，不能按设备 ID 粗暴踢掉。

## 实施顺序与退出条件

| 阶段 | 改动与原因 | 完成条件 |
| --- | --- | --- |
| 1 引擎核心 | 纯 Dart 状态机、端口、假时钟、取消与预算；先验证不依赖场景 | 状态/乱序/重复触发/资源上限测试通过 |
| 2 接入故障恢复 | 适配现有 LAN/RTC；修 onDone；迁移 DeviceSession 重试；接入网络事件 | 四路并行、断网恢复、授权/手动断开回归通过 |
| 3 交接协议 | channel control schema/SDK、Host active 归属、SessionBridge 拆分 | 断在交接每一步、确认丢失、旧代排队请求、旧端兼容均通过 |
| 4 自动升级和展示 | 启用优先级升级、防抖、共享统计快照及两端 UI | 中继→IPv6→LAN 与反向故障切换实机通过 |
| 5 发布 | 两端匹配 Release、可关闭主动升级的策略开关 | 固定版本双端验收，记录日志、耗时和资源基线 |

阶段 2 只有恢复能力，不能当成完整需求交付；阶段 3 未完成不得提前开启健康路径迁移。每阶段保持可独立回归，但最终必须覆盖全部三项基础能力及三个外部触发入口。

## 验收矩阵与风险

- 确定性测试：所有路径完成顺序、重复 reconcile、环境连续变化、过期结果、关闭时 late peer、凭据更换、重试耗尽后新输入；断言唯一 active、无重复写、无遗留定时器。不同触发源产生相同输入时结果一致。
- 交接测试：握手拒绝、准备成功后旧链路坏、activate 响应丢失、Host 重启、排空超时、两端取消交错、多个合法逻辑会话；确认丢失不能双主，已接纳写操作不能重放。
- 网络实测：Wi-Fi↔蜂窝、企业 VPN/Tailscale 地址、VPN 开关、IPv6 地址变化、仅路由器上游断网、Host 侧换网、后台恢复。系统默认网络变化不等于私网目标不可达。
- 压力基线：至少 1000 次假网络切换，检查连接/任务/定时器数量回归零；双端原生持续 30 分钟切换，记录内存、FD、CPU 与重连耗时趋势。慢速大响应并发心跳测试须覆盖现有合法消息上限，不靠压缩重复字符伪造收益。
- 性能记录：从触发到开始探测、通道 ready、active 确认、首个业务成功分别计时；报告 p50/p95、故障中断时长、健康升级暂停时长、空闲探测流量。可行目标为硬断线确认后 1 秒内启动可运行的恢复尝试；最终连接完成受网络约束，不先承诺秒数。全局预算饱和的排队需单独计量。
- 发布回归：保留现有 Flutter 全量/架构检查、Rust Host 通道测试、Android↔Host 原生测试；UI 窄屏/后台返回/选中设备变化验证不丢失当前归属。旧端不具备新协议时只允许已验证的回退。

日志沿用标准纯文本，保留 target/session/connection/attempt 与代次、状态变化原因、阶段耗时、实际 pair 类别及释放结果；不打印正文、密钥或每帧重复数据。应用触发源在外层记录并关联同一操作 ID，引擎无须读取其语义。

## 已完成与待验证

实现入口：

- Remote `packages/codepet_connection_engine`：纯 Dart 契约、状态机、候选预算、单调时钟、30 个固定流量桶；功能/压力测试不启动 Flutter、App、账号或 VPS。
- Remote `lib/gateway/connection/`：身份已校验的候选、交接屏障、Host CAS 确认、事件游标恢复；不会重放结果未知的 RPC。`DeviceSession` 的实际通道重试由此接管，注入的旧客户端保留兼容分支。业务快照恢复失败仍由应用层重建投影。
- Remote `NetworkChangesBridge` 与 `ConnectionEnvironmentObserver`：Android 所有网络、链路与默认路由变化，以及前后台输入；不修改系统代理或 VPN 路由。
- Host `remote/channels/ownership.rs`、`session.rs`、`remote/connections.rs`：已认证范围内独立逻辑会话的 active 归属、请求双重校验、只投影 active 的流量；旧客户端语义保留。
- `protocol/channel/control/v1`：独立 types schema、fixtures 和 WSS/RTC 能力协商契约；通过现有生成器生成 Rust/Dart SDK。没有向 Gateway schema 加入控制命令。

核心测试在包目录直接执行 `dart test`；压力入口 `dart run tool/stress.dart 10000 16`。CI 有独立 Dart 作业，输出纯文本压力产物。架构测试禁止核心 import/export Flutter、业务、Gateway、IO 或网络依赖。原生适配器需要单独集成验收，核心性能数字不代表原生 ICE/带宽。

本地自动验证的执行命令与结果记录在 [连接引擎验收](../40-runbooks/connection-engine-verification.md)。仍未完成的实机条件：手机 Wi-Fi/4G 切换、企业 VPN/Tailscale、真实公网 IPv6/IPv4/TURN、后台长时运行和双端 30 分钟资源趋势。不得把 mock 延迟或本机 loopback 当成这些结果。

已知限制：WSS 只给出完整消息，无法拿到接收片段进度，因此短探测在已有大响应/活动 WSS 上保留最长 60 秒应用心跳宽限；硬关闭立即恢复，静默黑洞恢复依赖该宽限及原生 keepalive。native close 失败会保留隔离许可，可能要求重启进程；不能以释放 Dart 计数替代原生清理。交接提交后无法确认或恢复游标时显式失败，重新建立业务快照，保证不双写。
