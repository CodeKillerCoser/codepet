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
| 保留最小协调 | `CodexThreadScope` | 只记录 remote thread provenance 与瞬时动作 fence，用于阻止同一 thread 被 companion 投影；不传递 Provider payload、session 或 App Server 状态。 |
| 薄适配 | `runtime_gateway/provider_host_compat.rs` | 把既有 v0 UI 调用映射到 `ProviderGatewayService`；不启动进程，不持有 App Server client，不产生第二份事件。 |

## 两条数据链路

远程链路：

```text
Gateway/既有远程 UI
  -> runtime_gateway_* compat 薄适配
  -> ProviderGatewayService
  -> PluginManager / PluginProcess
  -> codepet-provider-codex
  -> 官方 Codex App Server stdio JSON-RPC

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

两条链路没有 payload、registry、event bus、replay、session、owner 或动作路由的交叉。Pet UI 不导入 remote client，不监听 `runtime-gateway-event`；Provider 事件不进入 `SharedState` activity、companion replay 或 `pet-event`。`CodexThreadScope` 只是一道来源排除栅栏，不是第三条数据链路。

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
| `conversation.list` | `thread/list` | 支持 cursor/limit。 |
| `conversation.get` | `thread/read(includeTurns=true)` | 支持。 |
| `conversation.create` | `thread/start` | 支持 permission/model/reasoning/workspace；App Server 不支持 title 或 Provider extension，传入时明确返回 `capability_unsupported`。 |
| `turn.start` | 必要时 `thread/resume`，再 `turn/start` | 支持，保留 `clientUserMessageId`。 |
| `turn.steer` | 必要时 `thread/resume`，再 `turn/steer` | 支持；turn 必须已在同一实例观察并关联到 conversation。 |
| `turn.interrupt` | 必要时 `thread/resume`，再 `turn/interrupt` | 支持。 |
| `approval.resolve` | 对原 server request id 回写 command/file decision | 支持二元 approve/deny；只处理当前实例当前 session 的 pending approval。 |
| `provider.shutdown` | 关闭全部实例的 App Server session，结束 stdio 主循环 | 支持且幂等。 |

Gateway v1 的 `turn.send` 在 Host 中映射为 Provider `turn.start`；带 `steerTurn` 时映射为 `turn.steer`。兼容 v0 继续暴露既有 `turn.send` 调用形状，但只调用同一个 Gateway service。

Provider 发送全部六种 v1 事件：`event.instanceStatusChanged`、`event.conversationUpserted`、`event.turnUpserted`、`event.turnOutputDelta`、`event.approvalRequested`、`event.approvalResolved`。App Server 的 thread/turn/delta 与 command/file approval 有真实映射；未知 notification 被忽略，未知或无法无损表达的 server request 使用原 request id 返回 JSON-RPC `-32601`，不会伪造 grant 或成功事件。

## 身份与审批路由

Provider/Gateway 的资源 route 是 `deviceId + providerInstanceId + nativeResourceId`。`PluginManager` 在接收事件时同时带着产生该事件的 `providerPluginId`，先用 instance registry 校验 plugin 归属，再允许进入 Gateway。Gateway provider 枚举保留 `pluginId`；兼容 v0 的 conversation、turn、delta 和 approval 输出通过 `codepet.gateway.route` extension 显式携带 `deviceId + providerPluginId + providerInstanceId + nativeResourceId`，旧对象自身的 `id` 也继续等于 `nativeResourceId`。provider 枚举没有原生资源 id，因此 extension 只携带前三项。

每个 Provider instance 只持有一个 App Server session 和自己的 pending approval map。`approval.resolve` 先由 routed approval 定位实例，再查找该实例内的原生 request id，并在持有同一 mapper/session 锁的情况下回写；其他实例、旧 session 或未知 approval 都明确失败。

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
- `src-tauri/src/agent/codex_desktop_ipc/`：保持不变，仍是桌宠 Codex 数据源。

## 风险与验证

- App Server 协议漂移：Provider client 单元测试与 fixture 覆盖 initialize、乱序 response、resume、list/read/start/steer/interrupt、delta、approval 和未知 request fail-closed。
- 路由或审批串实例：Host 校验 route/plugin，纵向测试从 manifest 启动真实 Provider binary，并完成 Gateway RPC、事件、审批和 interrupt。
- Provider 污染桌宠：Tauri mock runtime 同时监听 remote、companion、pet 三个 channel，断言 Provider 只进入 remote replay/event，companion replay、activity store 与 Desktop adapter spy 不变化；PetApp 静态测试继续断言只导入 companion client。
- 配置漂移：Provider 拒绝非绝对 executable 和未知 settings；Tauri 每次只使用 resolver 覆盖该字段。
- 机械生成漂移：运行 `npm run protocol:check` 与 `git diff --check`，不提交 Tauri build 自动改写的 schema。

## 明确限制

- 没有 Gateway v1 网络 listener、配对、认证或持久 event cursor。
- 没有自动重启、backoff、签名、沙箱或插件市场；这些是明确非目标。
- 只无损支持 command execution 与 file change 的二元审批。permissions、tool user input、MCP elicitation 不广告为可操作审批。
- App Server 不支持在 `thread/start` 设置 title；Provider 明确拒绝该可选字段。
- model/reasoning effort 列表来自显式 instance settings，尚未从 App Server 动态发现。
- Gateway v1 capability 目前把 start/steer 合并为 `turn.send`，compat 层只能根据 Codex plugin identity 表达 `canSteer`；v1 schema 尚无独立 steer flag。
- Gateway v1 的 output delta 不携带 conversation route；compat 层依赖先前的 turn response/upsert/approval 建立关联。没有任何可观察 turn 上下文的孤立 delta 会 fail closed 丢弃，而不是猜测 conversation。
- Provider Protocol 当前不携带请求“未发送/明确拒绝/结果未知”的细分证据；compat 在 remote create/turn 调用返回歧义错误时采用保守来源隔离，可能暂时或永久排除同一窗口中的 Desktop 候选，而不会冒险把 remote thread 投影进桌宠。
- remote thread provenance 仍是进程内状态；应用重启后的来源恢复尚未定义。
