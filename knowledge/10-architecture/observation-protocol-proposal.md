# Observation 能力集合的协议拆分提案

> 已合并为 [Observation 与 SDK 完整改造计划](observation-and-sdk-refactor-plan.md)。本文独立 plugin/observation 包和公共 Pet v2 SDK 的早期设想已收敛；采用三类公共 SDK、Provider 内部能力模块及 Gateway 本地 pet 方法组。

状态：2026-11-11 设计评估，未实施，方法名为建议。本期 Observation 仅服务桌宠，不建设通用收件箱或桌面完整对话。

后续 SDK 边界评估：用户提出对外仅提供 common / gateway / provider。本文 plugin、observation 等描述内部协议职责，不要求各自发布独立 SDK；公共导出方向以 [SDK 分发评估](sdk-public-surface-and-distribution-review.md) 为准，Observation 与公共插件生命周期应作为 Provider 扩展能力的模块组织。

## 背景与证据

`protocol/provider/v1/manifest.json` 同时定义 provider.initialize/ping/shutdown、Agent 控制、event.subscribe/unsubscribe 和 event.notification。`ProviderPluginDescriptor.instanceKinds` 必填且 minItems=1，纯观察插件仍需声明 Agent 实例类型。通知正文是 JsonObject，`pet_gateway/projection.rs` 因此解析 Hook/OpenCode 原生字段。

Host `providers/process.rs` 使用 Provider 的 ProtocolClient/ProtocolMethod/WireMessage，`manager.rs` 固定执行 provider.initialize/describe。现有 subscriptions.rs 按 plugin+generation 共享一个上游订阅，给本地消费者有界分发。Pet v1 则以 tasks/approvals 为中心，action 使用 kind+多个 optional 字段，响应仅 accepted/revision，尚不足以表达通用信息及实际动作结果。

生成器 `validatePackageReferences` 限定 pet 只依赖 core；`cp-sdk-gen` 当前只接受 provider/gateway/lan-channel。拆包还必须扩展生成目标、运行时装配和资源分发，不能只改 schema。

## 目标

- 同一个插件可提供 Provider 控制、Observation，或两者；Observation-only 不要求 Agent instance、harness 或会话方法。
- 订阅、信息和动作标准化，原生来源解析留在对应实现，Pet Gateway 只做桌宠投影。
- 复用现有传输与进程管理，保留独立控制链路，不修改手机的 Gateway 业务契约。

## 非目标

不同时建设新的网络通道、UI 插件系统和通用历史消息数据库。不把具备 Observation 等同于有回复或审批权限。不向手机开放本期 Observation 方法。

## 实现路径

### 1. 独立的通用插件握手

建议新增 `protocol/plugin/v1`，只依赖 core，拥有 plugin.initialize/describe/ping/shutdown、插件身份、服务版本协商与公共目录 DTO。现有 stdio mux 的 framing、压缩、控制帧、超时与有界收发从 Provider SDK 抽成公共运行时；不另造传输，也不在 channel 层加入业务对象。

插件安装 manifest 显式标识新的 bootstrap 类型/版本，声明所提供服务的版本范围，例如 provider 和 observation。Host initialize 提出支持范围，插件返回选定版本；Host 校验 manifest 与运行结果一致，不支持的可选服务独立标记不可用。静态服务声明与运行时的账号健康/动作可用性是不同事实。

能力结构示意（不是最终 schema）：`services: [{name: 'provider', versions: ...}, {name: 'observation', versions: ...}]`。同进程组合两套 dispatcher，只握手一次并按命名空间分发，不维护第二条 stdio reader/writer。服务声明不含每个来源当时可用的回复按钮。

### 2. Observation v1

新增 `protocol/observation/v1`，只依赖 core，建议最小方法：

| 方法/事件 | 契约 |
| --- | --- |
| observation.sources.list | 返回来源身份、名称、账户展示、健康、能力 revision、支持的方法 |
| observation.subscribe | 针对明确 source 建立订阅，返回 streamId、已知信息快照与 sequence fence |
| observation.unsubscribe | 幂等取消观察，不停止同插件的 Agent 控制 |
| observation.changed | 判别联合：信息 upsert/remove、来源状态变化、gap、动作结果更新 |
| observation.action.execute | 可选，执行来源声明的动作，返回可查询的执行记录 |
| observation.action.get | 声明 action.execute 的来源必须支持，用同一 requestId 核对状态 |

来源由 pluginId+sourceId 唯一寻址；sourceId 是插件内稳定的观察绑定，不依赖 ProviderInstanceId，账户/租户等区别必须体现在绑定身份中。更换账号不得复用旧来源去路由回复。前端只获得 Host 分配的 opaque 来源/信息/动作 ID，不直接选择插件或账号。

订阅上下文绑定 plugin generation+sourceId+streamId。sequence 仅在该 stream 内单调，用于去重与检测缺口，不冒用 Remote 的 eventCursor。快照和 fence 必须来自同一原子状态，Host 在 subscribe RPC 前准备事件缓冲，按 fence 合并先到事件。

快照表示插件当前知道的有界状态，不是原应用完整历史。Hook 丢事件且无法重建时，来源或相关活动保持 unknown/有缺口，不把重新订阅视为已恢复真实状态。重启更换 streamId，旧流事件丢弃，来源返回的新快照替换该来源的已知视图，不跨流比较 sequence。

ObservationItem 包含来源内 itemId、revision、标题/摘要、时间、可用动作和判别 kind；建议区分 activity、message、request。活动状态与消息状态分别定义：不能为普通 IM 消息伪造 running/completed。事件 eventId 用于投递去重，itemId 用于原位更新，revision 用于拒绝旧更新。删除/撤回使用 tombstone 或可比较版本，避免乱序 upsert 复活已撤回消息。

Codex/Claude Hook 与 OpenCode 的原生事件解释、任务轮次归并和错误/子代理过滤迁至对应 Observation 实现；Host 保留来源间隔离和 Pet 展示逻辑。归并算法可复用 Rust 模块，但契约不传裸 Hook JSON让 Pet 判断来源。未知扩展不成为 UI 的必需输入。

### 3. 显式动作

动作描述绑定 item/revision，包含 actionId、kind、label、enabled 及禁用原因。首期仅规定 open 和 reply；未来审批等增加独立 typed variant，不借普通 reply 文本承载决定。

execute 携带 requestId、source/item/action 身份、expectedRevision、capabilityRevision，以及由 kind 判别的 typed input。重复 requestId 同参数返回同记录，不同参数冲突；同 ID 在结果未知时用于核对。Host 负责绑定与去重账本，插件也应使用稳定执行 ID 识别重复调用。声明为非幂等副作用，不因账本去重就宣称原生服务 exactly-once。

结果区分 accepted/running/succeeded/failed/outcomeUnknown；accepted 只表示已受理。连接断开或进程崩溃后不能盲目重发，get 无法核实时维持 unknown。插件重启后的去重保留周期和结果查询责任必须随实现明确，不能靠进程内 Map 保证。

open 返回结构化 ActivationTarget：应用、深链或终端/IDE 会话引用，由 Host 的平台接口执行并记录最终结果；不能仅以目标解析成功就报告已激活。reply 由所属来源执行，来源不支持则不声明；Agent 快速回复不得暗中接管原生 App 或强制获取写锁。通知内容不能变成任意可执行 shell。

### 4. Pet 接口

建议新增 Pet v2，将任务专用快照改为 PetItem 的判别联合，呈现源信息、活动状态、文本、可用动作与动作结果；pet.action 请求改成 actionId+requestId+expectedRevision+typed input，增加结果核对。Pet source.setEnabled 仍表达用户偏好，由 Host 调整订阅，不把关闭桌宠误作停止插件进程。

Pet 协议只依赖 core，不引用 Provider/Agent/Observation 的进程路由 DTO。Host 将 Observation 标准信息投影成 Pet 展示对象，维护 opaque ID 到真实来源动作的绑定；不在 Pet schema 放插件凭据、系统命令或远程控制生命周期。

### 5. 版本与迁移

新增 plugin/v1、observation/v1 各自协商版本；现有 Provider v1 与手机 Gateway v1 不因新包自动升版。先保留 Provider v1 的旧生命周期和 raw observation 方法作为显式旧接入路径，新插件走公共 bootstrap。不要从 v1 删除方法却仍宣称对旧插件兼容；正式删除时单独确定 Provider 版本或结束旧支持的发布边界。

Host 根据安装 manifest 选择新旧入口，不通过先调用新方法、超时后猜旧方法自动探测。单插件单来源只能启用一条观察链路，避免新旧重复安装 Hook 和重复通知。内置插件可随 Host 同批迁移；独立插件按 manifest 显式选择支持路径。

Pet v2 与前端配套发布，并通过现有 webcontent/native API 能力检查阻止旧壳加载不兼容页面。新版普通框架不认识的服务不能泄漏到 Remote provider.list；只有 provider 服务中的有效 Agent 实例才注册 Remote。

## 涉及模块

- protocol/plugin、observation、pet：新握手、标准信息和 typed 动作，Provider/Gateway 保留控制边界。
- protocol/codegen.json、生成器依赖审计与 cp-sdk-gen：新增 package/role、fixture、digest lock、组合 dispatcher 和独立导出支持；不直接改生成文件。
- Rust SDK 公共运行时与 workspace re-export：解耦 Provider 的 mux/进程通信，保留超时、背压与关闭语义；Observation-only 可独立编译。
- Host catalog/process/manager/subscriptions/registry：服务协商和分发，按 source+generation 组织订阅，Agent 实例只在 provider 服务下创建。
- 三个内置 Provider 与 observation 共享代码：迁移原生解析及 Hook 安装生命周期，不重复安装或启动 harness。
- Pet Gateway、Tauri bridge、前端 model/两套 UI：来源动态发现、快照和动作适配；原生激活服务实现平台差异。
- scripts/stage_provider_plugins、webcontent 兼容和 App SDK 资源：分发新包与旧插件支持说明，避免只在工作区可编译。

## 风险与测试计划

- 纯观察仍触发 Agent：以 observation-only 插件运行 Host，断言不调用 instance.start/conversation/turn，不出现在 Remote Agent 列表。
- 双能力生命周期串线：只开 Pet 不启动 harness；取消观察不影响 Remote 活动 turn；Agent releaseControl 不终止观察。
- 快照/事件竞态与缺口：RPC 回包前发事件、重复/乱序/删除重放、插件重启旧事件、慢消费者断开均有 fixture 和集成测试。
- 跨来源错误动作及重复发送：双账号同 itemId、过期 revision、结果未知、重启后核对、同 ID 不同 input 冲突。
- 迁移回归：新旧 manifest、版本不兼容、单路 Hook 安装、Provider 查询与手机 Gateway 合同回归；检查 Tauri 对新枚举的完整匹配。
- 原生 UI：中文快速回复、焦点恢复、点击穿透、应用未运行和多窗口精确激活，在实际 Tauri 与目标平台验收。

实施时运行 protocol:check、sdkgen:test、公共 SDK 与 Host/Provider 定向集成测试、前端测试/构建及 App SDK 导出黑盒检查。范围为协议评估，本轮仅源码/测试约束静态阅读和 git diff --check，未改协议或运行应用测试。

## 知识沉淀与未知项

实施后更新协议依赖规约（新增 plugin/observation 层仍只依赖 core）、Provider 活动订阅架构和 SDK 作者指南。不要提前把提案写成已交付事实。manifest 字段命名、旧路径支持期限、动作账本保留周期、首个 IM 来源仍待确定；本文方法和 DTO 应在实现前经 fixture 校验收敛。
