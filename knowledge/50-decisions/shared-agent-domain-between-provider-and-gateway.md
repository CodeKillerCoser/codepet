# Provider 与 Gateway 共享 Agent 领域协议

## 背景与证据

此前 `provider/v1` 与 `gateway/v1` 分别声明 Conversation、Project、Turn、Approval、ConversationItem、Tool、Content、Model Selection、Authentication 和 Usage。它们在 wire 上大体同构，但生成后是两个不同的 Rust 类型，Host 只能通过逐字段 mapper 转换整棵对象。

2026-09-05 重组前的协议统计显示：Provider 有 167 个 definition，Gateway 有 157 个；两者存在 128 个同名 definition，其中 90 个原始 JSON Schema 完全一致。`crates/codepet-host/src/gateway.rs` 因此维护约 20 个纯结构复制函数；ConversationItem/Tool/Content 子树本身接近 200 行。这个重复既增加序列化前的内存峰值和修改面，也允许本应互斥的业务定义在两个协议中独立漂移。

资源身份又不能简单合并。Provider 调用边界需要 `deviceId + providerPluginId + providerInstanceId + nativeResourceId` 才能定位进程与实例；Remote 不应知道 plugin 和 device 内部路由，只应看到 Host 分配的 opaque `providerId + nativeResourceId`。同名 `RoutedResourceId` 曾表达这两种不同语义，是类型组织问题的直接证据。

## 决策

新增 types-only 的 `agent/v1`，版本仍为 V1。它是 Provider 和 Gateway 共同依赖的领域类型包，不是新的传输跳、服务端或 ACP wire。

`agent/v1` 统一拥有：

- Project、Conversation、TurnTask、Approval；
- ConversationItem 的封闭联合及 Tool、Content、Truncation 子树；
- Choice、ModelCatalog、TurnSelection 与 turn/create controls；
- ProviderAuthentication 与 ProviderUsage 等客户端可见摘要。

共享对象使用 Core 的 `RoutedResourceId { providerId, nativeResourceId }`。`providerId` 是 Host 分配并对客户端 opaque 的实例身份；当前内置 Provider 以 `providerInstanceId` 作为该值。Provider 私有的四段对象改名为 `ProviderResourceId`，只用于 Host→Provider 请求与进程/实例路由；`ProviderInstanceRoute` 继续用于实例生命周期。Host 在边界上解析和校验这两种身份，但不转换 Agent 业务对象。

Provider 与 Gateway schema 通过外部 `$ref` 引用 Agent 定义；Rust、TypeScript、Dart SDK 依赖并重新导出 `codepet-agent-sdk`。因此同一语言内两端得到同一个具体业务类型，Host 可以直接 move/clone，而不是逐字段复制。

`ProviderExtension` 不进入共享业务对象。Provider 请求、实例或能力确实需要的私有扩展仍留在 Provider 层；客户端可见业务对象只包含协议明确约定的字段。Gateway 的 event cursor、snapshot cursor、read-state 写入时机、usage 脱敏以及 capability 方法策略仍由 Host/Gateway 边界负责。

## 备选方案

### 继续保留两套业务 DTO，只优化 mapper

短期改动小，但重复定义和漂移风险仍在；投影器去重只能修复某个输出，不能从协议上保证互斥与唯一所有权，因此不采用。

### 把所有业务类型放进 Core

会让 Pet、LAN、Desktop 等低层包看到不需要的 Agent 概念，破坏依赖方向，因此不采用。

### 完全采用 ACP payload

ACP 对消息内容提供了有价值的参考，但不表达 CodePet 的 Project、Provider instance 生命周期、设备路由、Gateway replay 和常驻 harness server。`agent/v1` 可以保持 ACP 友好的内容建模，却不把 CodePet 的服务协议替换为 ACP，因此不采用整套替换。

### Agent 对象只携带 nativeResourceId，路由全部放 envelope

可进一步减少重复 ID，但会大幅改变现有 request/event DTO 和 Remote 的对象寻址方式。本轮目标是消除业务 mapper并解决大 Turn，而不是重写所有 envelope，因此暂不采用。

## 后果

- Provider/Gateway 共享业务实体 definition 的重复数降为 0；互斥结构只维护和验证一次。重组后两边仍有 19 个原始 JSON 相同的 method result/event/filter wrapper，它们保留各自边界类型名与演进权，不再内嵌复制 Conversation、Turn、Item、Tool 等对象。
- Host 仍保留安全边界逻辑，但删除 Provider→Gateway 的业务对象手写 mapper。
- Provider 返回对象和事件必须填充两段共享资源 ID；Host 必须验证其中 `providerId` 与被调用实例一致。
- Provider 私有请求继续使用四段 `ProviderResourceId`，不能把共享两段 ID 直接当作进程路由。
- 新增 SDK 依赖层：`agent -> core`，`provider/gateway -> agent + core`。SDK 导出、App staging 和独立 `cp-sdk-gen` 必须递归携带 Agent 包。
- 这项重组减少对象复制和 schema 漂移，但不能代替准确分页、Mapper 阶段的显式内容策略和 SDK Runtime 对最终 encoded Provider frame 的 16 MiB 检查；大 Turn 的主要字节来源仍由 payload ownership 规约治理。

## 验证

- `npm run protocol:check` 验证层依赖、跨包引用、Agent 负向 fixture 与生成物 freshness。
- Rust/TypeScript/Dart SDK 分别编译；Provider/Gateway SDK 能重新导出同一 Agent concrete type。
- Host 测试覆盖错误 `providerId`、错误 native ID、跨实例资源、事件路由与 cursor；业务对象路径不得再调用逐字段 mapper。
- 三个内置 Provider 的快照和事件均输出两段共享资源 ID，私有 extension 不泄露。
- 用同一历史样例复测业务 JSON、Provider stdio encoded-frame、WebSocket 压缩后字节和截断统计，区分“结构去重收益”“Provider zstd 收益”与“显式内容策略收益”。

## 状态

2026-09-05 接受并开始实施。协议基线提交为 `0af327e`。
