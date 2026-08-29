# Runtime Gateway 与远程控制架构

> 文档状态：本页是早期 Runtime Gateway 长期设计稿，其中把独立 `codex app-server` 写成当前或目标实现的段落已经被取代，不得作为现状或恢复旧路径的依据。当前 Codex Provider 使用 Codex Desktop 私有 Owner/Follower IPC，阶段一只读、无审批/回复，也没有 IPC 任务目录或 App Server/Hook/transcript fallback。
>
> 当前事实入口：`../30-domains/agent-control/codex-app-server.md`；架构决策：`../50-decisions/codex-desktop-ipc-as-provider-source.md`；故障排查：`../40-runbooks/codex-desktop-ipc-unavailable.md`；长期规约：`../60-rules/codex-desktop-ipc-fail-closed.md`。下文保留用于理解历史问题、标准协议和远程控制方向，出现冲突时以上述当前文档为准。

## 背景

Code Pet 当前是一个面向本机 AI 编程工具的桌面宠物应用。它通过透明悬浮窗口展示 Codex、Claude Code、Qoder 和 Cursor 的任务活动，并提供完成提醒、等待审批提示、轻量回复、打开原会话和 Token 用量统计。项目已经具备 Tauri 2 桌面壳、Svelte UI、Rust 后端、自动启动、自动化构建、签名、更新发布、通知、跨平台配置和桌宠交互等基础能力。

现有 Agent 接入以 Hook 和本地文件为中心：Agent 把 hook payload 发送到 Code Pet 的 localhost collector，Rust 将不完整的 payload 归一化成 `PetEvent`，前端再按 provider、session id 或 cwd 合并成任务卡片。为了补足 Hook 覆盖不足，项目又增加了 Codex audit 回放和监听、Claude transcript 监听、标题解析、transcript 扫描式 Token 统计，以及针对回复、审批和窗口激活的 provider 专属补丁。

这套机制适合早期验证桌宠产品形态，但不适合作为远程控制基础：Hook 不是完整的会话协议，缺少可靠的历史、运行状态、流式 item、模型配置、权限设置、精确用量和多客户端恢复语义；文件监听和 transcript 扫描得到的是滞后且可能不完整的推断数据；审批与回复依赖事件字段是否恰好携带 session id、终端元数据或可等待的 hook 进程。随着功能增加，补丁之间会继续互相影响。

本次改造将 Code Pet 升级为常驻的本机 Runtime Gateway。Code Pet 继续复用现有桌面 App 的构建、发布、更新和系统集成能力，但 Agent 数据源改为各 Provider 的正式 server/SDK 协议。桌宠 UI 和未来手机 App 都通过同一个标准协议访问同一个网关，由电脑端完成实际模型调用、文件访问、命令执行、审批转发和会话持久化。

## 产品目标

- 将 Code Pet 从 Hook 事件观察器升级为本机多 Agent Runtime Gateway。
- 以 Codex 为第一优先级，完整接入 `codex app-server` V2，覆盖会话历史、新建和继续对话、实时 turn/item、模型、推理强度、访问权限、审批、Diff、用量和状态。
- 保留现有桌宠体验：任务运行时显示气泡，需要审批、完成或失败时及时提醒；支持回复、停止、审批和打开对应桌面会话。
- 建立统一的 Code Pet Standard Protocol，使桌宠 Svelte UI 和未来 Flutter 手机 App 面对同一套方法、模型、事件、错误和能力声明。
- 使用统一 IDL 作为协议事实来源，从 JSON Schema 和方法/事件清单自动生成 Rust、TypeScript 和 Dart 代码，避免手写模型漂移。
- 将本地 Tauri IPC 与未来远程连接实现为同一 Gateway 的不同 Transport，使业务行为、权限检查和 Provider 状态只维护一次。
- 保持 Provider 可扩展性。完成 Codex 后，能够在不推翻 Gateway、标准协议和 UI 基础模型的前提下接入 OpenCode Server、Claude Agent SDK/runtime，并为 Qoder 等后端保留扩展空间。
- 所有 Agent 实际执行继续发生在用户电脑上；手机只承担 UI、输入、审批和状态查看。
- 远程链路默认安全：Provider 原生端口不暴露公网，Gateway 负责设备配对、认证、端到端加密、权限过滤和数据脱敏。

## 非目标

- 不实现远程桌面、屏幕镜像或桌面 App 的逐像素 UI 复制。
- 不承诺所有 Provider 的桌面 UI 与手机 UI 在每个内部状态上完全同步；同步程度取决于 Provider 是否允许多客户端附着同一运行时。
- 第一阶段不同时实现 Codex、OpenCode、Claude 和 Qoder。架构保持通用，但实现和验证以 Codex 为先。
- 不在 Code Pet 中复制 Codex App Server 的完整协议模型，也不试图构造覆盖所有 Provider 所有字段的万能领域模型。
- 不让手机直接访问 `codex app-server`、OpenCode HTTP/SSE 或 Claude SDK transport。
- 不以兼容现有 Hook、audit 和 transcript 数据管线为约束。本项目当前用户范围有限，改造允许直接删除旧 Agent 数据源代码。
- 不在本文定义项目排期、人员排班、版本发布日期或商业化部署计划。
- 不在首轮实现云端模型执行；Provider 运行时和代码工作区仍位于电脑。

## 现状理解

> 实施状态（2026-08-26）：Codex 已退出旧 Hook/audit 活动管线。协议 v0 的生成契约与 Runtime Gateway 手写核心已经接入：空 Provider registry、统一 dispatcher、内存事件 sequence/replay 窗口、in-process local transport 和 Tauri command/event bridge 会随应用启动。当前没有注册或伪造 Codex Provider，也没有实现 App Server client；Claude Code、Qoder、Cursor 的旧 Hook 行为及历史 Token 聚合暂时保留。

### 当前运行拓扑

当前主路径是：

```text
Agent Hook
  → src-tauri/hooks/code-pet-hook.mjs
  → http://127.0.0.1:47621/hook
  → activity/collector.rs
  → activity/events.rs::normalize_hook_payload
  → app/state.rs::SharedState
  → Tauri `pet-event`
  → frontend/lib/activity.ts
  → PetApp.svelte 任务卡片
```

补偿路径包括：

```text
Codex audit.jsonl
  → agent/codex_audit.rs 回放和文件监听
  → PetEvent / Token 刷新

Claude transcript
  → agent/claude_transcript.rs 监听结果
  → PetEvent

Token 用量
  → activity/token_usage.rs 扫描 audit 引用和 transcript
  → token_usage_summary Tauri command
```

这些路径对同一事实使用不同采集方式，存在时间顺序、字段完整性、重复事件、状态归并和可信度不一致的问题。

### 当前状态模型

`activity/events.rs` 将 Hook payload 归一化为 `PetEvent`，包含 provider、kind、status、title、message、session id、cwd、tool name、通知标志和部分终端来源信息。`app/state.rs` 在内存中保存最近事件和以 event id 为键的待审批。前端 `activity.ts` 通过 `provider:sessionId`、`provider:cwd` 或全局 fallback 推断任务 identity，再将事件归并成可见任务。

该模型把“采集事件”“会话事实”“一次任务”“审批请求”和“桌宠视图”混在一个对象中。`PetEvent.id` 是随机 UI 事件 ID，却同时参与审批定位；同一会话连续运行多个 turn 时，session id 无法独立标识当前任务；status 是根据 Hook 名称推断，而不是 Provider runtime 的权威状态。

### 当前主动控制

`agent/actions.rs` 定义了激活、回复和审批策略，但接口以 `PetEvent` 为输入。Codex 回复由 `agent/codex_app_server.rs` 完成：每次回复启动一个新的 `codex app-server --listen stdio://`，执行 initialize、`thread/resume` 和 `turn/start` 或 `turn/steer`，等待 `turn/completed` 后销毁进程。该实现无法长期订阅多 thread，无法并发分发完整通知，无法可靠处理 server 发起的审批请求，也不能作为桌宠和手机共享的状态源。

Codex 会话激活目前使用 `codex://threads/<thread-id>` deeplink，而不是 App Server RPC。这个行为应作为独立的桌面激活能力保留。

### 当前模块边界

- `src-tauri/src/activity/`：collector、Hook 事件归一化、标题解析和扫描式 Token 用量。
- `src-tauri/src/agent/`：Agent 注册、Hook 安装、Codex audit、Claude transcript、Codex 临时 App Server client 和事件级交互策略。
- `src-tauri/src/app/state.rs`：内存事件与 Hook 审批等待状态。
- `src-tauri/src/lib.rs`：大量手写 Tauri command、后台 watcher 和 Tauri event 注册。
- `frontend/lib/api.ts`、`frontend/lib/types.ts`：手写 TypeScript IPC 调用和模型。
- `frontend/lib/agentInteractions.ts`：根据 provider 和 `PetEvent` 状态判断前端动作能力。
- `frontend/PetApp.svelte`：消费 `pet-event` 并展示任务卡片。

### 允许的破坏性变化

本次改造不要求保持旧 Agent 接入行为兼容。以下代码可以在新路径具备对应能力后直接删除，而不设置长期双轨模式：

- Agent Hook 注册、配置改写和 hook collector 数据入口；
- Codex audit 回放和 watcher；
- Claude transcript outcome watcher；
- 依赖 audit/transcript 的 Agent Token 主统计路径；
- 以 `PetEvent` 和随机 event id 为核心的审批等待模型；
- 每次回复临时启动 App Server 的实现；
- 前端根据 provider 手写的 `PetEvent` 交互能力判断。

删除顺序仍应保证每个可见行为已有新来源覆盖，避免在一次大改中失去诊断能力；这是一种实施安全措施，不是历史兼容承诺。

## 整体架构

### 分层

系统分为五层：

```text
┌──────────────────────────────────────────────────────────┐
│ 1. UI / Presentation                                     │
│ Code Pet 主窗口、桌宠窗口、未来 Flutter 手机 App          │
└──────────────────────────────┬───────────────────────────┘
                               │ generated client API
┌──────────────────────────────▼───────────────────────────┐
│ 2. Transport / Channel                                   │
│ Tauri IPC / local channel / P2P / encrypted relay        │
└──────────────────────────────┬───────────────────────────┘
                               │ Code Pet wire envelope
┌──────────────────────────────▼───────────────────────────┐
│ 3. Gateway / Protocol Runtime                            │
│ dispatch、event stream、client session、auth、capability │
└──────────────────────────────┬───────────────────────────┘
                               │ generated service API
┌──────────────────────────────▼───────────────────────────┐
│ 4. Application / Standard Domain                         │
│ provider 聚合、会话索引、审批、用量、Activity Projection │
└──────────────────────────────┬───────────────────────────┘
                               │ provider adapter contract
┌──────────────────────────────▼───────────────────────────┐
│ 5. Provider Adapter / Infrastructure                     │
│ Codex App Server / OpenCode Server / Claude Agent SDK    │
└──────────────────────────────────────────────────────────┘
```

调用从 UI 向 Provider 下行，Provider 事件经 Application 和 Gateway 反向上行。任何 UI 都不得绕过 Gateway 直接调用 Provider。

### 推荐目录

在 Tauri 后端创建独立边界：

```text
src-tauri/src/runtime_gateway/
├── mod.rs
├── gateway/
│   ├── dispatcher.rs
│   ├── event_bus.rs
│   ├── client_session.rs
│   └── authorization.rs
├── application/
│   ├── runtime_hub.rs
│   ├── provider_registry.rs
│   ├── conversation_index.rs
│   ├── approval_inbox.rs
│   ├── usage_view.rs
│   └── activity_projection.rs
├── providers/
│   ├── mod.rs
│   └── codex/
│       ├── provider.rs
│       ├── process.rs
│       ├── transport.rs
│       ├── rpc.rs
│       ├── protocol.rs
│       ├── mapper.rs
│       ├── approvals.rs
│       ├── synchronization.rs
│       └── activation.rs
├── transports/
│   ├── mod.rs
│   ├── tauri.rs
│   └── remote.rs
└── store/
    ├── mod.rs
    └── projection_store.rs
```

协议和生成器独立于 Rust 实现：

```text
protocol/
├── manifest.json
├── schemas/
├── methods/
├── events/
├── fixtures/
└── README.md

protocol-codegen/
├── src/
└── tests/

generated/
├── rust/
├── typescript/
└── dart/
```

生成代码是机械产物，不包含业务逻辑，不允许手工编辑。Provider adapter 和 application service 使用生成类型实现接口。

## Code Pet Standard Protocol

### 设计原则

- 协议是桌宠 UI、主窗口、手机 App 与 Runtime Gateway 之间的唯一契约。
- Codex App Server 是概念和能力设计的重要参考，但 Code Pet 协议不复制 Codex 命名和完整 schema。
- 公共模型只表达产品真正跨 Provider 使用的语义；Provider 独有能力通过 capability 和受控扩展字段表达。
- Control Plane 强类型且稳定；Data Plane 可扩展，并为未知 Provider item 保留 fallback。
- 未识别字段不能导致解码失败；未识别的安全敏感命令和审批决定必须拒绝。
- 所有跨端协议都支持版本协商和 capability negotiation。

### IDL

JSON Schema Draft 2020-12 用于定义模型。JSON Schema 本身不描述 RPC 方向、方法与响应配对、事件交付和权限要求，因此增加 Code Pet Protocol Manifest：

```json
{
  "name": "conversation.list",
  "direction": "clientToServer",
  "request": {
    "$ref": "../schemas/conversation/ListConversationsRequest.json"
  },
  "response": {
    "$ref": "../schemas/conversation/ListConversationsResponse.json"
  },
  "idempotency": "safe",
  "requiredCapability": "conversation.list"
}
```

事件定义示例：

```json
{
  "name": "turn.started",
  "direction": "serverToClient",
  "payload": {
    "$ref": "../schemas/turn/TurnStartedEvent.json"
  },
  "delivery": "replayable",
  "scope": "conversation"
}
```

生成器读取 schemas、methods 和 events，生成：

- Rust serde DTO、request/response/event union、method registry、dispatcher 壳和 client trait；
- TypeScript 类型、discriminated union、runtime decoder、typed client 和 event map；
- Dart immutable model、JSON codec、sealed union、typed client 和 event stream。

业务 handler、Provider adapter 和 UI renderer仍然手写。生成器不生成业务决策。

### 跨语言 Schema 约束

- wire 字段统一使用 `camelCase`；
- union 必须有明确的 `kind` discriminator；
- ID、cursor 和幂等键使用 opaque string；
- 时间统一为 Unix epoch milliseconds；
- optional 与 nullable 明确区分；
- enum 支持 unknown/fallback 解码策略；
- 避免无 discriminator 的复杂 `oneOf`、递归 schema 和 `patternProperties`；
- 二进制内容通过资源 ID 或独立资源通道传输，不内嵌大型 JSON；
- 数值范围必须兼容 JavaScript，超出安全整数范围的值使用字符串；
- Provider 扩展字段只允许出现在明确的 extension point，并在发往远程前脱敏。

### Wire Envelope

所有 Transport 使用相同的逻辑 envelope。Tauri IPC 可以在进程内直接传递生成类型，不要求先编码为网络字符串，但语义必须相同。

Request：

```json
{
  "protocolVersion": 1,
  "type": "request",
  "id": "req_01",
  "method": "conversation.list",
  "params": {}
}
```

Response：

```json
{
  "protocolVersion": 1,
  "type": "response",
  "id": "req_01",
  "result": {}
}
```

Event：

```json
{
  "protocolVersion": 1,
  "type": "event",
  "seq": 1042,
  "event": "turn.started",
  "payload": {}
}
```

Error：

```json
{
  "protocolVersion": 1,
  "type": "response",
  "id": "req_01",
  "error": {
    "code": "provider_unavailable",
    "message": "Codex App Server is unavailable",
    "retryable": true,
    "details": null
  }
}
```

远程 Transport 可在 envelope 外增加 connection id、ack 和密文 framing；这些字段不进入业务协议。

### 初始化与能力协商

连接首先调用 `system.initialize`，交换：

- client 名称、版本和平台；
- client 支持的最小/最大协议版本；
- Gateway 选定的协议版本；
- server 名称和版本；
- event replay、资源传输等协议能力；
- Provider 列表、状态和 capability 概要。

Provider capability 通过 `provider.readCapabilities` 动态读取。UI 不写死模型、推理强度和权限枚举，而是按 Provider 返回值构建选择器。旧客户端面对新能力时隐藏未知操作，而不是依赖 App 版本猜测。

### 初始公共方法

```text
system.initialize
system.health

provider.list
provider.readCapabilities
provider.connect
provider.disconnect

project.list

conversation.list
conversation.read
conversation.create
conversation.resume
conversation.fork
conversation.rename
conversation.archive

turn.start
turn.steer
turn.interrupt

approval.list
approval.resolve

model.list
usage.read
```

### 初始公共事件

```text
provider.statusChanged

conversation.created
conversation.updated
conversation.archived
conversation.deleted

turn.started
turn.updated
turn.completed

timeline.itemStarted
timeline.itemUpdated
timeline.itemCompleted

approval.requested
approval.resolved

usage.updated
warning
```

### 公共模型的边界

公共模型不是 Provider schema 的逐字段副本。它只支撑统一 UI 和远程操作：

```text
Provider
Project
ConversationSummary / ConversationDetail
Turn
TimelineItem
Approval
Usage
Capabilities
ProtocolError
```

`ConversationSummary` 包含 provider id、conversation id、项目引用、标题、preview、状态、时间和能力。完整 Provider thread metadata 不全部复制。

`TimelineItem` 使用 discriminated union，初始支持：

```text
text
reasoning
command
fileChange
toolCall
approval
usage
unknown
```

每个 item 有公共 identity、状态和可选的 `providerData`。Codex 新增未知 item 时，adapter 可以生成 `unknown`；旧桌宠可忽略，手机可显示通用卡片，后续再按产品需要增加正式投影。原始 Provider payload 可以在电脑内部保留用于诊断，但不得不经审查地完整发送到手机。

### Control Plane 与 Data Plane

Control Plane 包括 provider、项目、会话、turn 操作、审批、能力、同步和错误。它强类型、版本化并进行权限验证。

Data Plane 包括流式文本、reasoning、工具输出、Diff 和 Provider 实验性 item。它使用稳定 envelope 和可扩展 item union，允许 unknown fallback。该划分避免把每次 Provider 字段扩展升级成整个手机协议的破坏性变更。

## Runtime Gateway 与业务状态

### 已实现的手写核心与生成契约边界

`protocol/schemas/v0.json` 继续是 wire DTO 的唯一事实来源。`runtime_gateway/generated.rs` 只负责 serde DTO、`ProtocolRequest`、`ProtocolResponse`、`ProtocolEvent`、`ProtocolServer` 和机械 dispatcher；手写代码不得复制这些跨边界类型，也不得在生成文件中加入 registry、状态或 Provider 特例。

阶段二的手写职责位于 `src-tauri/src/runtime_gateway/`：

- `provider.rs` 定义 Provider adapter 的异步能力边界，方法参数和结果直接使用生成的 v0 request/response。
- `registry.rs` 按 `providerId` 注册、移除、枚举和路由 adapter，并把未知 Provider 与非 ready Provider 分别映射为标准 `unknown_provider` 和 `provider_unavailable` 错误。
- `gateway.rs` 实现生成的 `ProtocolServer`。握手、Provider 枚举和六个 Provider 操作都经过同一 registry；空 registry 是合法启动状态。
- `event_bus.rs` 接收生成的 `ProtocolEvent`，覆盖其 wire version 和 sequence，分配进程内单调 sequence，并维护有界内存重放窗口。窗口之外的 cursor 返回 `event_replay_unavailable`，由客户端重新获取快照；该窗口不是持久化会话存储。
- `transport.rs` 的 `Transport` contract 将 request/response dispatch 与 event subscribe/replay 分开；`LocalTransport` 是直接调用同一 Gateway 的 in-process 实现，不包含 WebSocket、P2P、认证或远程加密。
- `tauri_bridge.rs` 只在 Tauri 边界传递生成的 `ProtocolRequest`、`ProtocolResponse`、`ProtocolEvent` 和 `ProtocolError`。`runtime_gateway_request` 负责 JSON IPC request，`runtime_gateway_replay` 补回窗口内事件，实时事件统一通过 `runtime-gateway-event` 发出；Provider 原生 `Value` 不得穿过该边界。

本地调用链为：

```text
generated TypeScript wire request
  → Tauri runtime_gateway_request
  → LocalTransport
  → generated dispatch
  → Gateway / ProviderRegistry
  → ProviderAdapter

Provider ProtocolEvent
  → GatewayEventBus（重写 eventSequence）
  → LocalTransport subscription
  → Tauri runtime-gateway-event
  → generated TypeScript ProtocolEvent
```

这条本地通道与未来远程 Transport 共享 Gateway 业务语义，但不预先引入远程 framing。Provider adapter 可以持有 Gateway 提供的 `ProviderEventSink` 上报标准事件；Gateway 不读取或转发 Provider 原生 payload。

### Runtime Hub

`RuntimeHub` 是桌宠、主窗口和手机的唯一业务入口，持有：

```text
ProviderRegistry
ConversationIndex
ApprovalInbox
UsageView
ActivityProjection
GatewayEventBus
ProjectionStore
```

本地和远程 Transport 都调用同一个 dispatcher：

```rust
async fn dispatch(
    context: ClientContext,
    request: ProtocolRequest,
) -> ProtocolResponse
```

本地 Tauri command 负责把生成 request 交给 dispatcher；远程连接负责解码 frame 后调用同一 dispatcher。事件总线同时挂接 `TauriEventSink` 和 `RemoteEventSink`。

### Provider 是事实来源

Provider runtime 和其持久化历史是会话事实来源。Code Pet 不复制完整 Provider 会话数据库。Gateway 保存的是可以重建的索引和 UI projection：

```text
provider instance 状态
conversation summary/index
当前 active turn
待审批
usage snapshot
event cursor / sequence
paired devices
remote delivery ack
```

如果引入 SQLite，它是可丢弃的 projection cache，不是 Provider transcript 的替代品。数据库损坏后应能通过 Provider list/read/resume 重建核心状态。

### Activity Projection

桌宠不直接消费 Provider event，也不直接消费完整 conversation。Application 层把权威状态投影成紧凑卡片：

```text
providerId
conversationId
turnId
title
project
status
latestAction
pendingApproval
usage
capabilities
```

任务 identity 使用 `providerId + conversationId + turnId`。一次 conversation 可以有多个连续 turn，不能再只按 session id 合并。

事件与桌宠状态的基础映射：

| 标准事件 | 桌宠行为 |
| --- | --- |
| `conversation.created` | 更新会话索引，通常不单独显示运行气泡 |
| `turn.started` | 显示“正在运行”任务气泡 |
| `timeline.itemStarted(command)` | 显示正在执行的命令摘要 |
| `timeline.itemStarted(fileChange)` | 显示正在修改文件 |
| `approval.requested` | 切换为等待审批并按设置提醒 |
| `approval.resolved` | 清除审批状态并恢复运行或空闲状态 |
| `turn.completed(completed)` | 显示完成 |
| `turn.completed(failed)` | 显示失败 |
| `turn.completed(interrupted)` | 显示已停止 |
| `usage.updated` | 更新卡片和用量页 |

`thread/started` 或 `conversation.created` 只表示会话存在，不表示任务已经运行；运行气泡以 turn 状态为准。

### Approval Inbox

审批以 Provider request id 和标准 approval id 关联，不再使用随机 UI event id。标准审批包含：

- provider、conversation、turn 和 item identity；
- command、file change、permission、MCP elicitation 或 user input 类型；
- 安全摘要和可展示内容；
- Provider 实际允许的 decision 列表；
- pending、resolved、expired、unknown 状态；
- 必要且经过过滤的 Provider 扩展数据。

手机调用 `approval.resolve` 时只提交标准 decision id 和被 capability 允许的参数。Gateway 校验设备权限和当前审批状态，再由 adapter 转换为 Codex `accept`、`acceptForSession`、policy amendment 等原生响应。手机不得发送任意 Provider JSON-RPC response。

### 用量

用量分为：

- Conversation usage：本轮、累计、缓存输入、reasoning 输出和 context window；
- Account usage：额度窗口、已用比例、重置时间和 spend/credit 状态；
- Provider-specific usage：Qoder credits、OpenCode provider usage 等扩展。

Codex 以 `thread/tokenUsage/updated`、resume 恢复事件、`account/usage/read` 和 `account/rateLimits/read` 为来源，不再扫描 transcript 计算主数据。不同含义的用量不能合并成一个数字。

## Codex Provider

### 接入点

Codex 使用官方 `codex app-server` V2。App Server 支持 thread、turn、item、模型、配置、账户、用量和双向审批。网络 WebSocket transport 仍具有实验性质，因此 Code Pet 在本机优先使用 stdio 或受控 Unix socket；App Server 不监听公网地址。

### 生命周期

Code Pet 启动时创建一个长期 Codex Provider：

```text
定位 Codex binary
  → 读取版本
  → 启动 codex app-server --listen stdio://
  → initialize / initialized
  → account/read
  → model/list
  → thread/list
  → thread/loaded/list
  → 持续处理 response、notification 和 server request
```

Code Pet 生命周期内不为每条消息、每个客户端或每个 conversation 单独启动 App Server。

内部至少有：

- writer task：串行写 JSONL；
- reader task：持续解析 RPC response、notification 和 server request；
- stderr task：持续排空并记录诊断，避免管道阻塞；
- pending request map：`requestId → oneshot response`；
- subscription/event dispatcher：按 thread/turn identity 路由事件；
- process supervisor：健康检查、退出检测和重连；
- reconciliation：重连后通过 list/read/resume 修复状态。

### 会话能力

Codex 第一阶段覆盖：

- `thread/list`：设备全部会话、cwd 过滤、分页、搜索和排序；
- `thread/read`、`thread/turns/list`、`thread/items/list`：历史加载；
- `thread/start`、`thread/resume`、`thread/fork`：创建、继续和分叉；
- `thread/name/*`、archive/delete 生命周期；
- `turn/start`、`turn/steer`、`turn/interrupt`；
- `turn/started`、`turn/completed`、`item/*` 和 `turn/diff/updated`；
- `model/list` 和模型支持的 reasoning efforts、service tiers；
- sandbox、approval policy 和版本支持时的 permission profile；
- command、file、permission、user input 和 MCP approval；
- thread token usage、account usage 和 rate limits。

模型和权限值来自 runtime capability，不写死在 Code Pet schema enum 中。

### 项目和普通聊天

Codex 的 thread 统一以 cwd 记录工作目录。Code Pet 使用规范化绝对路径和本机项目索引将会话归入项目；未命中已知项目的 cwd 显示在“普通聊天/其他会话”，而不是假定 Provider 存在独立的普通聊天类型。worktree 路径可以映射到同一逻辑项目。

### 桌面激活

打开 Codex 桌面会话继续使用 `codex://threads/<thread-id>` deeplink。激活属于 Provider 的 Desktop Activation capability，与 App Server 消息传输分离。其他 Provider 可以实现 URL、终端 session、bundle activation 或受控的 Accessibility fallback。

### 跨进程限制

必须单独验证 Code Pet 启动的 App Server 是否能实时观察独立 Codex Desktop App 进程创建的 turn、item 和审批。已知可以通过 `thread/list` 读取持久化会话，但不能仅凭协议文档假设不同 App Server 进程之间广播实时通知。

需要区分：

- Managed conversation：由 Code Pet App Server 创建或恢复，Code Pet 拥有完整实时流和审批通道；
- External conversation：由独立 Codex Desktop/CLI runtime 创建，可能只能通过 list/read 很快发现，实时 delta 和审批能力以实验结果为准。

本项目不为此保留旧 Hook 架构。如果 External conversation 无法完全附着，应在 capability 和 UI 中明确限制，而不是继续用推断数据伪装成权威实时状态。

### 协议升级

Codex runtime 可以生成版本匹配的 TypeScript 和 JSON Schema。CI 使用生成 schema 做契约对比和 fixture 验证。Rust runtime 不必生成 App Server 的全部类型；它强类型化 JSON-RPC envelope、核心 thread/turn/status/usage/approval，并对其他 item 按 `type` 解析必要字段，同时保留未知 payload。

Codex 增加无关字段时不要求 Code Pet 更新。只有调用参数变化、安全相关审批变化、已有字段语义破坏或产品决定展示新 item 时才更新 adapter。

## 后续 Provider

### OpenCode

OpenCode 是第二优先验证 adapter。`opencode serve` 提供 headless HTTP、OpenAPI 3.1 和 SSE，支持 project、session、message、diff、fork、abort、permission 和 session/message 事件。Provider 优先附着同一个 OpenCode Server，使桌面 UI 和 Code Pet 共享 session；未运行时可由 Code Pet 启动受控 headless server。OpenCode 原生端口仅绑定 loopback，由 Gateway 对外提供能力。

OpenCode adapter 用于验证 Standard Protocol 没有被实现成只换名称的 Codex 协议。

### Claude

Claude 第一阶段指 Claude Code/Claude Agent SDK 本地运行时，不泛指普通 Claude Desktop 云聊天。Adapter 负责本地 session list/messages、resume/fork、stream、interrupt、usage 和 `canUseTool` approval callback。若官方 SDK 更适合 TypeScript/Python，可由 Code Pet 管理一个受控 sidecar，但 sidecar 对 Rust Runtime Hub 只暴露标准 Provider contract。

Claude Agent SDK 能力不等同于附着 Claude Desktop 当前 UI；桌面同步程度应通过 capability 和实际验证表达。

### Qoder 与其他 Provider

Qoder Agent SDK 已提供 TypeScript/Python、多轮 session、权限、usage、checkpoint 和自定义 transport，可按相同 adapter 边界接入。Cursor 等缺少完整公开 runtime 协议的产品不在首轮范围；是否重新支持取决于可验证的开放能力，而不是恢复 Hook 推断。

## Transport

### 本地通道

Code Pet 主窗口和桌宠窗口继续使用 Tauri IPC/event，但不再手写一组独立业务 API。生成的 TypeScript client 通过 `TauriTransport` 调用 Gateway dispatcher，事件由 `TauriEventSink` 发送。

本地 UI 与手机使用相同 method、request、response、event 和 error 类型；差异只存在于连接、序列化和认证层。

### 远程通道

远程目标是手机与电脑建立双向连接：

1. 同一局域网优先直接连接；
2. 跨网络优先尝试 P2P；
3. P2P 失败后使用自托管 TURN/WSS relay；
4. relay 只转发端到端密文；
5. 手机断线不终止电脑上的 Provider turn；
6. 重连通过 snapshot、单调 sequence 和 ack 补齐状态；
7. `clientMessageId`/幂等键防止 `turn.start`、消息和审批重复执行。

纯 P2P 不作为成功率承诺。蜂窝网络、CGNAT、对称 NAT、企业防火墙和移动系统后台限制都可能要求 relay。控制数据主要是文本、事件和 Diff，稳定性优先于避免少量中继带宽。

### 远程同步

手机连接后先获取：

```text
Provider 状态和能力
项目索引
最近会话摘要
当前 active turn
待审批
用量摘要
当前事件 sequence
```

用户打开会话后再分页读取 turns/items。实时事件携带 sequence；重连客户端提交最后 ack，Gateway 在可用窗口内重放，否则返回新 snapshot。Transport 重试不能自动重放非幂等业务 request。

## 安全

- Codex App Server、OpenCode Server 和 SDK sidecar 只监听 loopback、stdio 或受控本机 socket。
- Code Pet 首次启动生成长期设备身份密钥；手机通过短期二维码或等价流程完成相互认证。
- 应用 payload 使用端到端加密；relay 不读取 prompt、response、command、diff 或路径。
- 手机密钥保存于 Keychain/Keystore，电脑密钥保存于系统安全存储；支持设备查看和撤销。
- 设备权限至少区分只读、可发消息、可审批、可修改会话设置和管理员。
- 手机不能自行把 sandbox 或 approval policy 提升到电脑未授权的级别。
- `approval.resolve` 必须验证设备权限、approval 当前状态和 Provider advertised decisions。
- Provider 原始 payload 发往远程前移除凭据、环境变量、attestation、无关绝对路径和内部调试信息。
- 文件读取和下载必须进行真实路径解析、workspace allowlist、大小限制和敏感文件规则校验。
- push 通知只包含 opaque device/event id，不包含任务正文。
- 日志记录 method、状态、耗时、ID 和 payload 大小；默认不记录 prompt、回复、命令正文和 Diff。

## 实施路径

以下阶段描述依赖顺序和完成条件，不表示时间排期。每个阶段完成后保持代码库可运行，并删除已被替代的旧实现。

### 阶段一：协议与生成基础

- 创建 `protocol/`、manifest、JSON Schema 跨语言子集和 protocol version。
- 定义 initialize、provider、conversation、turn、timeline、approval、usage、capability 和 error 的第一版模型。
- 定义第一批 methods/events。
- 实现最小 codegen，生成 Rust 和 TypeScript；Dart 生成可以在手机项目启动前补齐，但 IDL 约束从第一版就需兼容 Dart。
- 生成 dispatcher/client 壳和 runtime decoder。
- 建立 fixture 与 round-trip 测试，确保 Rust/TS 对同一 JSON 的行为一致。
- 让现有 Tauri UI 通过生成 client 调用一个最小 `system.health`，验证本地通道闭环。

完成条件：IDL 是协议唯一事实来源；手写业务代码不再重复声明相同 wire DTO。

### 阶段二：Runtime Gateway 壳与状态边界

- 创建 `runtime_gateway/`，实现 dispatcher、event bus、client context 和 ProviderRegistry。
- 实现 ConversationIndex、ApprovalInbox、UsageView 和 ActivityProjection 的接口与内存 store。
- 将 Tauri IPC/event 适配到 Gateway，但暂不要求所有旧 UI 行为切换。
- 明确 Provider 状态、错误和 capability 的公共语义。
- 建立 event sequence 和 snapshot 结构，为远程重连预留，但暂不实现公网 Transport。

完成条件：本地 UI 和未来 Remote Transport 能共享同一 dispatch/event 接口。

### 阶段三：Codex 长期连接与协议核心

- 替换一次性 `codex_app_server.rs` client，建立长期进程 supervisor。
- 实现 initialize、typed request map、notification 路由、server request 路由、stderr drain 和重连。
- 实现 runtime 版本探测、binary 查找、健康状态和协议不兼容错误。
- 实现 thread list/read/resume、turn/item 事件和状态 reconciliation。
- 完成独立 Codex Desktop 跨进程可见性 Spike，记录 Managed/External conversation 能力边界。

完成条件：Code Pet 可以在不依赖 Hook/audit 的情况下列出和读取 Codex 会话，并持续接收其管理会话的运行事件。

### 阶段四：Codex 完整业务能力

- 实现会话创建、继续、fork、重命名、归档和分页历史。
- 实现消息发送、steer、interrupt 和 client message 幂等。
- 实现 command、file、permission、user input 和 MCP approval。
- 实现 model、reasoning effort、service tier、sandbox/permission 和 approval policy capability。
- 实现 thread usage、account usage 和 rate limits。
- 实现 Codex timeline item 到标准 item 的按需投影和 unknown fallback。
- 保留 `codex://threads/<id>` 桌面激活能力。

完成条件：标准协议覆盖桌宠和手机 MVP 所需的 Codex 操作，不需要读取 Hook、audit 或 transcript 才能完成核心流程。

### 阶段五：桌宠迁移与旧管线清理

- 将 `PetApp.svelte` 和主窗口事件源切换为生成 client 与 Activity Projection。
- 保留现有宠物动画、气泡、音效、通知和布局体验。
- 将回复、停止、审批和激活改为标准 Gateway method。
- 用 `providerId + conversationId + turnId` 替代旧 activity identity。
- 删除 Hook 安装、collector Agent 入口、Codex audit watcher、Claude transcript watcher、扫描式主 Token 数据源和旧 `PetEvent` 审批路径。
- 删除前端手写 provider 交互能力判断，改为消费 capability。
- 更新设置页：从“启用 Hook/选择 Hook 事件”改为 Provider 连接、状态和能力诊断。

完成条件：Codex 桌宠功能完全由 App Server 和 Runtime Gateway 驱动，代码中不存在为了历史兼容保留的旧 Agent 数据源。

### 阶段六：远程 Transport 与设备模型

- 定义并实现设备身份、配对、撤销、连接授权和设备权限。
- 实现 Remote Transport 的 frame、E2EE、心跳、sequence、ack、snapshot 和重放。
- 先完成 LAN/WSS relay 闭环，再加入 direct-first P2P；Transport 可替换但业务协议不变。
- 实现远程 client session、并发客户端、速率限制和非幂等 request 去重。
- 为 Dart 生成并发布 Code Pet Protocol package。

完成条件：独立测试客户端可以远程列出 Codex 会话、读取历史、发送消息、查看流式状态、停止任务并处理审批。

### 阶段七：手机 App

- 使用生成 Dart client 完成设备、Provider、项目、会话、timeline、审批和用量 UI。
- 依据 capability 动态显示模型、推理强度、权限和操作，不写死 Codex 枚举。
- 实现前后台恢复、网络切换、离线状态和 push 唤醒。
- 验证手机断线不会终止电脑任务，重连不会重复执行消息或审批。

完成条件：手机提供接近电脑端的 Codex 会话控制体验，实际执行仍在电脑。

### 阶段八：Provider 扩展验证

- 优先实现 OpenCode Server adapter，验证 REST/SSE Provider 能映射到现有标准协议。
- 再实现 Claude Agent SDK/runtime adapter，验证 sidecar、callback approval 和本地 session 模型。
- 仅在真实差异无法通过 capability 或 extension 表达时扩展标准协议；不得把 Provider 分支散落到 UI。
- 根据开放能力评估 Qoder 和其他 Provider。

完成条件：新增 Provider 主要修改其 adapter、capability 和必要 renderer，不要求推翻 Gateway、Transport 或公共控制协议。

## 涉及模块

- `protocol/`、`protocol-codegen/`、`generated/`：新增协议事实来源、生成器和跨语言产物。
- `src-tauri/src/runtime_gateway/`：新增 Gateway、Application、Provider、Transport 和 projection store 边界。
- `src-tauri/src/agent/codex_app_server.rs`：现有一次性 client 被长期 Codex Provider 替代后删除。
- `src-tauri/src/activity/collector.rs`：Agent Hook 数据入口删除；若其他本地非 Agent 功能仍需 HTTP 服务，应拆出独立用途后保留。
- `src-tauri/src/activity/events.rs`：`PetEvent` 主模型被标准 protocol 和 Activity Projection 替代。
- `src-tauri/src/activity/token_usage.rs`：扫描式 Agent 用量主路径被 Provider usage 替代；仅保留仍有独立产品价值的非 Provider统计时才拆分保留。
- `src-tauri/src/agent/hooks.rs`、`control.rs`、`registry.rs`：Hook 配置模型和开关 UI 删除，Provider registry 迁入 Runtime Gateway。
- `src-tauri/src/agent/codex_audit.rs`、`claude_transcript.rs`：旧补偿 watcher 删除。
- `src-tauri/src/agent/actions.rs`：事件级 driver 被标准 Gateway commands、Provider adapter 和 DesktopActivator 替代。
- `src-tauri/src/app/state.rs`：内存 `PetEvent`/Hook approval 状态被 Runtime Hub store 替代。
- `src-tauri/src/lib.rs`：手写 Agent Tauri commands、watcher 启动和 `pet-event` 注册迁移到生成 dispatcher 与 event sink。
- `frontend/lib/api.ts`、`types.ts`：Agent 相关手写 wire 类型和调用替换为生成 TypeScript client。
- `frontend/lib/agentInteractions.ts`：手写 provider 分支替换为标准 capability。
- `frontend/lib/activity.ts`：保留纯展示辅助时改为消费 `ActivityProjection`；删除从低层事件猜测任务状态的职责。
- `frontend/App.svelte`、`PetApp.svelte`：保持视觉层，切换数据与操作入口。
- `knowledge/`：随实现更新运行拓扑、事件管线、Agent 控制、Provider 风险和验证 runbook。

## 风险

- 风险：独立 Codex Desktop 与 Code Pet App Server 不共享实时事件。缓解与验证：完成跨进程 Spike；按 Managed/External capability 表达真实支持，不用文件推断冒充实时协议。
- 风险：Codex App Server V2 和部分 transport/API仍在演进。缓解与验证：固定支持版本范围，生成当前 runtime schema，维护协议 fixture，CI 覆盖最低支持、推荐和最新版本。
- 风险：自定义标准协议过度复制 Codex，导致其他 Provider 难以接入。缓解与验证：OpenCode 作为第二 adapter 进行架构验收；公共模型只包含产品公共语义，Provider 差异放 capability/extension。
- 风险：公共模型过薄，手机被迫解析原生 Provider payload。缓解与验证：手机 MVP 的每个页面只使用标准 DTO；`providerData` 只能用于增强展示，不能成为核心流程依赖。
- 风险：公共模型过厚，Provider 新字段导致频繁全端升级。缓解与验证：未知字段容忍、unknown timeline item、按需投影和 schema 兼容性测试。
- 风险：Rust、TypeScript 和 Dart 生成器产生不同的 optional、nullable、enum 或整数行为。缓解与验证：限定 JSON Schema 子集，使用共享 JSON fixtures 做三语言 round-trip 与负例验证。
- 风险：长期 App Server reader 在慢消费者、并发 RPC 或大量 delta 下阻塞。缓解与验证：有界 channel、按 turn 路由、背压错误、stderr drain、并发 turn 压力测试和慢客户端测试。
- 风险：App Server 重启后出现重复 turn、重复消息或丢失审批。缓解与验证：业务幂等键、event sequence、Provider reconciliation、故障注入和重连 E2E。
- 风险：多个本地/远程客户端同时处理同一审批。缓解与验证：ApprovalInbox 原子状态转换；首个成功 response 为准，其余客户端收到 `approval.resolved`。
- 风险：远程控制扩大为远程代码执行入口。缓解与验证：设备权限、E2EE、loopback Provider、审批 decision allowlist、路径校验、密钥撤销和独立安全审计。
- 风险：删除旧 Hook 后某些外部 Provider 会话不再实时显示。缓解与验证：删除前按目标场景建立验收矩阵；不满足的场景明确标记为非支持，而不是恢复不可靠推断。
- 风险：Tauri App UI 关闭或窗口隐藏导致 Gateway 生命周期异常。缓解与验证：Gateway 生命周期绑定后台进程/托盘而非窗口，测试关闭主窗口、隐藏桌宠、系统睡眠和登录启动。
- 风险：移动系统后台冻结长连接。缓解与验证：push 只做唤醒、前台恢复 snapshot、断线不终止电脑 turn，并在 iOS/Android 真机验证。
- 风险：中国大陆网络下 P2P 或国外 relay 不稳定。缓解与验证：标准 443、自托管 signaling/relay、多节点选择、P2P 与 relay 并行拨号，以及 Wi-Fi/4G/5G/CGNAT/丢包测试。

## 测试计划

### IDL 与生成代码

- 验证所有 `$ref` 可解析，schema 符合受支持子集。
- Rust、TypeScript、Dart 对共享 fixture 执行 decode/encode round-trip。
- 覆盖 optional、nullable、unknown enum、unknown timeline item、int64 和错误响应。
- CI 重新生成代码并检查工作区无差异，禁止提交过期生成物。
- 检查 method/event 名称唯一，request/response `$ref` 存在，capability 引用有效。

### Gateway

- dispatcher 对每个 method 路由到正确 handler。
- 本地与远程 Transport 对相同 request 返回语义一致的 response/error。
- 未授权设备不能调用写入、审批或权限升级方法。
- event sequence、ack、重放窗口和 snapshot fallback 正确。
- 多客户端订阅、慢客户端和断开清理不会阻塞 Provider reader。

### Codex Provider

- initialize 顺序、重复 initialize、进程退出和 stderr 处理。
- 并发 RPC response 按 request id 正确分发。
- thread list/read/resume/create/fork 和分页历史。
- turn start/steer/interrupt 与文本 delta 顺序。
- command、file、permission、user input 和 MCP approval 全流程。
- `serverRequest/resolved` 清理待审批。
- token usage 恢复、实时更新、account usage 和 rate limits 分开展示。
- 未知 notification/item 不导致连接退出。
- `-32001` overload 使用指数退避和 jitter，非幂等请求不盲目重试。
- App Server 异常退出、Code Pet 重启、系统睡眠和网络恢复后的 reconciliation。
- 独立 Codex Desktop 创建、运行、审批、完成和归档会话的跨进程实验。

### 桌宠

- `turn.started` 生成运行气泡，单独 `conversation.created` 不误报运行。
- 同一 conversation 的多个 turn 不错误合并。
- 多 conversation 并发时标题、工具、状态和审批不串线。
- approval resolved 后按钮消失且状态正确恢复。
- 完成、失败、停止的动画和通知行为保持一致。
- 回复、steer、stop、审批和 deeplink 激活按 capability 显示。
- 现有布局、焦点、点击穿透、拖动、缩放、声音和多显示器行为不回归。

### 远程与手机

- LAN、跨网络 relay、P2P 成功和 P2P 失败回落。
- 手机断网、电脑换网、手机前后台和系统睡眠。
- 消息发送结果丢失后重连，不重复创建 turn。
- 两台手机同时查看和竞争审批。
- relay 无法读取应用 payload；日志不包含 prompt、command 和 diff。
- 大会话分页、长 Diff、流式高频 delta 和图片/文件资源。
- 设备撤销后现有连接被关闭且不能重新连接。

### 删除旧代码

- `rg` 检查 Agent Hook、Codex audit watcher、Claude transcript watcher 和旧 `pet-event` 主路径不再被生产代码引用。
- 更新或删除对应测试、知识文档和设置迁移代码。
- 运行前端 Vitest、Rust 单元/集成测试和 Tauri 构建检查。
- 不运行未经指示的格式化工具。

## 知识沉淀

- 本文作为 Runtime Gateway 与远程控制的主功能设计文档，随协议和实施边界变化更新。
- `knowledge/10-architecture/runtime-topology.md` 在 Gateway 接管启动流程后更新为新拓扑。
- `knowledge/10-architecture/event-pipeline.md` 在桌宠迁移后更新为 Provider → Runtime Hub → Projection → UI。
- `knowledge/10-architecture/frontend-backend-boundary.md` 补充生成 client 与 TauriTransport 边界。
- `knowledge/10-architecture/agent-control.md` 改写为标准 Gateway command、approval 和 DesktopActivator。
- `knowledge/30-domains/agent-control/codex-app-server.md` 更新长期连接、协议版本和 Managed/External conversation 结论。
- 为 Codex 连接失败、协议不兼容、重连、审批卡住和远程设备故障分别补充 runbook。
- IDL 跨语言规则、未知字段兼容和生成代码不可手改应沉淀到 `knowledge/60-rules/`。
- Hook、audit 和 transcript 文档在代码删除时同步删除或改为历史决策说明，避免知识库继续描述不存在的主路径。

## 未知项

- Code Pet 独立 App Server 对 Codex Desktop 进程内实时 thread/turn/item/approval 的可见程度尚未实测。
- Codex Desktop 是否存在适合第三方稳定附着的共享 daemon/control socket，以及其长期兼容承诺尚未确认。
- Code Pet Standard Protocol 的 schema 生成器采用自研最小生成器、组合现有工具，还是以 OpenRPC 为输入层，尚需用 Rust/TS/Dart 小型 Spike 比较。
- Flutter 手机项目的最终状态管理、UI 组件和发布方式尚未确定，不影响 Dart protocol package 设计。
- 远程首版采用 WSS relay 先行还是同时实现 WebRTC，需要在 Transport Spike 中根据中国大陆实测决定；业务协议不依赖该选择。
- projection cache 是否在 Codex 阶段立即使用 SQLite，还是先用可重建内存 store，取决于手机断线重放和 App 重启恢复的最小需求。
- Claude Agent SDK sidecar 的语言、进程托管和与现有 Claude Code UI 的 session 共享程度尚未验证。
- OpenCode 当前运行实例的可靠发现方式和同一 server 多客户端行为需要在接入阶段确认。
- 产品最终是否继续支持 Cursor，取决于其是否提供足够完整、稳定且可验证的运行时接口。
