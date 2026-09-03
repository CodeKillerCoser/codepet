# Codex Provider 插件运行时

## 当前结论

远程 Codex 能力已经从 Tauri 进程内直连实现迁到独立二进制 `crates/providers/codepet-provider-codex`。二进制只依赖 `codepet-provider-sdk`，通过 Provider Protocol v1 的 JSON-RPC 2.0 / stdio JSON-lines 与 `codepet-host` 通信；它不依赖 `codepet-host`、Tauri、Pet SDK 或 Desktop 私有 IPC。Provider 入口只构造 `CodexProvider` 并调用 SDK 的 `serve_stdio`；reader、writer、dispatcher、event sink、普通/控制双通路、过载、frame limit 与 terminal cleanup 全部属于公共 SDK runtime，Codex crate 不再复制 transport server。

每个 Provider instance 长期持有一个纯读 observer App Server；`conversation.create` 独立启动 App Server 执行 `thread/start`，成功后把同一 session 转移为该 conversation 的临时 execution slot，首条消息直接 `turn/start`，不等待 list/get/read 物化，也不额外 `thread/resume`。历史 conversation 的 writer 仍按 conversation 独立 resume。observer 初始化后调用 `model/list`，并用 `project/list(limit=1)` 探测实验 Project API：成功才整组广告五个 Project CRUD 能力，`-32601` 则不广告，其他错误使 instance start fail closed；Codex 0.151/0.152 的 Section 与 permission profile 不映射到 Host/Remote。Gateway/Provider 通过幂等 `conversation.acquireInteraction` 建立或续租交互权，30 秒租期内复用同一 writer；无 active turn 且租期过期后退出。observer 与 create 从 spawn 前占位开始进入同一 generation/cancellation registry，stop/shutdown 只有在占位操作收敛、子进程退出后才发布 stopped。SDK runtime 的普通请求最多并发 16 个、排队 32 个，manifest 标记的 lifecycle control 使用 2 并发/4 排队保留通路；队列满时按原 id 返回 retryable `provider_overloaded`，response 仍允许乱序返回。

Host 是唯一进程与路由所有者：`PluginManager` 从 manifest 启动 Provider、创建并启动实例，`ProviderGatewayService` 把 Gateway v1 请求路由到实例，并把 Provider 事件变成可 replay 的远程事件。兼容 v0 的 Tauri command/event 只是一层 Gateway DTO 适配，不再拥有或启动 Codex App Server。

## 目标

- 以普通 Provider manifest/binary 接入 Codex，不保留进程内兼容实现或隐式 fallback。
- Provider 内只保留官方 Codex App Server client、标准协议 mapper、实例状态和 stdio 主循环。
- 保留 Desktop Companion 的实时桌宠链路，并从源头禁止 Provider 数据进入 Pet 投影。
- 让设备、插件、实例与原生资源身份在请求、响应、事件和审批回写中保持可追踪。

## 非目标

- 不实现 dylib ABI、插件市场、签名、沙箱、自动重启/backoff 或第二套运行时。
- 不把 Desktop 私有 IPC 放入 Provider，也不让桌宠动作调用 Plugin Manager。
- 不修改 Gateway LAN/WSS、配对、凭据或 relay；网络连接不拥有 Provider execution 生命周期。
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
| 可广告的 model/reasoning effort | 当前 observer App Server 的官方 `model/list` | model 使用全部可见项；全局 reasoning control 只发布所有已广告 model 的共同支持交集，Provider 不猜测或硬编码列表。 |

应用启动和 runtime set/clear/refresh 都使用同一个 resolver 结果更新 Codex instance setting，然后显式重启该 Provider 插件。Desktop IPC connection、owner、revision 和 companion projection 不受影响。没有 resolver 结果时 `appServerExecutable` 缺失，实例 create 明确失败；不会搜索用户目录、调用 Desktop IPC 或启动备用 App Server。

## Provider v1 覆盖矩阵

| Provider v1 方法 | Codex 映射 | 状态与边界 |
| --- | --- | --- |
| `provider.initialize` | 校验版本与 Host identity，绑定单一 device/client | 支持；同一进程不能改绑另一 Host。 |
| `provider.describe` | 返回 `dev.codepet.codex`、版本与 `codex` instance kind | 支持。 |
| `instance.create` | 解码 Host 注入的 settings，建立实例状态 | 支持；executable 必须是绝对路径，未知字段失败。 |
| `instance.start` | 启动纯读 observer App Server、`initialize`、`initialized`、`model/list`、Project API 探测与长期 reader | 支持；Project 探测只读取 `project/list(limit=1)`；spawn 前登记 generation 占位，spawn 后、initialize 前登记 session；observer 不发送 `thread/start`、`thread/resume` 或 turn/approval 写操作，无 fallback。 |
| `instance.stop` | 关闭 observer、一次性 create 与全部 conversation 执行 App Server | 支持且幂等；正在 spawn/initialize/resume 的槽会被取消并唤醒等待者，全部 PID 退出后才返回 stopped。 |
| `instance.destroy` | 删除已停止实例 | 支持；运行中明确拒绝。 |
| `instance.capabilities` | 返回当前真实 method/permission/model/reasoning 列表 | 支持。 |
| `project.list/get/create/update/delete` | observer `project/list|read|create|update|delete` | 仅在启动探测成功时整组支持；Project 保留 routed identity、roots、typed string metadata、只读 position 与时间戳。 |
| `conversation.list` | observer `thread/list` | 支持 cursor/limit 和 all/standalone/project 三态筛选；分别省略 `projectId`、发送 null、发送项目 native id。固定 `updated_at desc` 且 `useStateDbOnly=true`。 |
| `conversation.search` | observer `thread/list(searchTerm)` | 支持 route-scoped cursor/limit；与 list 相同固定 `updated_at desc` 和 state DB only，不做跨 Provider 聚合或本地过滤。 |
| `conversation.get` | observer `thread/read(includeTurns=false)` + cursor `thread/turns/list(itemsView=full, sortDirection=desc)` | 支持 cursor/limit；limit 是最多返回的原生 turn 数，Provider 用最多 10-turn 的上游页读取请求范围，恢复为时间正序后返回准确 nextCursor。严格纯读，不创建执行槽、不调用 `thread/resume`，也不改变 loaded-thread 状态。 |
| `conversation.acquireInteraction` | conversation 执行 App Server `thread/resume`，或对当前执行槽续租 | 支持且幂等；返回 resume 得到的真实 permission/model/reasoning selection 与 30 秒租期。Claude/OpenCode 等无 writer lock 的 Provider 可返回成功与当前/空 selection，不执行 resume。 |
| `conversation.create` | 独立 App Server `thread/start`，成功 session 转移到 execution slot | 支持 permission/model/reasoning/workspace 和可选显式 project；项目同路由且能力探测成功时映射 `projectId`，省略即 standalone，不根据 cwd 推断。从 spawn 前就在 instance registry 可见，最终 `thread/start` 写入与 cancel 通过短门线性化。response 立即可供首条 `turn.start`，不等待 observer 物化；session 受 30 秒临时租期和 active turn 管理。App Server 不支持 title 或 Provider extension，传入时明确返回 `capability_unsupported`。 |
| `turn.start` | conversation 执行 App Server `thread/resume`、权威 `thread/read`、`turn/start` | 支持，保留 `clientUserMessageId`；同 conversation 并发首次请求共享一个创建槽，只发送一次 resume。 |
| `turn.steer` | 同一 conversation 执行 App Server `turn/steer`，随后 `thread/read` | 支持；请求显式携带 conversation 与 turn 四段身份，响应使用权威 turn 状态。 |
| `turn.interrupt` | 同一 conversation 执行 App Server `turn/interrupt`，随后 `thread/read` | 支持；不根据 interrupt ack 伪造完整 Turn。权威 terminal snapshot 会清除 active turn；仍有有效交互租约时保留 writer，否则退出。 |
| `approval.resolve` | 对 owning execution session 的原 server request id 回写 command/file decision | 仅支持普通 accept/decline 二元审批；generation 与 pending approval 必须同时匹配，不能换进程回写。 |
| `provider.shutdown` | 关闭全部实例的 observer、一次性 create 与执行 App Server，结束 stdio 主循环 | 支持且幂等；并发 shutdown 等待同一清理完成，即使某个 shutdown 失败也会先尝试关闭其他 session。stdio EOF、坏帧、response/event stdout 写失败同样先调用该清理，再在 2 秒 drain 边界后中止未收敛的 dispatch task。 |

Gateway v1 的 `turn.send` 只对已有空闲 conversation 启动新 turn，并在 Host 中映射为 Provider `turn.start`。Provider v1 的 `turn.steer` 仍是独立内部能力，不由 Gateway `turn.send` 自动选择。兼容 v0 继续暴露既有 `turn.send` 调用形状，但 `canSteer=false`，也不会根据 Codex `pluginId` 推断 steering 能力。

Provider 发送全部八种 v1 事件：`event.instanceStatusChanged`、`event.projectChanged`、`event.conversationUpserted`、`event.conversationItemUpserted`、`event.turnUpserted`、`event.turnOutputDelta`、`event.approvalRequested`、`event.approvalResolved`。Project change 只由 observer 映射，避免 writer session 重复发布；delta 自带 conversation route，不依赖 replay 顺序补状态；item upsert 用于提交工具调用的结构化状态和结果。App Server 的 `waitingOnApproval` 与 `waitingOnUserInput` 分别映射为 v1 的 `waiting-approval` 与 `waiting-user-input`。未知 notification 被忽略；未知或无法无损表达的 server request 使用原 request id 返回上游 error `-32601`，不会发布可批准的 Approval。

## 身份与审批路由

Provider/Gateway 的语言中立 `RoutedResourceId` 是 `deviceId + providerPluginId + providerInstanceId + nativeResourceId` 四段身份。instance route 是前三段。SDK、Host、Gateway、Provider、compat extension 在每个请求、响应、事件和审批入口逐跳保留并校验四段；compat 不再通过 registry 事后回查 plugin id。旧 v0 对象自身的 `id` 继续等于 `nativeResourceId`，完整身份位于 `codepet.gateway.route` extension。

Project 使用相同四段身份。Conversation 的 `project` 只映射 App Server `Thread.projectId`，而 `workspaceRoot` 原样映射 `Thread.cwd`；Provider 不再扫描 Git metadata、归并 worktree 或用 cwd 推断项目。项目筛选和项目归属创建都在 Provider 与 Host 两层校验 route。

每个 Provider instance 持有一个 observer 和按 conversation 索引的执行槽。执行槽以 `Creating → Ready → Closing → Closed` 管理，创建失败则进入 `Failed` 并从 map 淘汰；并发等待者共享同一创建结果。槽从插入 map 起就有独立 attempt generation 与 cancellation 状态；子进程完成 spawn、尚未 initialize 前即登记到 Creating，因此 stop 能关闭其 stdin/进程并唤醒 initialize。initialize 返回与 subscribe 后都会重新核对 instance、slot、attempt/session generation。首次 `thread/resume` 的实际 request 写入与 cancel 共用一把只覆盖“复核 + 写 frame”的短锁：cancel 先线性化则不再写 resume/start；resume 写入先线性化则 stop 随后关闭 session，但不等待悬挂 response。Ready 槽关联一个 App Server generation、当前 active turn 与可选交互租期；不同 conversation 不共享进程、operation lock、租期或 loaded-thread 状态。

observer 与 `conversation.create` 共用 instance session lifecycle：请求先在当前 instance generation 插入 `Pending` 槽，再进入 `Spawning`；子进程一旦 spawn 就登记为 `Spawned`，initialize/model discovery 在任何长锁之外执行。stop/fail/shutdown 先推进 instance generation、从 registry 取走全部槽，再等待 `Spawning` 结束、关闭已登记 session，最后发布 stopped。槽注销必须同时匹配 id、generation 与 `Arc` identity，旧异步完成不能删除新一代 session。create 的最终 `thread/start` 与 cancel 共用只覆盖“当前性复核 + frame 写入”的 send gate；cancel 先取得门后不允许再写 start。

每次执行 App Server 启动生成唯一 generation；approval 的 `nativeResourceId` 同时编码 generation 与原 App Server request id。`approval.resolve` 先校验四段 route，再校验 generation、pending 记录与 owning conversation，最后回写持有请求的同一进程。即使新进程复用了相同 request id，旧 approval 也以 `stale_approval_session` 失败。执行进程异常退出或 turn 终态时，尚未解决的 approval 会过期，历史 ledger 可继续被 `conversation.get` 投影，但不再保留进程引用。

执行槽由 Provider 的交互租期与 active turn 共同所有，不由 event subscription 或 WSS connection 直接所有。Remote 详情页每 10 秒幂等 acquire，Provider 每次把租期延长到 30 秒；页面退出、进程挂起或断网只会停止续租，不发送显式 release。无 active turn 时租期到期关闭 writer；`running`、`waiting-approval`、`waiting-user-input` 即使租期到期也继续保留执行进程，直到 turn 终态后再关闭。所有 writer route 在取得 conversation operation lock 后统一复核 map 仍指向同一 Ready generation；若请求此前预取的 handle 已变为 Closing/Closed，就丢弃旧 handle、释放旧锁并等待或取得新 generation，绝不向旧 session 写。`turn/completed` 携带 `completed/failed/interrupted` 时清除 active turn：租约有效则发布事件后保持 Ready，租约缺失/过期才切 Closing、关闭子进程并移除同一槽。App Server crash、initialize/resume 失败、明确 RPC reject 与 `SentOutcomeUnknown` 仍立即关闭并移除对应槽，不自动重放 `turn/start`。

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

- App Server 协议漂移：本机 `codex-cli 0.151.0` 与官方 npm `0.152.0` schema/binary smoke 证明上游 response/notification 不要求 `jsonrpc`，request/notification 的 `params` 可缺失，并允许 `trace`/`emittedAtMs` 元数据；实际故障环境的 `0.151.0-alpha.7.2` 证明 `thread/turns/list` 可分页且 `thread/items/list` 返回 `-32601`。0.151 先 probe item pagination，缺失时回退 full turns；0.152 使用 `notLoaded` turns 加逐 item 分页，Section 不映射成 Project。fixture 与 typed DTO 覆盖 initialize、Thread、Turn、start/resume、approval 和 request fail-closed。Provider Protocol 自己仍严格使用 JSON-RPC 2.0。
- Project API 漂移：官方 0.151/0.152 schema 提供 Thread Section，而不是 Provider Project CRUD；因此实际 `project/list` probe 返回 `-32601` 时不广告任何 Project 能力，Section 也不会被适配成 Project。typed client 与 Provider fixture 仍覆盖可选上游实验 Project surface 的精确方法/参数、DTO、能力整组探测、CRUD、筛选、项目归属创建和 `-32601` fail-closed，供确实实现该 API 的 harness 使用。
- 路由或审批串实例：Host 测试从 manifest 启动真实 stdio fixture binary 并完成 Gateway RPC/事件/四段路由；Codex Provider 二进制测试独立完成 App Server 会话、审批和 interrupt 闭环。
- CodePet 接入漂移：`codepet-host/tests/builtin_provider_integration.rs` 读取三份正式 manifest，在同一个 Plugin Manager/Gateway 启动 SDK 化 Codex、Claude、OpenCode Provider 与各自 fixture，验证 ready、provider list、代表性 conversation 操作和有序 shutdown。Host shutdown 期间会丢弃晚到状态事件但继续读取 control response，避免提前关闭 stdout 造成 Broken pipe。
- Provider 污染桌宠：Tauri mock runtime 使用生产 bridge，断言 Provider 只进入 remote replay/event，companion replay、companion event、pet channel、activity store 与 Desktop adapter spy 不变化；PetApp 静态测试断言只导入 companion client，且不含跨链路同步路径。
- 审批 fail-open/串 session：真实 Provider 二进制测试证明 `additionalPermissions.network` 得到原 id 的 `-32601` 且无 Approval；stop/start 后复用相同 request id 时旧句柄不能批准新进程请求。
- framing/lifecycle：App Server stdout/stderr 与 Provider SDK 都完整 drain 超长物理行；observer 历史通过 metadata read 和最多 10-turn 的上游页读取客户端请求的有限范围。0.152 先读 `notLoaded` turn 元数据，再以 `thread/items/list(limit=1)` 逐 item 释放原生 DTO；原生 item 读取失败把当前及剩余 turn 降为稳定 unknown placeholder。Provider→Host 最终序列化超过共享 16 MiB 时先按原 id 返回 non-retryable `provider_response_too_large`；Remote 保持原 cursor 并缩至 limit=1 后，三个内置 Provider 共用序列化尺寸检查，把正文截至 32 KiB/item，仍超限则折叠为 History omitted，占位仍保留 `nextCursor`。分页顺序/稳定 ID/0.151 fallback/0.152 item path、缩页成功与超限后继续服务均有 fixture 覆盖。
- writer 生命周期：真实 Provider 二进制 fixture 以每 PID 独立日志证明 observer 只做 model/list、thread list、metadata read 与 turns list；首次 acquire 单独 resume，重复 acquire 只续租并返回真实 selection；租约内的 terminal 后新 turn 复用同一 PID。未 acquire 的兼容写路径仍在 terminal 后退出，既有 handle/terminal barrier 与 64 轮压力测试继续覆盖 Closing 竞态；initialize 后的 resume barrier 证明 cancel 先线性化时 fixture 没有 resume/start 记录。
- lifecycle/stdio 回收：延迟 one-shot initialize 后并发 stop 与 stdin EOF 都在 2 秒内终止请求，且无 `thread/start`；延迟 observer initialize 连续五轮 start/stop 时，每个 stop 响应前 PID 已退出、start 以原 id 返回错误且没有晚到 Ready。生产 Provider binary 另令 16 个 `thread/resume` 同时永不响应，保留通路上的 `instance.stop` 仍在 2 秒内响应并回收全部 PID；16 active + 32 pending 后第 49 个普通请求按 id 得到 `provider_overloaded` 且未进入 App Server。关闭 stdout 后触发异步 instance event，event 写失败会无锁地只发送一次主循环 terminal 信号，全局 shutdown 回收 active PID 后退出。
- writer 冲突与未知发送结果：只有 `thread/resume` 的明确 RPC reject 同时满足 `code=-32600`、无非空 `data`，且 message 精确等于 `thread <当前 conversation id> already has an active writer` 时，才返回 `conversation_write_conflict`、`retryable=true`、`operation=thread/resume`、`reason=owned-by-other-runtime`。wrong code、近似 message、其他 thread id 与非空 data 都保留普通 Provider error。fixture 另行覆盖 initialize failure、明确 turn reject、App Server crash 和 sent-outcome-unknown，断言失败槽无残留且 Provider 不自动重试 `turn/start`。
- 显式 restart：Host 测试先替换 instance setting，再制造 graceful stop 超时和 force-kill；确认旧进程结束后 replacement 的 RPC 返回新 setting。无法确认旧进程终止时保持 fail closed。
- 配置漂移：Provider 拒绝非绝对 executable 和未知 settings；Tauri 每次只使用 resolver 覆盖该字段。
- 机械生成漂移：运行 `npm run protocol:check` 与 `git diff --check`，不提交 Tauri build 自动改写的 schema。

## 明确限制

- Codex Provider 自身不拥有 Gateway network listener、配对、认证或 event cursor；这些由 Host/Gateway 层管理，连接终止不等于 turn 终态。
- 没有自动重启、backoff、签名、沙箱或插件市场；这些是明确非目标。
- 只无损支持 command execution 与 file change 的二元审批。permissions、tool user input、MCP elicitation 不广告为可操作审批。
- App Server 不支持在 `thread/start` 设置 title；Provider 明确拒绝该可选字段。
- Project API 仍属于 Codex experimental API；Provider 初始化会请求 experimental API，但只有实际 `project/list` 探测成功才向 Host/Gateway 广告任何 Project 能力。
- model 列表来自当前 observer session 的官方 `model/list`；reasoning control 因 Gateway v1 尚未表达 per-model effort，只能发布所有可见 model 的共同支持交集。
- observer crash 会使当前 Provider instance fail closed；本阶段没有 observer 自动 supervisor/backoff。active execution 会被关闭，需由既有 Provider restart/refresh 路径恢复。
- Gateway v1 `turn.send` 只表示空闲 conversation 的新 turn。compat 不按 Provider 或 harness 身份推断 catalog/selection 形状，也不按 Codex `pluginId` 推断 `canSteer`。
- `conversation.acquireInteraction` 的 30 秒 Provider 租期与 Remote 10 秒续租周期目前是固定常量；没有跨 Provider 持久 lease token，也不承诺 App 被系统长期挂起后仍保有 writer。
- compat v0 的 conversation/turn 模型要求 permission 与时间戳，也没有 `waiting-user-input`；v1 无法确认这些字段或状态时 compat 映射明确返回 `compat_data_unrepresentable`，不会补默认值。长期 live subscription 只把这一类错误降级为跳过单个事件并继续读取；无效 cursor、transport/subscription fault 和其他 mapping error 仍终止 bridge，避免一个 `Turn.updatedAt=None` 永久切断后续事件。
- remote 与 Desktop 同名 thread 不做去重、来源排除或状态同步；两条链路在本阶段按独立资源展示和操作。
- 0.151 环境若 `thread/items/list` 探测为 `-32601`，只能回退 `thread/turns/list(full)`；若该单个原生 turn 自身超过 App Server 16 MiB physical line，reader 会返回 bounded error，Provider 最终只能给该 turn 占位。该 session 的原生 reader 已失效，仍需持续观察实际 0.151 巨型单 item 行为。
- Provider/Gateway v1 的 `conversation.get` 已支持可选 cursor/limit 和 pageInfo；Remote 当前会拉完所有历史页再交给详情模型，因此解决了单帧上限，但超长会话仍有总传输量和客户端内存压力，后续可把既有“显示更早消息”改为按需请求旧页。
