# Runtime Gateway 与远程控制架构

> 文档状态（2026-08-31）：长期远程设计仍保留。当前 Codex 使用隔离双链路：`CodexRemote` 已迁到进程外 `codepet-provider-codex`，经 Provider Protocol v1、Plugin Manager 与内部 Gateway v2 service 提供远程能力；`CodexDesktopCompanion / IPC` 只驱动桌宠本地投影和面向 Desktop owner 的安全动作。Claude 已有能力较小的独立 `codepet-provider-claude`，只接官方 CLI stream-json，不提供全局会话 CRUD、steer 或审批；OpenCode 只通过独立 `codepet-provider-opencode` 加入同一个 remote Provider Gateway，不存在 companion/Pet 支路。compat `RuntimeGatewayState` 与 LAN listener 复用同一个 Provider registry、event bus 和 Gateway；Tauri 后端已接入 TLS listener、mDNS、pairing/credential 与有界生命周期，frontend UI 尚未接入。Remote 与 companion/Pet 链路不得共享 session、owner/revision 或 unavailable 状态，也不得把 Remote event 注入 Desktop/Pet/activity IPC。
>
> 当前事实入口：Codex 插件边界与协议矩阵见 `codex-provider-plugin-runtime.md`，Claude 见 `claude-provider-plugin-runtime.md`，OpenCode 见 `opencode-provider-plugin-runtime.md`，remote 领域见 `../30-domains/agent-control/codex-app-server.md`，companion 见 `../30-domains/agent-control/codex-desktop-companion.md`，Provider Host 见 `provider-host-device-and-plugin-runtime.md`；协议现状见 `protocol-layers-and-device-routing.md`、`../../protocol/provider/v1/manifest.json` 和 `../../protocol/gateway/v2/manifest.json`。下文的阶段规划和完整能力清单仍包含未实现的长期目标；旧目录、统一 wire envelope、方法名或已生成 Dart 的描述均视为 superseded，不是当前实现证据。

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
- 使用统一 IDL 作为协议事实来源。当前生成 Rust SDK、TypeScript Gateway/compatibility SDK 与 Dart core/Gateway SDK；未来 Python 必须通过显式 target adapter 接入，在实现前选择即 fail closed。
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

## 当前 remote 与 companion 隔离边界

```text
Codex remote client
  → runtime_gateway_request / replay / runtime-gateway-event
  → RuntimeGatewayState（compat 薄适配）
  → ProviderGatewayService（remote bus/sequence/replay）
  → PluginManager / codepet-provider-codex
  → 官方 Codex App Server stdio wire

PetApp
  → codex_desktop_companion_snapshot / request / replay
  → codex-desktop-companion-event
  → CodexDesktopCompanionState（companion registry/bus/sequence/transport）
  → Desktop IPC adapter
  → ~/.codex/ipc/ipc.sock → Desktop owner

```

Codex Provider 实现 Provider v1 的完整 lifecycle、`conversation.list/get/create`、turn start/steer/interrupt、`approval.resolve` 和通知映射。Desktop companion 不广告 list/create，只把已 bootstrap 的本地 thread 投影到桌宠；回复、停止和审批继续使用 generation、owner、revision、request 与 handler 校验。

Provider Host/Gateway 与 companion state 各自持有 registry、event bus、replay window 和 session。compat state 引用 Host 的同一个 Gateway service，但不拥有进程或第二个 remote bus。桌宠前端只引用 companion client/event，Provider conversation/turn/approval 不能进入 companion transport、thread scope/event 或 Pet projection。remote 与 Desktop 的同名 native thread 各自保留，不做跨链路排除或同步。Hook、audit、transcript 和文件监听不参与 Codex 桌宠数据。

OpenCode Server 通过 `codepet-provider-opencode` 的 HTTP/SSE adapter 进入同一条 Provider remote bus；它不注册 Desktop companion adapter，也不向 `SharedState`、`pet-event` 或 `codex-desktop-companion-event` 发布数据。能力与版本边界见 `opencode-provider-plugin-runtime.md`。

当前 Tauri 仍只有进程内 Gateway 调用面；Host crate 已有真实 TLS/WSS listener，但 App 尚未启动或发布它，也没有 P2P/relay。`RemoteAccessManager`、listener transport 与 `ProviderGatewayService` 是分层边界：manager 持有 TLS/pairing/credential，listener 负责 Authorization、socket clientId 与撤销取消，Gateway service 不感知 bearer 或 TLS。App Server 只在 Provider instance start 中初始化且有超时；它 unavailable 不阻塞 Desktop companion，Desktop socket unavailable 也不改变 remote Provider。

## 历史基线（非现状）

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

### 目录状态

下列 Tauri 分解是未来 Runtime Gateway application/remote session 的目标形态，并非当前目录清单；当前实现仍位于 `src-tauri/src/runtime_gateway/` 的扁平模块中，不应为了匹配此图一次性搬迁：

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

协议与生成器已经按以下目录落地；此前本文中的 `protocol/manifest.json`、`protocol/schemas/`、仓库根 `protocol-codegen/` 和 `generated/` 方案已 superseded：

```text
protocol/
├── codegen.json
├── core/v1/{schema.json,manifest.json}
├── pet/v1/{schema.json,manifest.json,fixtures/}
├── provider/v1/{schema.json,manifest.json,fixtures/}
├── gateway/v2/{schema.json,manifest.json,fixtures/}
├── desktop/v0/{schema.json,manifest.json}
└── README.md

tools/protocol-codegen/
├── generate.mjs
├── test.mjs
└── README.md

sdk/rust/codepet-{core,pet,provider,gateway}-sdk/
sdk/typescript/codepet-{core,gateway}-sdk/
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

JSON Schema Draft 2020-12 定义 DTO，分层 manifest 定义 RPC 方向、方法与响应配对、事件交付、transport 和 capability。当前事实来源是：

- `../../protocol/core/v1/manifest.json`：安全共享类型，无 methods/events。
- `../../protocol/pet/v1/manifest.json`：Desktop Companion 驱动的 Pet 方法和事件。
- `../../protocol/provider/v1/manifest.json`：Host ↔ Provider binary 的 JSON-RPC/stdio 方法、事件、生命周期和 capability contract。
- `../../protocol/gateway/v2/manifest.json`：Host ↔ Remote Client 的设备/实例路由与 replayable event。
- `../../protocol/codegen.json`：包依赖、target 状态与输出路径。

manifest 中的方法条目直接引用同层 schema，并以 `capability` 映射到 manifest 声明的 typed capability enum/container：

```json
{
  "name": "conversation.list",
  "direction": "hostToPlugin",
  "idempotency": "safe",
  "capability": "conversation.list",
  "request": { "$ref": "./schema.json#/$defs/ConversationListRequest" },
  "response": { "$ref": "./schema.json#/$defs/ConversationListResponse" }
}
```

生成器读取所有 package schema/manifest，并统一审计两者中的 `$ref`：

- Rust serde DTO、request/response/event union、typed capability/method mapping、dispatcher、client/transport 和 codec；
- TypeScript core 与 desktop-v0 类型、discriminated union、typed client 和 event map；
- Dart 当前从 normalized IR 生成 core/Gateway null-safe package、strict codec、manifest metadata 与 typed client；Python 只有 planned target adapter entry，显式选择会在写文件前失败。

Provider Rust SDK 还生成有界 JSON-line framing、统一 request/response/notification/event classifier、标准 JSON-RPC error mapping、inbound transport 接口与 typed request-to-wire 构造器。业务 handler、进程 supervisor、Provider manager 和 UI renderer 仍属于各自运行时。

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

此前“所有 Transport 共用一个带 `type` 的 envelope”草案已 superseded。当前协议按边界使用两种明确 transport：Pet/Gateway 使用 CodePet envelope；Provider plugin 使用 JSON-RPC 2.0/stdio-json-lines。实际 discriminator 和字段以各层 manifest 为准。

Gateway v2 request：

```json
{
  "protocolVersion": 1,
  "id": "req_01",
  "method": "conversation.list",
  "params": {
    "route": {
      "deviceId": "device_01",
      "providerInstanceId": "instance_01"
    }
  }
}
```

Gateway v2 response：

```json
{
  "protocolVersion": 1,
  "id": "req_01",
  "method": "conversation.list",
  "response": {
    "status": "ok",
    "result": {}
  }
}
```

Gateway v2 event：

```json
{
  "protocolVersion": 1,
  "eventCursor": "event_1042",
  "event": "turn.upserted",
  "payload": {}
}
```

Provider request 则使用标准 JSON-RPC；generated classifier 严格区分 request/response/notification/declared event，response 必须且只能包含 result 或 error：

```json
{
  "jsonrpc": "2.0",
  "id": "req_01",
  "method": "conversation.list",
  "params": {}
}
```

现有进程内 Runtime Gateway 仍走 `desktop/v0` 的 v0 envelope 和 `eventSequence`，它是兼容 profile，不是 gateway v2 或 Provider wire。未来远程 Transport 可在 gateway envelope 外增加 connection id、ack 和密文 framing；这些字段不进入业务协议。

### 初始化与能力协商

当前 Pet 使用 `protocol.initialize`、Gateway 使用 `protocol.handshake`、Provider 使用 `provider.initialize` 协商 `VersionRange` 并返回 selected version。此前 `system.initialize` 名称仅属未来草案，已被当前 manifests superseded。协商内容包括：

- client 名称、版本和平台；
- client 支持的最小/最大协议版本；
- Gateway 选定的协议版本；
- server 名称和版本；
- event replay、资源传输等协议能力；
- Provider 列表、状态和 capability 概要。

Provider instance capability 当前通过 `instance.capabilities` 读取，manifest capability metadata 映射到 schema 中的 typed capability enum。UI 不写死模型、推理强度和权限枚举，而是按 Provider 返回值构建选择器。旧客户端面对新能力时隐藏未知操作，而不是依赖 App 版本猜测。

### 初始公共方法

以下清单是完整远程产品的未来能力草案，不是当前 v1 manifest。当前可生成接口只以 `pet/v1/manifest.json`、`provider/v1/manifest.json` 和 `gateway/v2/manifest.json` 为准。

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

当前协议事实已收敛到 `protocol/{core,pet,provider,gateway}/v1` 与 `protocol/codegen.json`。详细的分层、设备路由、生成包和验证路径见 `protocol-layers-and-device-routing.md`；长期取舍见 `../50-decisions/language-neutral-protocol-idl-and-sdk-boundary.md`。

Provider Host 已实现 Gateway v2 application service。桌面 v0 wire profile 位于 `protocol/desktop/v0`，并生成到 `sdk/rust/codepet-desktop-sdk` 和 TypeScript desktop SDK；`runtime_gateway/provider_host_compat.rs` 只把这层既有调用面映射到同一个 Gateway v2 service。Provider v1 event 已接入远程 Tauri event/replay，但明确不接入 companion 或桌宠 projection。

阶段二留下的手写 compat core 位于 `src-tauri/src/runtime_gateway/`，当前主要服务 Desktop companion 与协议回归：

- `provider.rs` 定义 Provider adapter 的异步能力边界，方法参数和结果直接使用生成的 v0 request/response。
- `registry.rs` 按 `providerId` 注册、移除、枚举和路由 adapter，并把未知 Provider 与非 ready Provider 分别映射为标准 `unknown_provider` 和 `provider_unavailable` 错误。
- `gateway.rs` 实现生成的 `ProtocolServer`。握手、Provider 枚举和六个 Provider 操作都经过同一 registry；空 registry 是合法启动状态。
- `event_bus.rs` 接收生成的 `ProtocolEvent`，覆盖其 wire version 和 sequence，分配进程内单调 sequence，并维护有界内存重放窗口。窗口之外的 cursor 返回 `event_replay_unavailable`，由客户端重新获取快照；该窗口不是持久化会话存储。
- `transport.rs` 的 `Transport` contract 将 request/response dispatch 与 event subscribe/replay 分开；`LocalTransport` 是直接调用同一 Gateway 的 in-process 实现，不包含 WebSocket、P2P、认证或远程加密。
- `provider_host_compat.rs` 把 remote v0 request/response/event 映射到 Provider Gateway v2；不得启动 Provider/App Server 或创建第二个 remote event bus。
- `tauri_bridge.rs` 组合 Host/resolver 并传递生成的标准 DTO。remote 使用 `runtime_gateway_request/replay` 与 `runtime-gateway-event`；桌宠使用 `codex_desktop_companion_request/replay/snapshot` 与 `codex-desktop-companion-event`。两套 command、event bus 和 sequence 不得合并；Provider 原生 `Value` 不得穿过该边界。

本地调用链为：

```text
remote generated request
  → runtime_gateway_request
  → provider_host_compat / ProviderGatewayService
  → PluginManager / codepet-provider-codex / App Server

remote ProtocolEvent
  → ProviderGatewayService replay / runtime-gateway-event
  → 远程客户端（PetApp 不订阅）

Pet companion request/snapshot
  → codex_desktop_companion_* command
  → companion LocalTransport / Gateway / Desktop IPC adapter

Desktop ProtocolEvent
  → companion GatewayEventBus / codex-desktop-companion-event
  → PetApp activity projection
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

手机调用 `approval.resolve` 时只提交标准 decision id 和被 capability 允许的参数。当前 Codex Provider 只把普通 command/file 的二元 approve/decline 映射为上游 `accept`/`decline`；额外权限、session/policy decision 与未知语义明确拒绝。手机不得发送任意 Provider JSON-RPC response。

### 用量

用量分为：

- Conversation usage：本轮、累计、缓存输入、reasoning 输出和 context window；
- Account usage：额度窗口、已用比例、重置时间和 spend/credit 状态；
- Provider-specific usage：Qoder credits、OpenCode provider usage 等扩展。

Codex 以 `thread/tokenUsage/updated`、resume 恢复事件、`account/usage/read` 和 `account/rateLimits/read` 为来源，不再扫描 transcript 计算主数据。不同含义的用量不能合并成一个数字。

## Codex Provider

> 当前实现边界：App Server 只属于独立 Codex Provider，已接通 Provider v1 全部 lifecycle/业务方法与六种事件；Gateway/compat 提供 list/get/create、空闲 conversation 的新 turn send、interrupt 和二元 approval，不把 `turn.send` 自动切换成 steer。下文涉及 account/usage、fork/archive、permission/user-input/MCP approval、supervisor/reconciliation 的内容是长期设计，尚不能作为当前 capability。完整矩阵见 `codex-provider-plugin-runtime.md`。

### 接入点

Codex 使用官方 `codex app-server` V2。App Server 支持 thread、turn、item、模型、配置、账户、用量和双向审批。网络 WebSocket transport 仍具有实验性质，因此 Code Pet 在本机优先使用 stdio 或受控 Unix socket；App Server 不监听公网地址。

### 生命周期

Host 从 manifest 启动一个长期 Codex Provider process，并为 Codex instance 创建 App Server session：

```text
Agent Runtime resolver 定位并验证 Codex binary
  → Host 注入绝对 appServerExecutable
  → PluginManager 启动 codepet-provider-codex
  → instance.start 按 manifest appServerArgs 启动 App Server
  → initialize / initialized
  → 持续处理 list/read/start、turn、approval response/notification
```

Code Pet 生命周期内不为每条消息、每个客户端或每个 conversation 单独启动 App Server。

当前已经有：

- writer task：串行写 JSONL；
- reader task：持续解析 RPC response、notification 和 server request；
- stderr task：持续排空并记录诊断，避免管道阻塞；
- pending request map：`requestId → oneshot response`；
- subscription/event dispatcher：按 thread/turn identity 路由事件；
- bounded request/initialize timeout 和退出检测；异常后 Provider 转 unavailable。

持续 supervisor 和重连 reconciliation 尚未实现；runtime refresh 会更新 Host instance setting 并显式重启 Codex 插件，不会重启 Desktop companion。

### 长期会话能力目标

Codex 第一阶段覆盖：

- `thread/list`：设备全部会话、cwd 过滤、分页、搜索和排序；
- `thread/read`、`thread/turns/list`：当前历史加载；`thread/items/list` 只在上游版本实际支持时作为更细分页目标，已实测的 Codex `0.151.0-alpha.7.2` 返回 `-32601`，当前 Provider 不得依赖；
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

2026-08-30 实机已确认：独立 App Server 创建并完成的 thread 会被当前打开的 Codex Desktop 自动加载。这只证明 remote → Desktop 的持久化协同，不证明 App Server 能实时观察 Desktop-originated turn、item 或审批，也不表示两套协议可以共享 owner/event stream。

需要区分：

- Managed conversation：由 Code Pet App Server 创建或恢复，Code Pet 拥有完整实时流和审批通道；
- External conversation：由独立 Codex Desktop/CLI runtime 创建，可能只能通过 list/read 很快发现，实时 delta 和审批能力以实验结果为准。

本项目不为此保留旧 Hook 架构。remote task 暂不投影到桌宠；Desktop companion 只处理 IPC 本地投影，不读取或排除 Provider thread。如果 External conversation 无法完全附着，应在 remote capability 中明确限制，而不是用文件推断伪装权威状态。

### 协议升级

Codex runtime 可以生成版本匹配的 TypeScript 和 JSON Schema。CI 使用生成 schema 做契约对比和 fixture 验证。Rust runtime 不必生成 App Server 的全部类型；它按官方 wire 分类 request/response/notification，强类型化核心 thread/turn/status/usage/approval，并对其他 item 按 `type` 解析必要字段，同时保留未知 payload。上游 wire 不要求 `jsonrpc`；下游 Provider Protocol 仍严格使用 JSON-RPC 2.0。

Codex 增加无关字段时不要求 Code Pet 更新。只有调用参数变化、安全相关审批变化、已有字段语义破坏或产品决定展示新 item 时才更新 adapter。

## 后续 Provider

### OpenCode

OpenCode 最小 adapter 已由独立 `codepet-provider-opencode` 落地。它使用正式 v1.18.25 发行包内的 V2 session HTTP/SSE API，覆盖 list/get/create、queue/steer/interrupt、二元 permission 和 live events。每个 Provider instance 只启动并持有自己的 loopback Server；不发现或附着外部 Server，也不用 Hook/transcript 猜测缺失能力。该实现验证了 Standard Protocol 能直接映射 REST/SSE Provider，而不是只替换 Codex 名称。详见 `opencode-provider-plugin-runtime.md`。

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

用户打开会话后通过 `conversation.get(cursor, limit)` 分页读取 turns/items。limit 是最大值；Remote 以 40 为期望值，只有收到 `provider_response_too_large` 才保持原 cursor 将 limit 减半，成功后使用 nextCursor 拉取下一页。实时事件携带 sequence；重连客户端提交最后 ack，Gateway 在可用窗口内重放，否则返回新 snapshot。Transport 重试不能自动重放非幂等业务 request。

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

当前已落地的 Host core 使用 `DeviceRegistry` 作为 `deviceId/displayName` 唯一事实来源，不创建第二设备身份。LAN TLS leaf certificate 与 PKCS#8 private key 以 owner-only 原子文件持久化；leaf DER SHA-256 wire 值固定为 64 位小写 hex。TLS identity 变化会使绑定旧 fingerprint 的 credential store 安全重建为空。pairing id 与 32-byte secret 仅驻内存、五分钟过期、Host 重启失效，成功交换一次后原子作废。每个 remote client 获得 opaque 256-bit bearer，Host 只持久化其 SHA-256，并保存 client metadata、创建/最后访问/撤销时间。RBAC、刷新 token、证书轮换、mTLS 与后台连接不在该 core 中。

## 实施路径

以下阶段描述依赖顺序和完成条件，不表示时间排期。每个阶段完成后保持代码库可运行，并删除已被替代的旧实现。

### 阶段一：协议与生成基础（已落地范围）

- 建立 `protocol/{core,pet,provider,gateway}/v1`、分层 manifest、JSON Schema 子集和显式版本协商。
- 定义 Provider lifecycle/conversation/turn/approval/event/shutdown、Gateway device/instance routing 与 Pet snapshot/patch/action 边界。
- 实现 Rust/TypeScript/Dart target adapter；Dart 覆盖 core/Gateway v2，Python 保持 planned 且 fail closed。
- 生成五个 Rust SDK 的 DTO、server/client、dispatcher、typed capability mapping 和 codec；Provider 包含有界 stdio-json-lines framing。
- 建立 fixture、dependency/capability/target 负例、Rust SDK 和 desktop-v0 round-trip 测试。
- 现有 Tauri Runtime Gateway 通过 desktop SDK re-export 保持编译和双链路行为；未实现旧草案中的 `system.health` 或 UI 全量迁移。

完成状态：IDL 已是协议事实来源，现有 v0 wire 只作为同一 IDL 根下的兼容 profile；真实 gateway v2 remote session 与 Pet v1 adapter 仍属后续阶段。

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

### 阶段六：远程 Transport 与设备模型（Host 安全持久化 core 已部分落地）

- 设备身份继续复用 `DeviceRegistry`；LAN TLS identity、短期配对、hashed bearer 校验/列出/撤销已落地，连接授权 route 与设备权限尚未实现。
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

- OpenCode Server adapter 已完成最小验证；后续只按正式 Server schema 扩展能力。
- 再实现 Claude Agent SDK/runtime adapter，验证 sidecar、callback approval 和本地 session 模型。
- 仅在真实差异无法通过 capability 或 extension 表达时扩展标准协议；不得把 Provider 分支散落到 UI。
- 根据开放能力评估 Qoder 和其他 Provider。

完成条件：新增 Provider 主要修改其 adapter、capability 和必要 renderer，不要求推翻 Gateway、Transport 或公共控制协议。

## 涉及模块

- `protocol/`、`tools/protocol-codegen/`、`sdk/rust/`、`sdk/typescript/`：当前协议事实来源、target adapters、测试和生成产物；旧 `protocol-codegen/`/`generated/` 根目录方案已 superseded。
- `src-tauri/src/runtime_gateway/`：新增 Gateway、Application、Provider、Transport 和 projection store 边界。
- `crates/codepet-host/src/remote_access.rs`：LAN TLS identity、pairing session 和 remote credential store；不得引用 `ProviderGatewayService`、Desktop IPC companion、Pet 或 activity store。
- `crates/codepet-host/src/persistence.rs`：沿用既有 tempfile + fsync + replace 原子写，并为 TLS/private credential 文件增加 regular-file 检查和 Unix `0600` 保护。
- `crates/providers/codepet-provider-codex/`：长期 remote App Server session、mapper、Provider v1 server 与 stdio 主循环；不得反向依赖 Tauri/Host/Pet SDK。
- `crates/providers/codepet-provider-opencode/`：OpenCode Server HTTP/SSE client、mapper、Provider v1 server 与 stdio 主循环；同样不得反向依赖 Tauri/Host/Pet SDK。
- `src-tauri/src/agent/codex_app_server.rs` 与同名目录：已删除；不得恢复进程内直连、旧一次性 `PetEvent` reply driver 或 fallback。
- `src-tauri/src/agent/codex_desktop_ipc/`：独立 Desktop companion、Owner/Follower revision 状态机和安全动作。
- `src-tauri/src/activity/collector.rs`：Agent Hook 数据入口删除；若其他本地非 Agent 功能仍需 HTTP 服务，应拆出独立用途后保留。
- `src-tauri/src/activity/events.rs`：`PetEvent` 主模型被标准 protocol 和 Activity Projection 替代。
- `src-tauri/src/activity/token_usage.rs`：扫描式 Agent 用量主路径被 Provider usage 替代；仅保留仍有独立产品价值的非 Provider 统计时才拆分保留。
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

- 风险：独立 Codex Desktop 与 Code Pet App Server 不共享同一实时事件流。缓解与验证：已验证 remote-created thread 会被 Desktop 加载；其他方向仍按 Managed/External capability 表达，不用文件推断。
- 风险：Codex App Server V2 和部分 transport/API 仍在演进。缓解与验证：固定支持版本范围，生成当前 runtime schema，维护协议 fixture，CI 覆盖最低支持、推荐和最新版本。
- 风险：自定义标准协议过度复制 Codex，导致其他 Provider 难以接入。缓解与验证：OpenCode 作为第二 adapter 进行架构验收；公共模型只包含产品公共语义，Provider 差异放 capability/extension。
- 风险：公共模型过薄，手机被迫解析原生 Provider payload。缓解与验证：手机 MVP 的每个页面只使用标准 DTO；`providerData` 只能用于增强展示，不能成为核心流程依赖。
- 风险：公共模型过厚，Provider 新字段导致频繁全端升级。缓解与验证：未知字段容忍、unknown timeline item、按需投影和 schema 兼容性测试。
- 风险：未来 Rust、TypeScript、Dart 和 Python adapter 产生不同的 optional、nullable、enum 或整数行为。缓解与验证：当前先限定 JSON Schema 子集并让未实现 target fail closed；每个新 adapter 上线前增加共享 fixture round-trip 与负例。
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

- 验证 schema 与 manifest 的所有 `$ref` 可解析、符合 declared dependency/layer，schema 符合受支持子集。
- 当前 Rust SDK、TypeScript Gateway/compatibility SDK 与 Dart Gateway SDK 对共享 fixture 或类型面执行 decode/encode/strict compile；Dart 另验证 closed-object/constraint、判别 union、nullable、secret redaction、metadata 与 typed client，Python 只验证显式选择失败。
- 覆盖 optional、nullable、unknown enum、unknown timeline item、int64 和错误响应。
- CI 重新生成代码并检查工作区无差异，禁止提交过期生成物。
- 检查 method/event 名称唯一，request/response `$ref` 存在，capability enum/container/method mapping 一致。
- Provider 覆盖有界 line framing、超限、坏包、result/error XOR、未知 method、非法 params、notification/event 分类与 request id 保留。

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

- Host 定向测试覆盖 TLS identity 重启持久化与损坏轮换、`0600` secret file、pairing 五分钟过期/重启失效/并发单次消费、bearer hash 落盘和指定客户端/当前 credential 撤销。
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

- 已验证 remote App Server 创建并完成的 thread 会被 Desktop 加载；App Server 对 Desktop-originated 实时 turn/item/approval 的可见程度仍未确认。
- Desktop companion 当前使用私有 Owner/Follower IPC，其长期兼容承诺尚未确认；升级必须 fail closed 并重新验证。
- remote 与 Desktop 的同名 thread 暂不去重；若未来增加跨链路 origin 协调，必须先在协议中定义可验证身份，不能从时间窗口或文件状态猜测。
- Code Pet Standard Protocol 已采用 `tools/protocol-codegen` 自研最小生成器和 `codepet.protocol.codegen/v1` target adapter contract；未来只在现有 schema 子集无法表达跨语言需求时再评估 OpenRPC 等输入层。
- Flutter 手机项目的最终状态管理、UI 组件和发布方式尚未确定，不影响 Dart protocol package 设计。
- 远程首版采用 WSS relay 先行还是同时实现 WebRTC，需要在 Transport Spike 中根据中国大陆实测决定；业务协议不依赖该选择。
- projection cache 是否在 Codex 阶段立即使用 SQLite，还是先用可重建内存 store，取决于手机断线重放和 App 重启恢复的最小需求。
- Claude Agent SDK sidecar 的语言、进程托管和与现有 Claude Code UI 的 session 共享程度尚未验证。
- OpenCode 外部 Server 发现与多客户端附着不是当前能力；若未来需要，必须先有官方可验证的 instance 身份和认证边界，不能在 Provider 内新增第二套权威探测。
- 产品最终是否继续支持 Cursor，取决于其是否提供足够完整、稳定且可验证的运行时接口。
