# 最近会话 v1 冻结契约与 SDK 接入

## 背景与范围

2026-09-08 R1 依据 [语义设计](../10-architecture/recent-conversation-feed.md) 和 [交付计划](recent-conversation-delivery.md) 冻结 canonical wire。协议仍为 v1。本文记录准确字段和消费者接入要求；产品语义仍以设计为权威。

证据：Provider 原有 `ConversationListRequest` 必填 `route/projectFilter`；Gateway 原有 `ConversationMarkReadRequest` 只有 `conversation/observedActivityVersion`；Core 已有 `PageInfo`、`TimestampMs`、`EventCursor`；Agent 已有 `Conversation`、`ConversationReadState`、`ConversationStatus`。这些既有 shape 与含义不变。Host `conversation_state.rs` 的旧文档包含全局版本、scope baseline、逐会话 read/latest version 和三个 fingerprint，不能仅用 markRead 重建。

目标是让 Host 能完整收集原子候选集合、独立分页最近视图，并让 Remote 只依赖 Gateway。非目标：把 14 天、最近排序下沉 Provider；修改聊天/项目分页；重置已读；升级协议；编译或运行测试。

## Gateway wire

所有请求/响应沿用 JSON-RPC 2.0；以下表格省略 envelope。

| method/event | 请求/事件 payload | 响应 |
| --- | --- | --- |
| `conversation.recent` | `ConversationRecentRequest { providerId, cursor?, limit? }` | `ConversationRecentResponse { conversations: Conversation[], pageInfo: PageInfo, revision: ConversationRecentRevision, snapshotCursor: EventCursor }` |
| `conversation.recentChanged` | `ConversationRecentChangedEvent { providerId, revision }` | 无 |

`limit` 默认 20、范围 1..100；Rust 为 `Option<u64>`，Dart 为 `int?`。`revision` 是非空字符串别名，客户端只比较是否相等，不能解析或递增。快照每页 revision 相同，`pageInfo.nextCursor` 缺失才是结束；没有总数。摘要使用既有 Agent `Conversation`，每条须带该 reader 的权威 `readState`。

Gateway 事件仍是 `{jsonrpc:"2.0",method:"conversation.recentChanged",params:{eventCursor,payload:{providerId,revision}}}`。`snapshotCursor` 是 Provider 查询前捕获的重放 fence，原样传给 `event.subscribe.afterCursor`；不等于 recent revision。Host 必须处理构建中变化，不能在新事件之后安装旧快照。

最近游标绑定认证 callerScope、provider、实例 generation、快照 revision、时间截止与位置。过期返回 `recent_cursor_expired`；篡改或绑定不符返回 `invalid_cursor`。新首屏替换整个最近窗口，旧尾页不可接入新 revision。失效事件可以定向给该 reader；若采用共享 replay 的 provider 范围提示，则 revision 必须采用一致的 provider 范围失效版本，事件不得携带另一 reader 的私有版本/成员信息。

Gateway capability 新增 `conversation.recent`（Rust `GatewayCapability::ConversationRecent`，Dart `GatewayCapability.conversationRecent`）。只有 Provider 日期/IDs 查询及 active/unread/markRead 权威语义齐备，才广告此能力。旧 Gateway `conversation.markRead` 请求 `{conversation,observedActivityVersion}`、响应 `{readState}` 完全不变。Gateway 请求里不接受 `readerScope`；Host 必须从认证上下文导出，未认证调用方不能注入。普通 `conversation.list` 没有新增 query 字段，Standalone/Project 含义不变。

## Provider 原子 wire

所有资源字段 `conversation` 使用 Provider 私有四段 `ProviderResourceId`；`route` 为既有 `ProviderInstanceRoute`。共享 Agent 摘要资源仍使用 `RoutedResourceId`。

| method / DTO 前缀 | 请求 | 响应 |
| --- | --- | --- |
| `conversation.active.list` / `ConversationActiveList` | `{route,cursor?,limit?}` | `{conversations:ConversationActiveEntry[],pageInfo,revision}` |
| `conversation.unread.list` / `ConversationUnreadList` | `{route,readerScope,cursor?,limit?}` | `{conversations:ConversationUnreadEntry[],pageInfo,revision}` |
| `conversation.markRead` / `ConversationMarkRead` | `{conversation,readerScope,observedActivityVersion}` | `{readState:ConversationReadState}` |

DTO 前缀加 `Request/Response`。三个 methods 各有同名 `ProviderCapability`，分别生成 `ConversationActiveList/ConversationUnreadList/ConversationMarkRead` 枚举成员及 `conversation_active_list/conversation_unread_list/conversation_mark_read` Rust trait/client 方法。默认 trait 实现返回既有 `method_not_implemented` 错误，不能把 unsupported 改成空集合。

`ConversationActiveEntry {conversation,status:ConversationStatus,activityVersion:string}`；活动包含 running、waiting-approval、waiting-user-input 的未终结执行。`ConversationUnreadEntry {conversation,readState}` 必须满足 `readState.unread=true`。`activityVersion` 沿用已读观察版本，不是分页 revision。

active/unread 枚举必须覆盖完整集合、提供 immutable snapshot；默认 20、上限 100，按 nativeResourceId 升序。分页游标绑定 route/generation，unread 还绑定 readerScope。每页 `revision: ConversationEnumerationRevision` 相同，它是非空不透明字符串；过期 `conversation_cursor_expired`，绑定错误 `invalid_cursor`。不能任意截断为 N 项；nextCursor 缺失才表示完整结束。变化事件使用同一枚举版本体系，Host 收到构建中变化须失效/重试。

`ReaderScope` 是非空不透明字符串，来自可信 Host，保持旧 callerScope 身份。markRead 单调、持久推进到 observedActivityVersion；若最新版本更高，返回仍 unread。非法或未来观察版本为 `invalid_activity_version`。

## Provider list 查询与协商

原 `ConversationListRequest` 新增两个可选字段：`query: ConversationListQuery`、`readerScope: ReaderScope`。缺省时严格沿用旧请求行为。Rust literal 要新增 `query: None, reader_scope: None`。

query 是闭合 `kind` 联合，不能同时使用两种模式：

- `ConversationUpdatedAfterQuery {kind:"updatedAfter",updatedAfter:TimestampMs}`：UTC epoch 毫秒、包含边界；完整源先过滤再分页；updatedAt 降序、同时间 native ID 升序；缺少 updatedAt 不匹配；projectFilter 与日期条件取交集。
- `ConversationIdsQuery {kind:"ids",ids:NativeResourceId[]}`：1..100 个不同 native ID；`projectFilter.kind` 必须为 `all`，否则 `invalid_request`；先筛选再分页，native ID 升序；Host 必须取尽全部返回页。已删除项可以省略；查询失败不能当删除。IDs 批量上限由 Provider 服务验证（当前生成器的数组约束支持非空/去重，不含 maxItems）。

游标绑定 route、query、projectFilter、readerScope，不能跨查询接续。传 readerScope 时每条摘要携带权威 readState。日期/IDs 都不能只过滤 Harness 已返回的一页，更不能用 transcript N+1 来补齐摘要。

`ProviderCapabilities` 新增可选 `conversationListQuery: ConversationListQueryCapabilities {updatedAfter:boolean,ids:boolean}`。没有对象或相应字段 false 表示不支持；模式 true 承诺对应 query 完整性；readerScope 另外要求 `conversation.unread.list` capability 与可用权威 store，因此无已读存储时仍可独立提供日期/IDs 查询。Host 先协商，再发送新字段，以免旧严格 decoder 拒绝。能力放在独立 query 描述对象，因为既有 `methods` capability 枚举只能对应真实方法。

## Provider 事实事件与已读接管

Provider 事件沿用 `{jsonrpc:"2.0",method,params:<payload>}`，没有 Gateway eventCursor wrapper。

| event / DTO | payload |
| --- | --- |
| `event.conversationActiveChanged` / `ConversationActiveChangedEvent` | `{conversation,status,activityVersion,active:boolean,revision}` |
| `event.conversationUnreadChanged` / `ConversationUnreadChangedEvent` | `{conversation,readerScope,readState,revision}` |
| `event.conversationDeleted` / `ConversationDeletedEvent` | `{conversation}` |

active=false 表示退出活动集合；unread=false 表示读状态清除。revision 分别属于 active 或对应 scope 的 unread 枚举。既有摘要/turn/item 事件不变。删除事件是权威事实并使相关摘要/active/unread 快照失效；只因为普通请求出错或漏项不能发布删除。Host 不得把 readerScope/readState 广播给其他 reader。

PM/R2/R3 已确认通过同路径、同 v1 文档的公共 SDK store 与进程锁接管，不新增 bootstrap RPC。全局版本、scope baseline、逐会话阅读进度与指纹全部保留；须原子、幂等、有备份。缺少共享存储或接管尚未完成时 unread/markRead 不可广告为可用，更不能返回假空/重置数据。此处是接管约束，R1 不实现存储迁移。

## 生成与消费者适配

R1 修改 `protocol/provider/v1` 与 `protocol/gateway/v1`，因为它们分别拥有原子接口与聚合 facade；Core/Agent 已有的公共类型直接复用，没有复制模型。生成器只扩展 manifest method 名称校验以支持三段名称。生成输出为：

- `sdk/rust/codepet-provider-sdk/src/generated.rs`
- `sdk/rust/codepet-gateway-sdk/src/generated.rs`
- `sdk/typescript/codepet-provider-sdk/src/generated.ts`
- `sdk/typescript/codepet-gateway-sdk/src/generated.ts`
- `sdk/dart/codepet-gateway-sdk/lib/src/generated.dart`

源码生成命令（不会编译 SDK）：`node tools/protocol-codegen/generate.mjs`。Dart 独立导出命令，从本 Host worktree 根目录执行：

```powershell
bun tools/cp-sdk-gen/cp-sdk-gen.mjs --package gateway --role client --lang dart --protocol ./protocol --output <独立导出目录>
```

本 worktree 已生成的独立导出目录为 `sdk/rust/target/recent-gateway-dart/`，包含 `codepet-core-sdk/`、`codepet-agent-sdk/`、`codepet-gateway-sdk/` 与 `cp-sdk-gen.lock.json`。该目录按仓库 target 规则忽略，不提交为源码。导出包含递归 Core/Agent 依赖与 digest lock；可交 Remote 同步，不能向 Remote 仓库 cherry-pick Host 提交，也不能手改 generated。Dart 接入：`ProtocolClient.conversationRecent(ConversationRecentRequest(...))`；响应 `ConversationRecentResponse`；事件名 `ProtocolEventName.conversationRecentChanged`，payload `ConversationRecentChangedEvent`。

R2 适配三个 Provider mapper 的 `ProviderCapabilities` literal（旧路径 `conversation_list_query: None`），各 Provider 查询 literal 及测试，以及新原子 trait 实现/事件。R3 适配 Host `gateway.rs` 的 Provider list literal、`tests/fixtures/fake_provider.rs` capability literal、枚举 exhaustive match、Provider dispatcher/manager 和 Gateway recent handler。`crates/providers/test_support/native_runtime.rs` 与 Provider native/vertical 测试的 Provider list literals 归 R2；Host 测试中 Provider literals 归 R3。Gateway 原 list DTO literal 不受影响。新 trait 方法有默认 unsupported，不要求无关 mock 实现。

## 验证与风险

已执行 `node tools/protocol-codegen/generate.mjs`、`node tools/protocol-codegen/generate.mjs --check`、`git diff --check` 以及上述 Bun Dart 导出命令，均未触发编译。生成器加载 canonical manifest/schema 并检查正负 fixture；这只证明生成输入与结构自洽，不证明运行时行为。新增 fixture 覆盖日期/IDs 互斥、空/重复 IDs、日期边界、active/unread 首尾页相同 revision、true/false 事件、删除、markRead 新活动竞态响应、Gateway recent 首尾页、缺 revision/fence、scope 注入及 limit 范围。新增生成器测试覆盖多段 namespace 与旧 list/markRead 边界，测试代码未执行。

后续验证路径：R2/R3 服务检查 IDs 上限与 projectFilter、分页 snapshot 一致性、false/read 事件、旧数据无损接管；R4 检查 recent 事件 wrapper 与独立游标；SDK codec/Host/Provider/Remote 测试验证实际行为。特别注意 Rust serde/TypeScript 类型不等于所有业务约束已验证，服务仍须校验批次、排序、scope 和 capability。

本轮未执行 `cargo check/test/build`、`flutter build/test/analyze`、`npm build`、`protocol:test`、`protocol:check` 或任何 CI；没有编译 SDK。性能、真机、实际 Harness 完整性以及已读迁移运行结果均未验证。本文作为契约冻结记录；后续字段变更必须同时通知 PM/R2/R3/R4。
