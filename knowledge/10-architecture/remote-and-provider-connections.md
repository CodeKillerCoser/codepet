# Remote 与 Provider 连接状态和 Harness 生命周期

## 背景

会话详情原先每 10 秒续租，Provider 为每个 Codex 会话启动 writer 进程。页面退出停止续租后，空闲 writer 会被关闭；这种页面生命周期无法代表连接仍在线、用户仍需接收任务变化的状态。2026-09-05 确认改为连接级管理，并将 Host 的 Remote 接入与 Provider IPC 分开归组。

## 目标

- 支持 Server 模式的 Harness，每个 Provider **运行实例**只有一个 Server，多会话共用。插件可以配置多个实例，实例间继续隔离。
- 两段应用心跳传播真实在线状态；任一已认证客户端仍连接时，保留该实例 resume 过的会话和事件。
- 最后一个客户端离线或 Host 心跳过期时停止 Harness；Provider 插件进程继续存在并接收下一次心跳。
- Host 连接页和 Remote Provider 位置同时显示连接健康与 Harness 状态。

## 非目标

不新增万能 connect 管理器，不把 stdio 定义成 Remote channel，不实现桌面会话客户端、relay 或跨 Host 的写入锁协调。未来桌面 client 与移动 client 使用同一 Gateway；连接本机也通过 LAN 的回环地址。

## 现状理解

Host 目录以责任划分：

| 模块 | 权威状态与职责 |
| --- | --- |
| `crates/codepet-host/src/remote/access.rs` | 设备准入、配对、凭据及撤销 |
| `remote/connections.rs` | 已完成认证与 handshake 的连接集合；每条连接独立 ID，快照带 revision |
| `remote/channels/lan/` | TLS HTTPS/WSS、mDNS、网络地址、现有容量与背压控制 |
| `providers/` | catalog、实例 registry、PluginManager、Provider 进程与 stdio IPC |
| `providers/manager/heartbeat.rs` | 将 Remote 只读快照送给 Provider，维护本代插件连接健康 |
| `gateway.rs` | 业务路由、事件投影，以及向客户端提供 Provider 摘要 |

Tauri 的 `RemoteAccessRuntime` 仍是平台组合入口，负责把监听、网络事件、配对和退出流程接到 App；它不成为第二份 Provider 状态源。根模块保留公开 re-export，内部文件按上述目录归组。

## 实现路径

```text
Client -- protocol.ping(sequence) --> Host Remote 接入
Client <-- pong(sequence, ProviderSummary[]) -- Gateway 读取 Provider 状态

Host Provider 管理 -- provider.ping(sequence, hostSessionId, clients, instances) --> Provider SDK
Host Provider 管理 <-- pong(sequence, clientsRevision) -- Provider SDK
                                                      |
                                                      +-- 协调 instance.start/stop --> Harness Server
```

两段均每 20 秒交换，失联阈值 60 秒。Remote 请求超时 10 秒，持续未得到有效 pong 达阈值后进入既有重连流程；Host 的应用心跳超时关闭 socket。WebSocket 原生 Ping/Pong 保留，不能替代携带业务状态的应用心跳。连接增减或实例状态变化会唤醒 Host→Provider 的下一次交换，不等完整周期。应用 ping 绕过普通 RPC 排队，所以慢 get/resume 不阻塞其处理。

Provider SDK 立即确认有效 ping，另用每实例独立协调任务调用幂等 start/stop；启动等待不延迟 pong。无客户端的首次快照、最后连接离开和 Host 心跳过期都会停止实例。启动过程中离线，先取消等待再调用 stop；adapter 必须用 generation/cancellation 保证不出现晚到 Ready。重复心跳重新协调 adapter 状态，故 Server 故障后，只要客户端仍在线，后续心跳可再次尝试启动；不自动重发失败的业务请求。

`hostSessionId` 绑定当前插件进程；sequence 必须递增，client revision 不倒退，相同 revision 不得改变集合。旧包既不能改在线集合，也不能延长存活时间。Host 收到 pong 后复核插件 generation 与进程 identity；Remote 以连接 generation 和 Provider generation 丢弃旧响应。心跳摘要使用独立状态流，不伪装成有 cursor 的 replay event；同一次 ping 期间已发生的 provider.changed 优先于晚到摘要。

Provider `connectionStatus` 为 connecting/online/offline，表示 Host↔插件通信；实例原有 status 表示 Harness 是否 Ready。因此“Provider 在线，Harness 已停止”是无客户端时的正常状态。摘要沿用 capability revision，完整能力仍由 describe 获取；Remote 不把摘要的空 methods 当作不支持，能力到达或实例恢复 Ready 后补拉项目。

Codex 的一个 Server 负责 list/get/create/resume/turn/approval，一条 reader 按 thread ID 分发，多会话仍各有 operation lock。退出详情不 unsubscribe、不续租、不停止 Server；最后一条连接离开会关闭整个 Server，**包括正在运行的任务**。stdin EOF、Provider shutdown 同样清理 Server。两部手机经同一 Host 操作同一会话共用该 Server；并发 start 仍受原生 turn 状态和每会话串行控制，不能承诺同时启动两个 turn。

## 涉及模块

- `protocol/{core,provider,gateway}` 与 `sdk/`：连接快照、ping/pong、健康摘要，以及两端心跳驱动；`cp-sdk-gen` 随导出 SDK 分发手写 runtime 文件。
- Host `remote/`、`providers/`、Gateway：分别管理连接与插件状态，保留 LAN 安全和网络行为。
- Codex `provider.rs`、`provider/server_events.rs`：共享进程、跨会话事件与审批隔离；其他 adapter 复用 SDK 协调生命周期，非 Server Harness 不强制改为 Server。
- Tauri bridge / `ProviderConnectionStatus.svelte`：订阅同一 PluginManager 状态；Remote `ProtocolGatewayClient` / `DeviceSession`：心跳、摘要合并和恢复加载。

## 风险

- 旧客户端不发送应用 ping，60 秒后会被新版 Host 断开：Host/Remote/Provider SDK 必须配套更新；当前没有旧心跳兼容协商。
- 多端误删在线状态：以连接 ID 管理 RAII 注册，断开 A 不删除 B；由 Host presence 和 Provider binary 测试覆盖。
- 慢请求阻塞心跳或退出：LAN pending-get 与 Provider 16 个阻塞 RPC 测试覆盖；SDK 的 EOF 清理先停止心跳协调，再关闭 adapter。
- 共享 Server 故障影响全部会话：以 instance Error 明确暴露，并使审批 generation 失效；恢复需要重新 acquire，业务请求不自动重放。
- 窄屏状态文案挤压动作：Remote widget 测试、Host 前端构建及实际安装后的连接页检查。

## 测试计划

运行协议 freshness、SDK Rust/Dart 测试、导出 SDK 独立编译、Host LAN/manager 集成测试、Codex vertical、Remote 全量测试和 analyze。重点覆盖两个连接、最后断开、重新连接、长任务事件、审批隔离、旧 generation、心跳在 RPC 饱和时仍响应、Provider Ready 后项目补拉。真实手机/安装版的应用挂起和网络切换仍需端到端验收。

## 验证结果（2026-09-05）

- `cargo test --manifest-path crates/Cargo.toml -- --test-threads=1`：192 项通过，2 项需要外部真实 runtime 的测试按原约定忽略。首轮并行执行的 process FD 计数测试受同进程其他测试干扰，串行完整复跑通过。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test builtin_provider_integration`：三个正式 manifest + SDK binary + native fixture，覆盖无客户端、两端连接、最后断开、插件继续在线及重连。
- `cargo test --manifest-path sdk/rust/Cargo.toml -p codepet-provider-sdk -p codepet-gateway-sdk`：31 项通过；Gateway Dart SDK `dart test`：10 项通过。
- OpenCode 新增启动取消回归后，`cargo test --manifest-path crates/Cargo.toml -p codepet-provider-opencode` 全部通过；覆盖健康检查前及模型发现期间取消，无残留 PID 或晚到 Ready。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib runtime_gateway::tauri_bridge::tests`：6 项通过；Tauri `cargo check` 通过。
- Remote `flutter test`：260 项通过；`flutter analyze` 无问题。含 320px Provider 状态布局和断开动作。
- `npx vitest run frontend`：191 项通过；`npm run build`、`npm run protocol:check`（17 项）、`npm run sdkgen:test`（3 项）通过。直接对仓库运行 Vitest 会误收集 Node/Bun runner 的测试，因此按前端范围运行，生成器使用自己的 runner。
- 从 canonical protocol 导出独立 Rust Provider server / Gateway server SDK，两份导出目录分别 `cargo check` 通过；两仓库 `git diff --check` 通过。
- 未覆盖安装到正在运行的桌面 App 和 Android 模拟器，真实后台挂起/网络切换仍为安装验收项；未执行代码格式化工具。

## 知识沉淀

Host Runtime 卡片通过 `frontend/lib/providerRuntimes.ts` 统一消费连接状态：Provider 生命周期变化触发安装列表重读，实例状态变化只更新 Harness 标签。初始订阅和异步查询有时序保护，正常初始化显示 loading；证据与验收见 `../40-runbooks/host-provider-startup-stale-runtime.md`。

本架构替代按会话租约和 observer/writer 进程划分；对应规则见 `../60-rules/codex-provider-turn-writer-lifecycle.md` 与 `../60-rules/codex-provider-instance-session-lifecycle.md`。历史尺寸问题与原生 unsubscribe 探针证据保留在 `../40-runbooks/codex-detail-size-and-capability-loading.md`。

## 未知项

未覆盖所有手机系统的后台保活策略。App 被系统挂起、应用心跳超时后按离线处理；持续后台执行到任务完成不属于当前连接生命周期承诺。
