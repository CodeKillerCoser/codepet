# Provider Host、设备身份与进程外插件运行时

## 当前结论

`crates/codepet-host` 已提供可复用的 Rust Provider Host：它从显式目录读取 manifest，为本机持久化稳定 `DeviceId`，按 manifest 启动独立 Provider 二进制，并通过生成的 `codepet-provider-sdk` 在独占 stdio 上通信。它同时实现内部 `codepet-gateway-sdk::ProtocolServer`，但当前没有 Gateway v1 网络 listener。

Tauri 由 `ProviderHostState` 管理 Plugin Manager 与 Gateway service；compat-v0 `RuntimeGatewayState` 只持有同一个 service 的薄适配引用。Provider 数据只有一条远程路径：

```text
Provider binary
  -> PluginProcess / PluginManager
  -> bounded single-consumer Host update queue
  -> ProviderGatewayService replay + subscribers
  -> runtime_gateway_* / runtime-gateway-event
```

Provider 事件进入 Gateway v1 replay，并由兼容适配发布到远程 `runtime-gateway-event`；它们不进入 `SharedState` activity store、Desktop Companion replay、`codex-desktop-companion-event` 或 `pet-event`，也没有失败后回退到桌宠 IPC 的路径。真实 fixture 与 Tauri mock `AppHandle` 测试断言 remote event/replay 收到数据，同时 companion、Pet 与 Desktop adapter spy 保持不变。

## 范围与非目标

本阶段实现：

- 持久设备身份和 manifest 实例的稳定 ID 映射；
- 显式插件目录、进程启动、协议协商、实例启动、能力查询和业务路由；
- 有界 JSON-lines、并发 request id 关联、事件分流、超时、崩溃隔离、stderr 诊断和有界 shutdown；
- 内部 Gateway v1 的 device/provider/capability/conversation/turn/approval 与 event replay 边界。

本阶段不实现：

- OpenCode 或 Claude Provider binary；Codex 已由 `crates/providers/codepet-provider-codex` 以普通 manifest/binary 接入；
- dylib/trait ABI、签名、沙箱、市场、下载、自动重启或 backoff；
- 动态注册插件、动态创建生产实例或 Gateway instance lifecycle；
- Gateway v1 LAN listener、认证、配对或持久 cursor；
- 桌宠 UI、Pet 投影或桌宠动作接入 Provider Manager。

## 配置与持久化

Tauri 以解析后的应用数据目录为根：

- `provider-plugins/`：默认 manifest 目录；
- `settings.providerPlugins.directories`：附加显式目录，绝对路径保持不变，相对路径按应用数据目录解析；
- `provider-host/device-identity.json`：稳定本机设备身份；
- `provider-host/provider-instances.json`：manifest 实例的稳定身份映射。

Catalog 只接受目录，不接受运行时 descriptor 注入。每个目录可包含根 `codepet-provider.json`、子目录中的 `codepet-provider.json`，或 `*.codepet-provider.json`。`read_dir` 的目录错误和逐项读取错误都会形成诊断；不会静默丢弃条目。相对 executable 按 manifest 所在目录解析。

manifest 是插件进程和普通实例设置的配置权威：

```json
{
  "manifestVersion": 1,
  "pluginId": "example-provider",
  "displayName": "Example Provider",
  "executable": "codepet-provider-example",
  "args": [],
  "env": {},
  "enabled": true,
  "instances": [
    {
      "instanceKind": "default",
      "displayName": "Example",
      "settings": {},
      "enabled": true
    }
  ]
}
```

`provider-instances.json` 只镜像 `instanceId + pluginId + instanceKind + displayName`，用于在重启后复用未显式给出的 `instanceId`。`settings` 与 `enabled` 每次都来自当前 manifest，不作为第二配置源；manifest 删除的实例会从映射中 prune。registry 损坏或 device id 不匹配时 fail closed。设备身份损坏时，原文件先隔离为 `.corrupt-<timestamp>`，再生成新的 `device-<uuid>` 并保留诊断；hostname 从不充当稳定 ID。

Codex 的 `appServerExecutable` 是唯一例外：manifest 仍提供 `appServerArgs`，Tauri 在 Catalog 注册前删除 manifest 中的 executable setting，并只注入 `AgentRuntimeService` resolver 返回的绝对路径。runtime set/clear/refresh 通过 Manager 更新同一个实例 setting 并显式重启 Codex 插件；Provider 自身不搜索路径或补默认参数。详见 `codex-provider-plugin-runtime.md`。

`update_app_settings` 区分 `providerPlugins` 缺失和显式空数组：缺失保留现值，`{"providerPlugins":{"directories":[]}}` 才清空目录。前端完整 `AppSettings` 将该字段设为必填。

## 进程与 shutdown

每个 `PluginProcess` 独占一个子进程及其 stdin/stdout/stderr：

- `PluginProcess::spawn` 是同步入口；Manager 在持有插件表写锁并复核 shutdown gate 后完成 spawn，立即把进程写入对应 entry，再释放锁执行 initialize/describe，因此不存在已 spawn 但 shutdown 不可见的窗口；
- writer task 只持有 `Weak<RpcShared>`，不会与共享状态形成强引用环；
- SDK 生成的 `ProtocolRequest::from_method_params` 负责把 typed method/params 构造成 wire request，Host 不枚举 envelope variant；
- pending map 按 request id 关联响应；event/notification 进入唯一有界 inbound consumer，不会被当作 response；
- frame、inbound/outbound queue 和 stderr 历史均有上限；坏帧、超限、EOF 或 backpressure 只终止对应进程；
- 所有 terminal path 都关闭 inbound、完成 pending、结束 writer 并关闭 stdin；进程 monitor 在发布 exit 前有界等待 writer、reader 与 stderr task，因此末尾 stderr 已进入退出快照；
- 进程只保留一个 `shutting_down` 状态，并统一使用配置的 `shutdown_timeout`。正常关闭先发 `provider.shutdown`，关闭 stdin，并允许 Provider 先关闭 stdout、延迟退出；真实 fixture 在 Unix 直接关闭 fd 1，并用独立 marker 证明 EOF 后 shutdown future 仍未完成，直到 200ms 后子进程正常退出；
- malformed、oversized、EOF、crash、timeout、normal shutdown 和 Drop 共用同一张 current-thread Tokio case table，逐项验证 task、`Weak`、FD、pending 与 inbound 回收。

Tauri 的 Exit/ExitRequested 和托盘退出共用 `ProviderHostState::shutdown_once`。它直接以配置的 `shutdown_timeout` 包住 Manager shutdown future；失败或超时取消后，调用绕过进程 shutdown gate 的 `force_kill_all`，逐个等待子进程退出后才标记完成。并发/重复调用都等待同一完成信号并得到相同完成结果。

Manager 在插件表写锁内设置 shutdown gate，之后拒绝新的 plugin start；`start_enabled` 与 shutdown 都使用顺序 loop，不创建可丢弃的 per-plugin lifecycle task。延迟 initialize 回归测试证明 shutdown 后后续 manifest 插件不会 spawn。

## 状态与实例事实源

插件进程状态只有：

```text
stopped -> starting -> ready -> stopped
                 \-> crashed
```

不存在 `Discovered`、`Disabled` 或局部 `Degraded`。插件 enabled 直接读取 manifest；未启用插件保持 `Stopped`，Gateway 按 manifest enabled 映射为 Unavailable。其余 Gateway 状态即时由“插件进程状态 + SDK `ProviderInstance.status`”派生：`starting` 映射 Connecting，`ready` 但实例尚不存在或只有 Created 事实时映射 Unavailable，崩溃映射 Error。实例 create/start/stop/capability response 在同一写锁内替换 SDK instance，并只向 Host update queue 发布一次状态变更；失败不会留下 Host 自造的 Connecting 状态。

实例集合来自 manifest。Manager 启动 enabled 插件后，为每个 enabled manifest instance 执行 create/start；一个实例失败会让该实例保持 Unavailable，但不会阻止同插件的其他 manifest instance 启动。当前没有生产动态创建入口，也不从 Gateway 暴露 lifecycle。

## 路由与事件

远程资源必须完整携带：

```text
deviceId + providerPluginId + providerInstanceId + nativeResourceId
```

四段任一为空即拒绝。registry 先核对 device、plugin、instance 三段实例 route；Provider response/event 再核对完整资源 route。身份保持型 RPC 必须返回与 request 完全相同的资源 ID；`turn.start` 返回 turn 的 conversation 必须等于请求 conversation；Gateway steer/interrupt 都携带原始 conversation，并要求 Provider 返回的 `turn.conversation` 与它完全相等。route-less `conversation.list` 只在没有 Provider cursor 时聚合；带 cursor 直接返回 `aggregate_conversation_cursor_unsupported`，本阶段不定义复合分页。

`providerPluginId` 是 core IDL 中资源 route 的必填字段，不再由 compat 层事后查询 registry 补齐。兼容 v0 输出通过 route extension 原样物化四段资源身份，旧客户端对象自身的 `id` 仍只是 native id。

Manager 到 Gateway 只有一个有界 `mpsc` receiver，且只能领取一次。Gateway 映射后写入单个有界 replay bus；订阅者 lag 会返回显式错误，旧 cursor 超出 replay 窗口会返回 `event_replay_unavailable`。Gateway 为事件分配 `event-<20 位序号>`，service 内严格单调，事件自身始终保留完整 route。

## SDK 边界

Host 只依赖 `codepet-provider-sdk` 和 `codepet-gateway-sdk`，不定义第二套 Provider/Gateway DTO。四个 Rust SDK 都具备 description/authors/repository metadata，不再 `publish = false`；内部 path dependency 同时声明 `version = "0.1.0"`，可用 `cargo package --allow-dirty` 检查包内容。仓库当前没有 LICENSE 文件，因此 manifest 不虚构 license 声明；正式发布前仍需仓库所有者补充许可证决策。

Provider 包只依赖 Provider SDK，不能反向依赖 Host。Host 启动 manifest binary 的纵向测试位于 `codepet-host`；`codepet-provider-codex` 的 all-target/dev dependency graph 不含 Host、Gateway、Tauri 或 Pet SDK。默认 Provider 名称没有在 Host 预埋注册框架；后续 Provider 仍应以普通 manifest/binary 接入。

## 风险与验证证据

- 强引用环或 FD 泄漏：同一 current-thread Tokio runtime 循环制造坏帧，断言 task 计数、`Weak` 与 `/dev/fd` 回到基线。
- request/event 串线：真实 fixture 并发乱序响应并在 response 前发送 event/notification，断言按 id 和通道分类。
- shutdown 误杀：fixture 响应 shutdown 后真实关闭 stdout fd，写 close marker、输出末尾 stderr、等待 200ms 再成功退出；marker 出现时及 100ms 后 future 均必须未完成，最终断言成功 exit 与完整诊断。
- 启停竞态：initialize 延迟 fixture 与两个并发 `shutdown_once` 触发外层 timeout；断言两次调用都等到 force kill 完成、子进程已结束（Unix 额外用 PID 复核），且 shutdown gate 阻止后续 spawn。
- 路由串流：两设备、两实例、错误 device/plugin/instance、空 ID、错误 response identity 和单调 cursor 均有定向测试。
- 状态重复：stop/start lifecycle 后每次只收到一个 ProviderStatusChanged。
- 通道污染：Tauri mock runtime 真实监听三条 Tauri event；fixture 的 Provider event 只增加 remote event/replay，不改变 companion replay、`SharedState` activity、companion/Pet event 或 Desktop adapter spy。
- 机械漂移：验收检查工作树，不提交 `src-tauri/gen/schemas/macOS-schema.json` 或构建产物。

## 验证命令

```sh
cargo test --manifest-path crates/Cargo.toml -p codepet-host --all-targets
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --all-targets
cargo test --manifest-path sdk/rust/Cargo.toml
npm run protocol:check
cargo test --manifest-path src-tauri/Cargo.toml --test runtime_gateway_core_tests --test runtime_gateway_protocol_tests --test settings_tests --test tray_tests
cargo check --manifest-path src-tauri/Cargo.toml --lib
cargo package --manifest-path sdk/rust/Cargo.toml --workspace --allow-dirty
git diff --check
```

## 剩余风险

- Gateway event replay 和 device last-seen 仍为进程内状态，重启恢复未定义。
- manifest 的 executable、args 和 env 是受信任本地配置；签名、权限隔离与资源配额尚未实现。
- 没有自动重启/backoff；故障实例需要显式重启 Host/插件。
- Codex Provider 的 App Server 协议覆盖与明确限制见 `codex-provider-plugin-runtime.md`。
- 正式发布 SDK 前必须补齐仓库许可证决策，并确定 crate 发布顺序。
