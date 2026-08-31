# Codex Provider 插件运行时

## 当前结论

远程 Codex 能力已经从 Tauri 进程内直连实现迁到独立二进制 `crates/providers/codepet-provider-codex`。二进制只依赖生成的 `codepet-provider-sdk`，通过 Provider Protocol v1 的 JSON-RPC 2.0 / stdio JSON-lines 与 `codepet-host` 通信；它不依赖 `codepet-host`、Tauri、Pet SDK 或 Desktop 私有 IPC。

Host 是唯一进程与路由所有者：`PluginManager` 从 manifest 启动 Provider、创建并启动实例，`ProviderGatewayService` 把 Gateway v1 请求路由到实例，并把 Provider 事件变成可 replay 的远程事件。兼容 v0 的 Tauri command/event 只是一层 Gateway DTO 适配，不再拥有或启动 Codex App Server。

## 目标

- 以普通 Provider manifest/binary 接入 Codex，不保留进程内兼容实现或隐式 fallback。
- Provider 内只保留官方 Codex App Server client、标准协议 mapper、实例状态和 stdio 主循环。
- 保留 Desktop Companion 的实时桌宠链路，并从源头禁止 Provider 数据进入 Pet 投影。
- 让设备、插件、实例与原生资源身份在请求、响应、事件和审批回写中保持可追踪。

## 非目标

- 不实现 dylib ABI、插件市场、签名、沙箱、自动重启/backoff 或第二套运行时。
- 不把 Desktop 私有 IPC 放入 Provider，也不让桌宠动作调用 Plugin Manager。
- 不实现 Gateway v1 的 WebSocket/P2P/relay listener；当前 Tauri 入口仍是进程内兼容调用面。
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
| 可广告的 model/reasoning effort | manifest instance settings | 可选 `models`、`reasoningEfforts`；Provider 不猜测或硬编码列表。 |

应用启动和 runtime set/clear/refresh 都使用同一个 resolver 结果更新 Codex instance setting，然后显式重启该 Provider 插件。Desktop IPC connection、owner、revision 和 companion projection 不受影响。没有 resolver 结果时 `appServerExecutable` 缺失，实例 create 明确失败；不会搜索用户目录、调用 Desktop IPC 或启动备用 App Server。

## Provider v1 覆盖矩阵

| Provider v1 方法 | Codex 映射 | 状态与边界 |
| --- | --- | --- |
| `provider.initialize` | 校验版本与 Host identity，绑定单一 device/client | 支持；同一进程不能改绑另一 Host。 |
| `provider.describe` | 返回 `dev.codepet.codex`、版本与 `codex` instance kind | 支持。 |
| `instance.create` | 解码 Host 注入的 settings，建立实例状态 | 支持；executable 必须是绝对路径，未知字段失败。 |
| `instance.start` | 启动 App Server、`initialize`、`initialized`，启动长期 reader | 支持；无 fallback。 |
| `instance.stop` | 关闭同一 App Server session/process | 支持且幂等。 |
| `instance.destroy` | 删除已停止实例 | 支持；运行中明确拒绝。 |
| `instance.capabilities` | 返回当前真实 method/permission/model/reasoning 列表 | 支持。 |
| `conversation.list` | `thread/list` | 支持 cursor/limit；固定 `updated_at desc` 且 `useStateDbOnly=true`。 |
| `conversation.search` | `thread/list(searchTerm)` | 支持 route-scoped cursor/limit；与 list 相同固定 `updated_at desc` 和 state DB only，不做跨 Provider 聚合或本地过滤。 |
| `conversation.get` | `thread/read(includeTurns=true)` | 支持。 |
| `conversation.create` | `thread/start` | 支持 permission/model/reasoning/workspace；App Server 不支持 title 或 Provider extension，传入时明确返回 `capability_unsupported`。 |
| `turn.start` | 必要时 `thread/resume`，再 `turn/start` | 支持，保留 `clientUserMessageId`。 |
| `turn.steer` | 必要时 `thread/resume`，再 `turn/steer`，随后 `thread/read` | 支持；请求显式携带 conversation 与 turn 四段身份，响应使用权威 turn 状态。 |
| `turn.interrupt` | 必要时 `thread/resume`，再 `turn/interrupt`，随后 `thread/read` | 支持；不根据 interrupt ack 伪造完整 Turn。 |
| `approval.resolve` | 对原 server request id 回写 command/file decision | 仅支持普通 accept/decline 二元审批；只处理当前实例、当前 App Server session generation 的 pending approval。 |
| `provider.shutdown` | 关闭全部实例的 App Server session，结束 stdio 主循环 | 支持且幂等。 |

Gateway v1 的 `turn.send` 在 Host 中映射为 Provider `turn.start`；带 `steerTurn` 时映射为 `turn.steer`。兼容 v0 继续暴露既有 `turn.send` 调用形状，但只调用同一个 Gateway service。

Provider 发送全部六种 v1 事件：`event.instanceStatusChanged`、`event.conversationUpserted`、`event.turnUpserted`、`event.turnOutputDelta`、`event.approvalRequested`、`event.approvalResolved`。delta 自带 conversation route，不依赖 replay 顺序补状态。App Server 的 `waitingOnApproval` 与 `waitingOnUserInput` 分别映射为 v1 的 `waiting-approval` 与 `waiting-user-input`。未知 notification 被忽略；未知或无法无损表达的 server request 使用原 request id 返回上游 error `-32601`，不会发布可批准的 Approval。

## 身份与审批路由

Provider/Gateway 的语言中立 `RoutedResourceId` 是 `deviceId + providerPluginId + providerInstanceId + nativeResourceId` 四段身份。instance route 是前三段。SDK、Host、Gateway、Provider、compat extension 在每个请求、响应、事件和审批入口逐跳保留并校验四段；compat 不再通过 registry 事后回查 plugin id。旧 v0 对象自身的 `id` 继续等于 `nativeResourceId`，完整身份位于 `codepet.gateway.route` extension。

每个 Provider instance 只持有一个 App Server session 和一张 pending approval map。每次 session 启动生成唯一 generation；approval 的 `nativeResourceId` 同时编码 generation 与原 App Server request id。`approval.resolve` 先校验四段 route，再校验 generation 和 pending 记录，最后回写持有请求的同一 App Server 进程。即使新进程复用了相同 request id，旧 approval 也以 `stale_approval_session` 失败。

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

- App Server 协议漂移：本机 `codex-cli 0.151.0-alpha.7.1` schema 和 smoke 证明上游 response/notification 不要求 `jsonrpc`，request/notification 的 `params` 可缺失，并允许 `trace`/`emittedAtMs` 元数据；fixture 与 typed DTO 覆盖 initialize、Thread、Turn、start/resume、approval 和 request fail-closed。Provider Protocol 自己仍严格使用 JSON-RPC 2.0。
- 路由或审批串实例：Host 测试从 manifest 启动真实 stdio fixture binary 并完成 Gateway RPC/事件/四段路由；Codex Provider 二进制测试独立完成 App Server 会话、审批和 interrupt 闭环。
- Provider 污染桌宠：Tauri mock runtime 使用生产 bridge，断言 Provider 只进入 remote replay/event，companion replay、companion event、pet channel、activity store 与 Desktop adapter spy 不变化；PetApp 静态测试断言只导入 companion client，且不含跨链路同步路径。
- 审批 fail-open/串 session：真实 Provider 二进制测试证明 `additionalPermissions.network` 得到原 id 的 `-32601` 且无 Approval；stop/start 后复用相同 request id 时旧句柄不能批准新进程请求。
- framing/lifecycle：App Server stdout/stderr 与 Provider SDK 都完整 drain 超长物理行后 fail-stop。stdout/stderr fault 复用同一个 session terminal path，清空 pending 并终止 App Server；terminal fault 与 subscribers 在同一把锁下完成“读已有 fault / 注册未来 fault”，barrier 竞态测试与 late-subscriber 测试分别覆盖并发和已有 fault，实例不会误报 Ready。
- 显式 restart：Host 测试先替换 instance setting，再制造 graceful stop 超时和 force-kill；确认旧进程结束后 replacement 的 RPC 返回新 setting。无法确认旧进程终止时保持 fail closed。
- 配置漂移：Provider 拒绝非绝对 executable 和未知 settings；Tauri 每次只使用 resolver 覆盖该字段。
- 机械生成漂移：运行 `npm run protocol:check` 与 `git diff --check`，不提交 Tauri build 自动改写的 schema。

## 明确限制

- 没有 Gateway v1 网络 listener、配对、认证或持久 event cursor。
- 没有自动重启、backoff、签名、沙箱或插件市场；这些是明确非目标。
- 只无损支持 command execution 与 file change 的二元审批。permissions、tool user input、MCP elicitation 不广告为可操作审批。
- App Server 不支持在 `thread/start` 设置 title；Provider 明确拒绝该可选字段。
- model/reasoning effort 列表来自显式 instance settings，尚未从 App Server 动态发现。
- Gateway v1 capability 目前把 start/steer 合并为 `turn.send`，compat 层只能根据 Codex plugin identity 表达 `canSteer`；v1 schema 尚无独立 steer flag。
- compat v0 的 conversation/turn 模型要求 permission 与时间戳，也没有 `waiting-user-input`；v1 无法确认这些字段或状态时 compat 明确返回 `compat_data_unrepresentable`，不会补默认值。
- remote 与 Desktop 同名 thread 不做去重、来源排除或状态同步；两条链路在本阶段按独立资源展示和操作。
