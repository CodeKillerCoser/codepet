# Provider、Gateway 与 Host 边界复核（2026-11-14）

## 背景与范围

用户要求对当前架构再做 review，重点核对插件—Host 的 Provider 协议、操作端—Host 的 Gateway 协议及共享对象。以当前工作区为准，包含尚未提交的 Registry 和 Provider 卡片调整；不能把该工作区等同于主干或用户当前运行的原生版本。本次只记录评估，不改协议或实现。

## 目标与非目标

确认哪些边界已清晰、哪些仍靠实现约定，并给出小步推进和验证路径。不重新设计三种 profile，不合并两层协议，不恢复压缩载荷透传，不做全仓重构或新一轮公网验收。

## 已清晰的部分

`protocol/v1` 是权威源，Provider/Gateway 各自按功能登记 Schema、methods/events；common 和 agent-domain 中立共享，兼容视图由生成器组装。公开 Provider SDK 只包含对应依赖闭包，Gateway 留在内部，两段分别编解码。stdio 三 profile 与独立 transport crate、业务运行时分层合理。Registry 的安装/查询/卸载统一编排与实例身份持久化分工也可保留。目前 Registry 是 PluginManager 的职责模块，不是另一份注册表或独立服务；无需为了类名再包一层。

## 发现、影响与建议

### 1. 插件身份与实例身份的命名仍混用（P1）

证据：`tauri_bridge.rs` 的 ProviderConnectionView.providerId 赋值为 catalog.plugin_id；`gateway.rs::gateway_provider` 的 ProviderSummary.id 则赋值为 record.instance_id。两者各自运行成立，但同名字段不能直接互传。安装包/进程归插件，会话/turn 归实例。

建议：新增的本机管理 DTO 使用 pluginId/instanceId 明确命名；现有 Gateway providerId 暂保留兼容，但在 Schema description 明确它是实例 ID。Provider 管理的 Tauri commands 应明确为本机管理合同，逐步生成 DTO/方法登记，不能扩展成隐含的第二套业务协议；OS 对话框等本机能力不必搬进 Gateway。

验证：一个插件多个实例，确保卸载只接受插件身份，业务路由只接受实例身份；运行跨资源实例一致性和旧客户端 fixture。

### 2. 通用 Provider、Agent 与 Observation 还未完全分离（P1）

证据：插件 descriptor 只有 instanceKinds，没有插件级功能声明；实例能力集中在项目/会话/turn/usage。订阅模块已存在，零实例也可订阅，但 event.notification.payload 是 JsonObject。Pet projection 仍理解 hook_event_name、sessionID、OpenCode properties 等来源字段；UI 则按方法前缀推断类型，而当前 ProviderCapability 枚举并没有 observation.* 方法。

建议：插件级声明支持的能力集合，例如 agent、observation、runtimeInventory，而非强制互斥的单一类型。一个插件可以兼具多种能力；纯通知插件允许零 Agent 实例，不需要伪造 Harness。为跨插件消费的 Observation 定义有类型、有版本的载荷及动作合同，原始来源数据由 Provider adapter 处理，保留必要扩展。只投递原始业务数据的扩展仍可独立携带 schemaId/版本，不能把 JsonObject 当作内容合同已完成。

验证：一个纯通知插件、一个自管运行时 Agent、一个双能力插件；UI 不再猜类型，新增通知来源无需修改 Pet 的来源专用字段解析。

### 3. 严格解码与版本演进规则没有闭合（P1）

证据：Initialize/handshake 主要协商整数版本范围，目前均为 v1；生成对象拒绝未知字段。源摘要用于产物追踪，没有在连接时承担兼容协商。方法能力只能表达“能调用什么”，不能证明同名对象的字段兼容。

建议：先明确兼容政策。保持严格解码时，对旧端不兼容的对象变化必须升级协议版本，或通过明确的 revision/feature 协商选择 wire；不能默认新增可选字段无害。文档应区分协议版本、manifest 格式版本、SDK 发布版本、capability revision、进程 generation。摘要可用于定位构建来源，不能直接把摘要不相等判为不兼容，因为说明文字也可能影响摘要。

验证：旧 Host/新插件、新 Host/旧插件、旧手机/新 Host、新手机/旧 Host 的兼容样本，要求在握手或能力阶段明确拒绝，不在深层 DTO 解码时才失败。

### 4. Provider 清单的删除与状态变化语义需补齐（P2）

证据：Gateway 有 provider.changed，载荷只能携带一个现存 ProviderSummary；没有明确的移除事件和清单 revision。protocol.ping 返回当前全量实例清单，已覆盖下一轮心跳剔除，但 stopped 不等于 uninstalled。

建议：保留心跳全量快照作为校正，同时定义清单版本及 upsert/remove 或清单失效事件。明确权威快照是替换集合，不能只合并新增；generation 表示进程代次，不复用作清单版本。纯本机插件清单与远程可操作实例清单继续区分。

验证：卸载、禁用、重装、断线重放、多实例、迟到旧事件；客户端不会重新插入已移除项。当前下一轮心跳行为正常，这是同步合同补强，不是已证实的现有故障。

### 5. 配对源归属已正确，跨通道合同仍是部分收敛（P2）

证据：gateway/pairing 维护现有四个 HTTP 绑定；云邀请中仍有手写 v2 envelope、加密/签名载荷字段和 json! 组装。LAN SDK 名称及 gatewayUrl/TLS 字段是明确保留的兼容产物。

建议：按配对准入语义整理尚未进入 IDL 的邀请对象，按绑定分别表达 LAN 与云入口；旧包名最后再改。ICE、SDP、DTLS 仍属于通道，不要为目录整齐移进业务 Gateway，更不能进入公开 Provider SDK。

验证：现有邀请/二维码 fixtures、过期、拒绝、撤销与重放；先保持 wire 不变，再单独讨论真正的跨通道对象升级。

### 6. 兼容生成物与持续验证还有收尾工作（P2）

证据：modular-contract 为保留旧包路径，将 Provider 传输握手类型合入生成的 core Schema，Gateway 的 Core SDK 因而也包含无关的 ProviderTransport 类型。内部 SDK 摘要覆盖整个 canonical 图。协议 CI 覆盖生成、导出和若干 Rust 测试，但该工作流未直接运行完整 Registry/Gateway 集成或仓库外 Dart 消费验证。

建议：保留现有兼容视图，增加独立 transport 输出后再让新消费者脱离旧聚合；包摘要可另提供依赖闭包摘要，整个源码摘要保留用于追踪。把完整卸载/清单测试和生成 Dart 消费检查纳入适当 CI；两仓联动记录精确协议来源。核心生命周期方法的 description 还应说明前置状态、重复调用与失败含义，而不只满足非空检查。

验证：修改 Provider 专属传输配置不应改变 Gateway 业务 DTO；导出包边界、语言编译、协议兼容矩阵、独立插件黑盒与两端联合验收各自覆盖对应风险。

## 推进顺序

先确认身份词汇和插件级能力模型，再明确版本政策；随后补 Observation 载荷及清单同步合同。每步先改对应 IDL、生成并验证，再接 Host/Provider/操作端。配对剩余对象和兼容包清理分开推进，不与业务 wire 升级捆绑。

## 本次验证与未知项

实际运行 protocol:check 32 项、providers:test 5 项，均通过；说明当前生成及导出约束成立，不等于上述演进规则都已实现。本次未重跑原生联合验收、公网信令或真实模型。现有仓库外自定义插件的实际版本分布未知；兼容政策需在这些插件公开稳定前确定。

## 版本政策后续决定

用户随后确认前向兼容与 version/minVersion 方案。上文第 3 项的“严格解码下升级”是当时现状与备选，后续实施以 [前向兼容版本决策](../50-decisions/provider-gateway-forward-compatible-versioning.md) 和 protocol/v2 草案为准；当前 v1 codec 仍未改变。

## 本次 review 后的推进

严格解码与版本演进一项已按后续确认方案落地到 v2：独立 version/minVersion、2/2 首发、已知数据校验、未知接收值保留和安全消费、无静默回退/重放。源目录与服务版本解耦，公开 SDK 与内部 Gateway 分发边界保留。上述 v1 现状是 review 时证据，不代表当前仍以 v1 生成；其余身份命名、Observation 类型化、配对云邀请 IDL 及 transport 聚合视图建议没有借本轮扩展。

2026-11-15：用户确认的第 1、2、4 项已进入实施：跨平台生成路径、插件显式组合功能与身份说明、Gateway 清单移除/修订号；两层增量业务范围为 3/2。第 3 项 Observation 业务载荷类型化未包含在本轮。当前设计与验证入口见 [身份、功能与清单修订号](../50-decisions/provider-identity-features-and-inventory-revisions.md)，不再以上文历史现状代替当前实现。
