# 当前模块边界 Review：以局部整理为范围

状态：2026-11-11，源码审阅与整理建议，未修改实现。本轮以用户最新要求为准：评估现状、识别共性和边界问题，不启动前述完整改造计划，不进行大面积重构。

后续推进已收敛为 [局部整理与收敛计划](observation-and-sdk-refactor-plan.md)。用户明确任务谱系暂不考虑，相关现状记录仅作历史核对，不纳入迁移或删除范围。

## 背景与范围

审阅 Host/Gateway、LAN/RTC 与共享会话、Provider bootstrap/initialize、三个内置 Provider、共享观察与存储、SDK 导出和 Tauri 兼容桥。结合当前源码与附近测试，不把此前提案当作已经实施的事实。

最新分发定位：随 App 的开发工具面向 Provider 扩展，只包含 provider 与所需 common；Gateway 是官方桌面/手机/TUI 等调用入口的内部规范与 SDK，不以建设第三方开放接入平台为本期目标。RTC/LAN 仍是应用内部可复用连接实现。

## 结论

主干仍清楚：界面 → 本地桥或远程 channel session → Gateway → PluginManager → 独立 Provider → 原生工具。观察链与 Agent 控制链分流，物理 stdio、消息编解码、业务运行时也已有分层。没有证据支持推倒重建 Host、引入全新插件宿主或将每层拆成独立 crate。

需要整理的主要是六处：初始化公共规则与成功状态、通用日志放在 RTC 子模块、Host 共享存储归属在 provider-data、Pet 来源/原生事件耦合、SDK 分发范围、旧兼容桥与现状文档。另有一个小的产品专属 helper 放置问题。均可分批处理，不能因文件大或代码相似就统一原生执行模型。

## 已有清晰边界，应保留

### Bootstrap 与 initialize 不是重复机制

现行顺序：Host 准备目录/启动日志与环境 → 启动插件 → SDK transport bootstrap → provider.initialize → describe → 按配置创建/启动实例或观察订阅。

- `sdk/rust/codepet-provider-sdk/src/transport/handshake.rs:71` 的 negotiate 是 CPMX 物理传输握手：版本、功能、收发容量及 CONFIRM/READY，带超时。
- `transport/mux/mod.rs:66` 在 connect 中完成 bootstrap；`runtime/mux.rs:46` 在其成功后才构造 Provider 和业务分发。
- `providers/manager.rs:615` 调用业务 initialize，传 Host 身份、业务版本范围和目录；这才是插件业务装配入口。
- Codex App Server 自己的 native initialize 是 Provider 与底层工具的另一层握手，不应与 Host→Provider 初始化合并。

建议补清楚名称和状态说明，继续复用已有实现。此前“增加公共插件启动底座”的设想不应被理解为当前没有 bootstrap。

### 共性已经被复用

三个 main.rs 都只有创建 StdioServerOptions 和 serve_stdio，属于正常薄入口，无需为十余行代码再加框架。SDK 已提供 runtime → message → transport、跨平台进程树、background_probe、RuntimeScanner/选择存储端口和事件发布；`codepet-observation` 已共用 Hook 配置安装、文件锁、endpoint/token、本地接收与投递；provider-data 共用计量、游标和会话事实存储。

OpenCode `background_probe.rs:1` 仅约 20 行：补 OpenCode 环境和探测配置后调用 SDK background_probe，并不是重复实现进程管理。Codex 的 stdio App Server、Claude 的 CLI stream-json、OpenCode 的 HTTP/SSE runtime 有实质差异，保留各自 client/protocol/mapper 和执行生命周期。大型 provider.rs 可按明确职责拆文件，但行数本身不构成重构理由。

## 具体发现与局部建议

### R1 初始化规则三份重复，成功状态分散（优先做小修）

**证据**：Codex `provider.rs:1667`、Claude `:1575`、OpenCode `:1130` 各约 58–59 行 initialize，协议范围、Host 身份检查和绑定基本相同。三者均先写 host_device_id/initialized_client_id，再调用可能失败的 data.initialize；Claude/OpenCode 还会调用可能失败的 start_collection。SDK `runtime/mux.rs:66,171` 又单独维护“initialize 响应成功”的布尔状态，并在 ping 快路径使用。Provider 观察订阅却通过 host_device_id 是否存在判断已经初始化（各文件 `:1659/:1567/:1122`）。

**影响**：初始化失败后，Provider 对象可能已绑定身份，而 runtime 仍未成功；直接复用该对象、重试或并发请求时会遇到两种就绪定义。正常 Host 在初始化失败时会 fail_started_process，所以这里不宣称已经发生线上半初始化或数据损坏。

**建议**：先抽取小型版本/身份校验与初始化状态 helper，明确未初始化/初始化中/成功/失败的判断，成功后才对外视为可用。Provider 保留存储、采集器和原生 scanner 的业务步骤及清理策略，不让 SDK 接管 SQL。统一重复初始化同身份/不同身份/路径变更的结果。

**验证**：三 Provider 共用合同用例覆盖无效目录失败后重试、初始化失败后的业务调用、并发 initialize、同身份重复请求、Host 变更；保留 bootstrap/心跳/EOF/shutdown 测试。不能只把重复代码移动后宣称解决状态问题。

### R2 通用通道诊断反向依赖 RTC（适合最先整理）

**证据**：`remote/channels/lan/websocket_diagnostics.rs:1` 与共享 `channels/session.rs:365` 引用 webrtc::diagnostics::Diagnostic；该类型在 `webrtc/diagnostics.rs:85` 对 channels 开放，但输出 schema=codepet.rtc.v1。LAN 的 `lan.open`、`lan.stream.ended` 等也通过这个设施记录。

**影响**：不意味着 LAN 真的通过 RTC 传输，但模块归属和日志名称误导依赖，单独阅读/测试 LAN 也要理解 RTC 子目录。

**建议**：在 channels 下提取中性的诊断输出、连接 ID 和有界 sink；ICE/SDP/candidate/RTCPeerConnection 统计保留在 webrtc。先保留或明确迁移现有日志字段，避免破坏排障脚本；不把所有诊断迁入 common SDK。

**验证**：日志脱敏、丢弃计数、不阻塞发送、LAN stream 错误与 channel control 日志回归；依赖检查防止通用 session 再依赖 webrtc 子模块。

### R3 LAN listener 还承担远程运行时装配（命名/装配整理，不必重写协议）

**证据**：`lan/listener.rs:144` 创建 SignalingServiceController，`:160–161` 挂载 RTC offer 与 channel-bootstrap；`webrtc/cloud/service.rs:24` 加载配置并按配置启动信令 worker。现有 `channels/session.rs` 的 GatewaySink/GatewaySource/GatewayChannel 已共用会话鉴权、并发、事件、关闭和背压，不是两套业务会话。

**影响**：信令生命周期挂在名为 LAN 的 server handle 上，后续“启用哪种入口”的配置容易误操作整套远程生命周期。把 LAN 和 RTC 的来源目录分开，本身不是坏事；经 HTTPS 交换 RTC 信令也合理。

**建议**：先明确一个 Remote runtime 装配入口拥有 listener/session registry/signaling controller，LAN/RTC adapter 只负责各自建链与帧适配。可先用小 facade/文件职责整理表达所有权，不新增服务进程，不重做信令，不因换通道重建 Provider。

**验证**：信令配置热切换不关闭健康 LAN/RTC 会话，停单入口不意外停止另一入口，共享 credential 撤销仍覆盖所有连接；沿用既有多通道及 ownership 集成测试。

### R4 Host 的共享状态 DDL 位于 provider-data（中等优先，先澄清归属）

**证据**：Host `storage.rs:6` 委托 codepet_provider_data::app_state::open_app_state；后者的 app_state.sql 定义 Host 身份、Provider registry、会话/未读、队列及远程凭据等 Host 表。ProviderData `lib.rs:82–84` 同时从环境取得共享 app-state 路径并直接打开；manager.rs:558,567,615 则分别通过启动环境和 initialize 传日志/存储配置。独立 usage.sqlite 与共享 app-state.sqlite 同时存在。

**影响**：SQL 事实已共享，避免了重复实现，但 crate 名称和依赖方向让 Host 状态看起来由 Provider 业务层拥有；多个组件可以发起同一 schema 初始化，配置来源也不止一个。不能继续简化成“所有 Provider 都只用独立库”。分表也不是访问隔离。

**建议**：本轮先形成 Host 共享元数据 vs Provider 私有采集数据的所有权清单，统一 InitContext 中配置的权威来源；启动前日志 stderr 捕获仍需保留。后续仅在需要修改存储时，把共享 schema/open/migration 移到中性内部模块，或由 Host 提供具体状态接口。不合库、不立刻改所有 Provider SQL，也不将持久化塞进 SDK runtime。

**验证**：共享 schema 首次创建/重开/不支持版本、Host 与三 Provider 并发访问、原已读和计量行为、路径重绑定拒绝；如果移动模块，编译 Host/Tauri/Provider 的公开 re-export。

### R5 Observation 的安装机制已独立，语义归并仍耦合 Pet（扩来源时再处理）

**证据**：`codepet-observation/src/lib.rs` 已封装安装/互斥/投递；三个 Provider 的 event_subscribe 转发给各自 observation。Host `pet_gateway/mod.rs:15` 写死三个来源，`projection.rs:34,58` 识别原生 Hook/OpenCode 状态。Provider 协议 event.notification 的 payload 仍是原始 JSON。

**影响**：目前三个内置来源能工作，新增钉钉/飞书时才会明显扩大 Host 中的来源判断；观察能力与 Agent 实例发现也未形成可独立声明的合同。

**建议**：复用已有 codepet-observation 作为机制层；在实际扩来源那次变更中定义最小标准信息和动作描述，将原生解析/任务归并放来源侧，Pet 只做展示投影，来源按能力发现。当前不必先升 Provider/Pet 版本，也不需要重建一套观察宿主。

**验证**：迁移现有 projection 的子代理、工具失败、迟到轮次、终态与缺口用例，加一个本地 observation-only fixture 验证无 Agent 启动；真实 IM 接入另行验收。

### R6 SDK 源码分层好于分发边界（小范围收敛）

**证据**：Gateway/Provider 生成库不依赖 LAN。`scripts/stage_provider_plugins.mjs:305` 却整目录复制五组协议，cp-sdk-gen 默认支持 provider/gateway/lan-channel，provider-runtime.mjs 逐项嵌入手写 runtime。已安装 App 与此一致，带入 fixture 和 RTC 文档。

**影响**：随 App 的扩展工具暴露了官方客户端内部物料。现有 staging 测试恰好锁定这种全量行为，因此“测试通过”不能证明分发边界符合最新要求。

**建议**：先给 App 增加 Provider-only 分发 profile：仅 Provider 及必需共享输入、相应说明和生成目标；Gateway/channel 留仓库内部。发布 fixture 校验与源码校验分离后再排除 fixture，不能直接删文件。common 可先作为公共入口/facade收敛，是否物理合包不属于这次 review 必做项；methods/events 改名也可单独处理，不捆绑 Provider v2。

**验证**：文件白名单和禁止项检查、Provider 输出依赖闭包、App 最终包离线导出/仓库外编译；保持源码 fixture 负例测试失败防线及 runtime 导出清单完整性。

### R7 兼容桥仍在生产装配，部分架构文档落后（先盘点与改文档）

**证据**：`src-tauri/src/lib.rs:469` 仍创建 RuntimeGatewayState，并启动兼容 event bridge；tauri_bridge 同时有 canonical codepet_gateway_request、旧 runtime_gateway_request 与私有 pet_gateway_request。仓库生产前端检索未发现 runtime_gateway_request 的调用。`knowledge/10-architecture/runtime-topology.md:9,14` 仍将旧 collector 和 Desktop Companion 写作现行组件；frontend-backend-boundary.md 后端描述也仍指向旧采集链。较新的活动订阅文档与当前 Pet 源码已采用 Provider 观察链。

**影响**：入口重复并不直接证明错误，但容易让后续功能选错合同；旧文档会让人误判当前仍需开发或复用哪些能力。仓库未检出调用方不等于外部使用者不存在。

**建议**：先列当前生产消费者并为旧入口标注兼容用途，修正文档事实；确认外部/自动化调用后再按独立小改删除无消费者的桥。不一次删除整个 runtime_gateway、旧 agent 或 activity 目录。

**验证**：生产入口检索、命令注册/事件订阅检查、Tauri 启动与 canonical request/event 回归；旧路径删除前保留 fixture/集成行为证据。

### 附：一个低优先级 helper 放置问题

`sdk/.../local_runtime.rs:756` 直接提供 opencode_environment，而模块首注释说产品布局留在具体 Provider。这是 SDK 中一处具体产品配置规则，与公共 PATH/进程探测机制不同。可在下次修改该规则时移回 OpenCode adapter 或单独的产品 helper，保留可复用的进程/环境机制；目前该包装避免多调用点不一致，不值得单独扩大重构。验证沿用 OpenCode 环境隔离、探测和 native runtime 测试。

## Tauri IPC 与 Gateway 收敛专项核对

用户进一步要求核对“IPC 只作为通道，通道之上统一 Gateway”。结论：当前 Agent 会话业务已经复用同一 Gateway，但整个桌面 App 的业务接口与客户端会话尚未完成统一，不能称为 IPC 已全部收敛。

### 当前调用路径

| 场景 | 实际路径 | 判断 |
| --- | --- | --- |
| 会话/队列/用量等 canonical 业务 | frontend/lib/codepetGateway.ts:9 → invoke(codepet_gateway_request) → tauri_bridge.rs:35 → ProviderGatewayService.dispatch_for_local_client | 已使用手机相同的 JSON-RPC 契约与业务服务 |
| LAN/WSS、RTC | adapter → remote/channels/session.rs → dispatch_for_caller_scope / subscribe_events | 两类网络通道已共用会话与业务服务 |
| 桌宠 | petGateway.ts:4 → pet_gateway_request → 私有 Pet envelope/dispatcher | 独立协议，尚未归到统一客户端 Gateway |
| 任务谱系 | taskLineage.ts:16 及 lineageApi → task_lineage_* commands → 任务领域服务 | 领域有独立实现，但客户端合同仍绑定 Tauri |
| Provider 探测与连接管理 | api.ts、providerRuntimes.ts → 多个 commands + provider-connection-status/provider-runtime-changed | 部分本地管理面未进入 Gateway，新增客户端不能直接复用整套入口 |
| 配对、远程运行配置 | remoteAccess.ts → remote_access_* 等 commands | 本地管理与原生 UI 操作混合，应分别归属，不放进远程准入前的业务握手 |
| 设置、事件查询、宠物库等 | api.ts/EventJournal → 独立 commands/events | 仍为本地业务接口，是否需要跨客户端复用应按场景决定 |
| 窗口、透明/穿透、拖动、系统权限、剪贴板、系统文件选择 | Tauri/platform/plugin API | 原生宿主能力，保留独立平台适配合理，不必机械包装成网络 Gateway RPC |

### 请求共享程度高于事件和客户端生命周期

canonical 请求已经共用，但 `codepetGateway.ts` 直接依赖 Tauri invoke，还没有可替换 transport 的 connect/request/events/close 客户端装配。`tauri_bridge.rs:760–814` 在 App 启动时从当前 cursor 订阅，广播 codepet-gateway-event，同时另发 Provider 状态事件及 v0 兼容事件；页面 listen 不是客户端自身发起的带 cursor 的订阅。

网络 session 则有 credential 绑定、handshake、订阅及 replay 边界。两者共用底层 event bus，但尚非同一种客户端会话/恢复合同。不能据此推断当前页面必然丢数据：页面有自己的快照、revision 和事件处理，需按业务复现；这里只确认它们尚不能透明替换通信方式。

另外，`runtime_gateway/transport.rs:27` 虽名为 LocalTransport，承载的是旧 v0 Gateway envelope，不能当作 canonical IPC/WSS/RTC 已统一实现的证据。本地 canonical 请求目前使用固定 desktop-main 身份、请求级在线 guard；这符合已有 [本地连接规约](../60-rules/local-gateway-client-presence.md)，不是需要改成长驻假在线的错误。新 adapter 应提供稳定、可信的调用上下文，网络入口仍经认证，不得复用可信本地准入接口。

### 启动顺序与合理差异

`src-tauri/src/lib.rs:469–488` 先管理已创建的 Host/Gateway 状态、建立本地事件桥，再后台启动 Provider、Pet 和 Remote listener；Tauri commands 在 builder 中注册。canonical IPC 并不依赖网络端口先启动。客户端 UI 已可发请求时，后台服务仍可能 starting，应有明确可重试状态。

合理模型是：服务端先创建共用业务服务和状态，再按配置装配各 adapter；客户端先选定/建立通道，再进行适用于该通道的 session 建立、能力/版本协商及业务调用。Tauri 原生桥的身份和就绪机制与网络配对认证不同，不要求通过本地 IPC 走 LAN TLS 或 RTC SDP，但业务结果、错误、事件及取消语义应一致。

### 本轮应收敛到的局部目标

1. 分类现有 commands：跨入口业务、本地管理业务、原生平台操作、旧兼容。先形成实际消费者清单，不一次性搬迁全部方法。
2. 在 canonical Gateway client 外加可注入的应用内部 transport，集中 Tauri invoke/listen；页面保留 typed 方法调用，不能直接判断 WSS/RTC。
3. 优先明确事件订阅、cursor/replay、页面重挂与取消的合同，先验证既有会话业务在 adapter 下行为一致，再迁移 Pet 等真正需要复用的本地业务。
4. 本地管理业务进入统一 Gateway 时明确 local-only 上下文与事件范围；统一协议不意味着向手机开放全部本机操作。配对/信令的原生 UI 步骤仍通过管理/平台适配，避免将准入依赖反向塞入业务握手。
5. 原生窗口操作继续走平台接口。前端可共用业务服务，不要求手机/TUI 能执行桌宠穿透、窗口拖动或系统剪贴板动作。

验证路径：对同一业务运行本地/网络 adapter 合同测试（请求 ID、错误、取消、快照+事件衔接、cursor 过期、订阅释放）；检查 local-only 方法/事件不可经网络调用；保持现有本地请求在线 guard 与无手机连接时的用量查询回归；最后检查桌宠原生焦点与窗口行为。本补充只做源码和附近测试静态核对，未重跑 Vitest/Cargo/实机或修改业务实现。

## 建议整理顺序

1. 本次先交付 review；不执行实现变更。
2. 若开始整理，先修正现状文档与入口标注，接着提取通用 channel diagnostics；两项均不改业务 wire。
3. 独立小改处理 InitContext 校验/成功状态及失败重试测试，不合并三种原生 runtime。
4. 再处理 App Provider-only SDK 物料 profile，完成最终包导出验证。
5. 存储模块中性化、Observation 标准化及 Remote 装配边界在对应功能需要时推进，不作为上述小整理的前置。

上述顺序不要求 common 合包、Gateway/Pet 统一入口、Provider v2、新本地 IPC、第三方 RTC 支持或合并数据库。前述完整计划保留为讨论材料，不能转化为自动执行清单。

## 验证结果与限制

本轮执行：

```sh
node --test tools/protocol-codegen/test.mjs scripts/stage_provider_plugins.test.mjs crates/providers/codepet-provider-opencode/tests/observation-plugin.test.mjs
```

30 项通过、0 失败、0 跳过，覆盖当前协议引用/生成新鲜度、物料 staging 和 OpenCode 原生观察插件。该结果不验证 Rust 的初始化失败/并发路径、真实 RTC 多网、数据库并发或原生桌宠交互，也不证明上述目标边界已经实现。

另执行 git status --short、源码及附近测试阅读、文档本地链接检查与 git diff --check。没有修改生产代码、协议、SDK 或构建脚本，没有运行 Cargo/Flutter/实机测试或安装新包。

## 知识沉淀与未知项

本文记录可复核证据与局部调整候选，不将建议写成已交付事实。开始实施时再更新具体模块文档与相关规约，不新增中央知识映射。需补核外部旧接口使用者、初始化异常的运行时复现、未来观察动作范围；其余风险各有上文验证路径。
