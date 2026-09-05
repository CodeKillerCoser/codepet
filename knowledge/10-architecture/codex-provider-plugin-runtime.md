# Codex Provider 插件运行时

## 当前结论

远程 Codex 能力已经从 Tauri 进程内直连实现迁到独立二进制 `crates/providers/codepet-provider-codex`。二进制只依赖 `codepet-provider-sdk`，通过 Provider Protocol v1 的 JSON-RPC 2.0 业务 payload 与 `codepet-host` 通信；stdin/stdout 物理通道使用 SDK Runtime 的 CodePet Provider Frame V1。它不依赖 `codepet-host`、Tauri、Pet SDK 或 Desktop 私有 IPC。Provider 入口只构造 `CodexProvider` 并调用 SDK 的 `serve_stdio`；JSON 序列化、raw/zstd、reader、writer、dispatcher、event sink、普通/控制双通路、过载、frame limit 与 terminal cleanup 全部属于公共 SDK runtime，Codex crate 不再复制或感知 transport server。

每个 Provider instance 只持有一个共享 App Server。instance.start 完成 initialize、model/list 与 Project API 探测，所有 conversation 的 create/resume/read/turn/approval 复用该进程；会话槽不再拥有独立子进程。公共 SDK 接收 Host 心跳携带的客户端集合：有连接时协调实例启动并保持所有已 resume 会话，最后连接离开或 Host 心跳过期时停止 Server，包括 active turn。页面退出不释放会话，不存在续租 timer 或 reaper。Provider 插件本身保持在线。完整模块和心跳设计见 [连接架构](remote-and-provider-connections.md)。

Server 从 spawn 前占位开始进入 generation/cancellation registry；stop/shutdown 等待占位收敛、进程退出才发布 stopped。SDK 普通请求保持 16 active/32 pending，生命周期 control 保留 2 active/4 pending，过载按原 id 返回 provider_overloaded。provider.ping 在 reader 中直接确认，实例启动/停止由独立协调任务完成。

Host 是唯一进程与路由所有者：`PluginManager` 从 manifest 启动 Provider、创建实例并启动连接心跳，`ProviderGatewayService` 把 Gateway v1 请求路由到实例，并把 Provider 事件变成可 replay 的远程事件。兼容 v0 的 Tauri command/event 只是一层 Gateway DTO 适配，不再拥有或启动 Codex App Server。

Remote 进入会话统一调用 Gateway `conversation.resume(limit=20)`。Host 复用已有 `conversation.acquireInteraction` 与 `conversation.get`，成功时一并返回交互结果和首屏；获取交互权失败时返回 `interactionAcquired=false` 与结构化错误，Remote 再 get 只读历史。原生 resume 显式 `excludeTurns=true`，不返回全量 turns。Remote 复用 get 的组装流程消费首屏并拉完剩余 cursor；历史超限只缩页，不重做 resume。

## 目标

- 以普通 Provider manifest/binary 接入 Codex，不保留进程内兼容实现或隐式 fallback。
- Provider 内只保留官方 Codex App Server client、标准协议 mapper、实例状态和 stdio 主循环。
- 保留 Desktop Companion 的实时桌宠链路，并从源头禁止 Provider 数据进入 Pet 投影。
- 让设备、插件、实例与原生资源身份在请求、响应、事件和审批回写中保持可追踪。

## 非目标

- 不实现 dylib ABI、插件市场、签名、沙箱、自动重启/backoff 或第二套运行时。
- 不把 Desktop 私有 IPC 放入 Provider，也不让桌宠动作调用 Plugin Manager。
- 保留既有 LAN/WSS、配对、凭据与网络地址行为；连接集合由 Host 管理，生命周期策略由 SDK 执行。
- 不伪造 Codex App Server 不支持的能力或审批成功结果。

## 审计边界

| 处理 | 基线模块 | 结论 |
| --- | --- | --- |
| 提取 | `src-tauri/src/agent/codex_app_server/client.rs`、`protocol.rs` | 移入 Provider crate，去掉 runtime resolver、Tauri log 和硬编码启动参数，只保留纯 Rust App Server client/DTO。 |
| 重写 | 旧 App Server mapper/provider | 按 Provider v1 的 routed resource、instance lifecycle、capability 和 event 类型实现；不再依赖 compat v0 Provider trait。 |
| 删除 | Tauri 内 `codex_app_server` module 与 `CodexRemoteProviderAdapter` 生命周期 | Tauri 不再直接 spawn App Server，也不再拥有独立 remote registry/session/event source。 |
| 保留 | `codex_desktop_ipc`、`CodexDesktopCompanionState`、companion snapshot/replay/event | 继续作为 Pet UI 的唯一 Codex 数据和动作来源。 |
| 删除 | Provider 到 `CodexThreadScope` / Desktop Companion 的来源协调 | remote 与 Desktop 允许出现同名 thread；Provider 不标记、排除或同步 Desktop 投影。 |
| 薄适配 | `runtime_gateway/provider_host_compat.rs` | 把既有 v0 UI 调用映射到 `ProviderGatewayService`；不启动进程，不持有 App Server client，不产生第二份事件。 |

## 两条数据链路

远程链路：

```text
Gateway/既有远程 UI
  -> runtime_gateway_* compat 薄适配
  -> ProviderGatewayService
  -> PluginManager / PluginProcess
  -> codepet-provider-codex
  -> 官方 Codex App Server stdio wire

Provider event
  -> PluginManager 校验 route
  -> ProviderGatewayService cursor/replay
  -> runtime-gateway-event
```

桌宠链路：

```text
Codex Desktop 私有 IPC
  -> CodexDesktopCompanionAdapter
  -> CodexDesktopCompanionState snapshot/replay
  -> codex-desktop-companion-event
  -> Pet UI activity projection
```

两条链路没有 payload、registry、event bus、replay、session、owner、来源排除状态或动作路由的交叉。Pet UI 不导入 remote client，不监听 `runtime-gateway-event`；Provider 事件不进入 `SharedState` activity、companion replay、PetProjection 或 `pet-event`。同名 remote/Desktop thread 暂时各自存在，任何一侧都不尝试排除或同步另一侧。

## 配置权威

配置分为两个不同进程，不能混为一谈：

| 配置 | 唯一权威 | 传递方式 |
| --- | --- | --- |
| Provider 二进制路径、Provider 参数与环境变量 | `codepet-provider.json` | Catalog 将相对 `executable` 按 manifest 目录解析后交给 `PluginManager`。 |
| Codex App Server executable | Tauri `AgentRuntimeService` 的 Codex resolver | Host 在内存中的 Codex instance settings 覆盖 `appServerExecutable`，必须是绝对路径。manifest 中的同名字段会先被移除。 |
| Codex App Server 启动参数 | Codex Provider manifest 的 `appServerArgs` | Host 原样放入 instance settings；Provider 不补默认参数。 |
| 可广告的 model/reasoning effort | 当前 共享 Server App Server 的官方 `model/list` | model 使用全部可见项；全局 reasoning control 只发布所有已广告 model 的共同支持交集，Provider 不猜测或硬编码列表。 |

应用启动和 runtime set/clear/refresh 都使用同一个 resolver 结果更新 Codex instance setting，然后显式重启该 Provider 插件。Desktop IPC connection、owner、revision 和 companion projection 不受影响。没有 resolver 结果时 `appServerExecutable` 缺失，实例 create 明确失败；不会搜索用户目录、调用 Desktop IPC 或启动备用 App Server。

## Provider v1 覆盖矩阵

| Provider v1 方法 | Codex 映射 | 状态与边界 |
| --- | --- | --- |
| `provider.initialize` | 校验版本与 Host identity，绑定单一 device/client | 支持；同一进程不能改绑另一 Host。 |
| `provider.describe` | 返回 `dev.codepet.codex`、版本与 `codex` instance kind | 支持。 |
| `instance.create` | 解码 Host 注入的 settings，建立实例状态 | 支持；executable 必须是绝对路径，未知字段失败。 |
| `instance.start` | 启动唯一共享 Server、initialize、model/list、Project API 探测和 reader | 支持且幂等；spawn 前登记，只有当前 generation 可发布 Ready。 |
| `instance.stop` | 关闭该实例 Server，取消全部会话槽与审批 | 支持且幂等；等待 PID 退出后返回 stopped。 |
| `instance.capabilities` | 实例缓存的真实能力与模型目录 | 只广告实际支持的能力，revision 随实例配置和目录变化。 |
| `project.list/get/create/update/delete` | 共享 Server 的实验 Project API | project/list 探测成功才整组支持；-32601 不广告，不把 Section 映射为 Project。 |
| `conversation.list/search` | thread/list、可选 searchTerm | cursor/limit、updated_at desc、useStateDbOnly=true，all/standalone/project 三态筛选。 |
| `conversation.get` | metadata thread/read 与 full turns 单页读取 | 纯读，不 resume；透传 caller cursor/limit 和上游 nextCursor。tool 文本在 item 生成时按独立策略截断并记录 `_meta`。 |
| `conversation.acquireInteraction` | 共享 Server 的 thread/resume(excludeTurns=true)，或复用 Ready 槽 | 返回权威 selection；幂等，无租期，leaseExpiresAt 兼容字段省略。 |
| `conversation.create` | 同一 Server 的 thread/start | 支持 permission/model/reasoning/workspace/显式 project；响应建立会话槽，首条消息直接 turn/start，不等待历史物化。不支持的 title/extension 明确报错。 |
| `turn.start` | 同一 Server 的 resume、权威 read、turn/start | 首次并发请求合并 resume；原样传递 clientRequestId，未知结果不自动重发。 |
| `turn.steer/interrupt` | 同一 Server 的原生方法与权威 read | 不根据 ack 伪造完整 Turn；终态只清理 active turn，不关闭共享进程。 |
| `approval.resolve` | 同一 Server 的原 request ID 回写 | 仅无损支持普通 command/file accept/decline；同时校验 generation 与 owning conversation。 |
| `provider.ping` | SDK 确认 Host session/sequence/client revision | 立即 pong，异步协调实例；旧心跳不能延长存活。 |
| `provider.shutdown` | 关闭全部实例 Server 并结束 stdio | SDK 先取消心跳协调，再全局 cleanup、有界 drain/abort；EOF/fatal/event 写失败进入同一路径。 |

Gateway v1 的 `turn.send` 只对已有空闲 conversation 启动新 turn，并在 Host 中映射为 Provider `turn.start`。Provider v1 的 `turn.steer` 仍是独立内部能力，不由 Gateway `turn.send` 自动选择。兼容 v0 继续暴露既有 `turn.send` 调用形状，但 `canSteer=false`，也不会根据 Codex `pluginId` 推断 steering 能力。

Provider 发送全部八种 v1 事件：`event.instanceStatusChanged`、`event.projectChanged`、`event.conversationUpserted`、`event.conversationItemUpserted`、`event.turnUpserted`、`event.turnOutputDelta`、`event.approvalRequested`、`event.approvalResolved`。所有事件由唯一 Server reader 映射，避免多 reader 重复发布；delta 自带 conversation route，不依赖 replay 顺序补状态；item upsert 用于提交工具调用的结构化状态和结果。App Server 的 `waitingOnApproval` 与 `waitingOnUserInput` 分别映射为 v1 的 `waiting-approval` 与 `waiting-user-input`。未知 notification 被忽略；未知或无法无损表达的 server request 使用原 request id 返回上游 error `-32601`，不会发布可批准的 Approval。

## 新建会话的工作目录

Remote 按所选项目传递 routed project 和 workspaceRoot。main 模式使用项目原目录；worktree 模式由 Provider 从 workspaceRoot 创建 detached Git 工作树，放到 `~/.codepet/remote_workspace/codex/worktree/<工作树ID>/`，保留请求目录相对仓库根目录的子路径，再把新位置作为 thread/start 的 cwd。projectId 始终保留原项目身份。App Server 不负责通过 workspaceMode 创建工作树。

非项目会话的默认目录为 `~/.codepet/remote_workspace/<provider>/task/<任务ID>/`。Provider 广告的 defaultWorkspaceRoot 仍是 `<provider>` 层；Remote 在其下分配 task 路径，Provider 创建实际目录。Claude/OpenCode 沿用相同 task 分层，worktree 仅向支持该模式的 Provider 开放。已有会话继续使用原路径，本次不迁移旧目录。

## 身份与审批路由

Provider 私有 `ProviderResourceId` 是 `deviceId + providerPluginId + providerInstanceId + nativeResourceId` 四段身份，instance route 是前三段；Agent/Gateway 的语言中立 `RoutedResourceId` 是 `providerId + nativeResourceId` 两段 opaque identity。Host 在 Gateway 入站处解析 instance，在 Provider 响应/事件处校验 `providerId == providerInstanceId`，但 Conversation/Turn/Item/Approval 对象本身直接共享，不再逐字段映射。旧 v0 对象自身的 `id` 继续等于 `nativeResourceId`，两段身份位于 `codepet.gateway.route` extension。

Provider 请求中的 Project 使用四段身份，共享 Agent Project 与 Conversation.project 使用两段身份。Conversation 的 `project` 只映射 App Server `Thread.projectId`，而 `workspaceRoot` 原样映射 `Thread.cwd`；Provider 不通过扫描 Git metadata、归并 worktree 或 cwd 推断项目归属。项目筛选和项目归属创建都在 Provider 与 Host 两层校验 route。

每个实例有一个 Server session registry 和 conversation-keyed 执行槽。槽以 Creating→Ready→Closed 管理，失败进入 Failed 并淘汰；每槽 operation lock 串行 acquire/turn/approval。不同会话共享进程与 Server generation，但不共享 operation lock 或 active turn。首次 resume 的 frame 写入与 cancel 共用短 send gate；stop 先线性化后不得再向旧代发送。

`provider/server_events.rs` 是唯一事件路由器。它按 thread ID 选择槽，缓存 start/resume 响应尚未安装槽时提前到达的事件，同时继续处理其他会话。terminal mapping 和 publication 持会话 operation lock，确保下一操作不会越过尚未发布的终态。Server crash 清理整个实例；明确业务 RPC reject 留在该请求内，不杀掉其他会话。

approval native ID 同时编码 Server generation 与原生 request ID；resolve 还需匹配四段 route、pending 记录与 conversation。旧进程的审批不能批准新进程复用的 request ID。原生 thread/unsubscribe 是取消订阅，不能当作立即释放 writer；当前设计无需按页面调用它。

连接集合是 Server 存活依据，相关规则见 [共享 Server 生命周期](../60-rules/codex-provider-turn-writer-lifecycle.md) 和 [实例初始化回收](../60-rules/codex-provider-instance-session-lifecycle.md)。instance stop/fail/restart 清空会话槽、审批和未物化缓存；重连后显式 acquire，失败业务请求不重放。

## 本地开发安装

1. 构建 Provider，不运行整 App 构建：

   ```sh
   cargo build --manifest-path crates/Cargo.toml -p codepet-provider-codex
   ```

2. 在当前应用数据目录下创建 `provider-plugins/codex/`。应用数据目录以设置中的 `dataDirectory` 为准；未配置时使用 Code Pet 的平台应用数据目录。
3. 将 `crates/target/debug/codepet-provider-codex`（Windows 为 `.exe`）和 `crates/providers/codepet-provider-codex/codepet-provider.json` 复制到该目录。manifest 的相对 `executable` 会指向同目录二进制。
4. 保留 manifest 中的 `appServerArgs`。不要把个人 Codex 路径写入 manifest；Host 会把 Agent Runtime resolver 的绝对路径作为 `appServerExecutable` 注入实例。
5. 也可把开发目录加入 `settings.providerPlugins.directories`；相对目录按应用数据目录解析。相同 `pluginId` 出现在多个目录会 fail closed，而不是随机选择。

样例 manifest 是显式开发安装输入，不是隐式内置插件。Headless Host 若不经过 Tauri resolver，必须由它自己的唯一 runtime 配置层向 instance settings 注入绝对 `appServerExecutable`。

## 影响模块

- `crates/providers/codepet-provider-codex/`：Provider binary、App Server client/mapper、manifest 与真实子进程 fixture。
- `crates/codepet-host/`：manifest settings 覆盖、实例设置更新、插件显式 restart、route 到 plugin identity 查询。
- `src-tauri/src/runtime_gateway/provider_host_compat.rs`：既有 v0 调用面到 Gateway v1 的薄适配。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs`：Host/resolver 组合、远程 event bridge 与独立 companion state。
- `src-tauri/src/agent/codex_desktop_ipc/`：保留 Desktop IPC 状态与动作链路，只删除旧的 remote 来源协调入口；仍是桌宠 Codex 数据源。

## 风险与验证

- App Server 协议漂移：原生 response/notification 不要求 `jsonrpc`，params 可缺失，并允许 trace/emittedAtMs。历史统一调用 `thread/turns/list(itemsView=full)`，不再根据版本选择 notLoaded/item 分页或能力占位；0.151、0.152 和 Host userAgent fixture 均覆盖 full 请求。Section 不映射 Project，Provider Protocol 自己仍严格使用 JSON-RPC 2.0。
- Harness 版本解析认识 `codex-cli/<version>`、`Codex Desktop/<version>` 和 `code-pet/<version>`，后者与 initialize 的 clientInfo.name 共用常量。版本用于 Harness 描述，不再决定历史分页路径。历史版本误判的证据保留在 `../40-runbooks/codex-detail-size-and-capability-loading.md`。
- Project API 漂移：官方 0.151/0.152 schema 提供 Thread Section，而不是 Provider Project CRUD；因此实际 `project/list` probe 返回 `-32601` 时不广告任何 Project 能力，Section 也不会被适配成 Project。typed client 与 Provider fixture 仍覆盖可选上游实验 Project surface 的精确方法/参数、DTO、能力整组探测、CRUD、筛选、项目归属创建和 `-32601` fail-closed，供确实实现该 API 的 harness 使用。
- 路由或审批串实例：Host 测试从 manifest 启动真实 stdio fixture binary 并完成 Gateway RPC/事件/四段路由；Codex Provider 二进制测试独立完成 App Server 会话、审批和 interrupt 闭环。
- CodePet 接入漂移：`codepet-host/tests/builtin_provider_integration.rs` 读取三份正式 manifest，在同一个 Plugin Manager/Gateway 启动 SDK 化 Codex、Claude、OpenCode Provider 与各自 fixture，验证 ready、provider list、代表性 conversation 操作和有序 shutdown。Host shutdown 期间会丢弃晚到状态事件但继续读取 control response，避免提前关闭 stdout 造成 Broken pipe。
- Provider 污染桌宠：Tauri mock runtime 使用生产 bridge，断言 Provider 只进入 remote replay/event，companion replay、companion event、pet channel、activity store 与 Desktop adapter spy 不变化；PetApp 静态测试断言只导入 companion client，且不含跨链路同步路径。
- 审批 fail-open/串 session：真实 Provider 二进制测试证明 `additionalPermissions.network` 得到原 id 的 `-32601` 且无 Approval；stop/start 后复用相同 request id 时旧句柄不能批准新进程请求。
- framing/lifecycle：原生 JSONL 流式解码，不设整行或 turn 字节上限；坏行 drain 后只拒绝对应消息，stderr 保留前 64 KiB 并持续 drain。metadata read 后只发一次 full turn page，透传 limit/cursor，默认 20；短页不自动补齐。仅 `kind: tool` 的超大文本在 mapper 生成 item 时截断，每字段 256 KiB，写入 item `_meta.truncations`，不改变 turn/item 数量。SDK Runtime 仍对最终 raw/zstd Frame V1 执行 16 MiB 上限；Remote 对该层超限保持 cursor 按 20→10→5→1 缩页。事件编码拒绝和 RPC timeout 不终止共享 Server；无法路由队列保留最新 4,096 条并诊断丢弃，真正 EOF/I/O 断开仍清理进程。详见 [文本与分页规约](../60-rules/provider-item-text-and-pagination.md)。
- 共享生命周期：vertical fixture 证明 64 个会话共用 PID，A 终态仍保留 B，延迟输出和审批正确路由；两端连接、最后断开、重连新 PID 与 Provider 存活均有测试。
- lifecycle/stdio：延迟 Server initialize 连续五轮 start/stop，stop 返回前 PID 退出；保留 resume/cancel barrier。16 个阻塞 resume 下应用 ping 和 instance.stop 仍有保留通路；49 个饱和请求、EOF 和异步 Broken pipe 验证有界拒绝与清理。
- writer 冲突与未知结果：只有原生 code=-32600、无非空 data、message 精确匹配当前 thread 的 active-writer reject 才映射 conversation_write_conflict。fixture 覆盖近似错误、明确 turn reject、Server crash 和未知结果不自动重发。
- 显式 restart：Host 测试先替换 instance setting，再制造 graceful stop 超时和 force-kill；确认旧进程结束后 replacement 的 RPC 返回新 setting。无法确认旧进程终止时保持 fail closed。
- 配置漂移：Provider 拒绝非绝对 executable 和未知 settings；Tauri 每次只使用 resolver 覆盖该字段。
- 机械生成漂移：运行 `npm run protocol:check` 与 `git diff --check`，不提交 Tauri build 自动改写的 schema。

## 明确限制

- Codex Provider 自身不拥有 Gateway network listener、配对、认证或 event cursor；这些由 Host/Gateway 层管理，连接终止不等于 turn 终态。
- 没有插件进程自动重启、专用 backoff、签名、沙箱或插件市场；这些是明确非目标。
- 只无损支持 command execution 与 file change 的二元审批。permissions、tool user input、MCP elicitation 不广告为可操作审批。
- App Server 不支持在 `thread/start` 设置 title；Provider 明确拒绝该可选字段。
- Project API 仍属于 Codex experimental API；Provider 初始化会请求 experimental API，但只有实际 `project/list` 探测成功才向 Host/Gateway 广告任何 Project 能力。
- model 列表来自当前 共享 Server session 的官方 `model/list`；reasoning control 因 Gateway v1 尚未表达 per-model effort，只能发布所有可见 model 的共同支持交集。
- Server crash 使整个实例不可用；Host 在线时 SDK 后续心跳可重试幂等 instance.start，旧审批与会话仍必须重新校验，不重放 turn/start。
- Gateway v1 `turn.send` 只表示空闲 conversation 的新 turn。compat 不按 Provider 或 harness 身份推断 catalog/selection 形状，也不按 Codex `pluginId` 推断 `canSteer`。
- 旧客户端没有应用心跳会被新版 Host 按 60 秒失联阈值断开，需要配套更新。App 长期挂起会按离线处理。
- compat v0 的 conversation/turn 模型要求 permission 与时间戳，也没有 `waiting-user-input`；v1 无法确认这些字段或状态时 compat 映射明确返回 `compat_data_unrepresentable`，不会补默认值。长期 live subscription 只把这一类错误降级为跳过单个事件并继续读取；无效 cursor、transport/subscription fault 和其他 mapping error 仍终止 bridge，避免一个 `Turn.updatedAt=None` 永久切断后续事件。
- remote 与 Desktop 同名 thread 不做去重、来源排除或状态同步；两条链路在本阶段按独立资源展示和操作。
- 旧版或未知版本不再触发 item 探测与 fallback 分支；若原生 full turns API 不支持或拒绝请求，返回真实业务错误并保持 Server。已覆盖版本不等于验证所有历史版本；巨大非 tool 文本仍可能触发最终 wire frame 超限。
- Provider/Gateway v1 的 `conversation.get` 已支持可选 cursor/limit 和 pageInfo；Remote 当前会拉完所有历史页再交给详情模型，因此解决了单帧上限，但超长会话仍有总传输量和客户端内存压力，后续可把既有“显示更早消息”改为按需请求旧页。
