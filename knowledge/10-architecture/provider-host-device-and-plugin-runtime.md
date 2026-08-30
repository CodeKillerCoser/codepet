# Provider Host、设备身份与进程外插件运行时

## 当前结论

`crates/codepet-host` 已提供可复用的 Rust Provider Host：它从显式目录读取 manifest，为本机持久化稳定 `DeviceId`，按 manifest 启动独立 Provider 二进制，并通过生成的 `codepet-provider-sdk` 在独占 stdio 上通信。它同时实现内部 `codepet-gateway-sdk::ProtocolServer`，但当前没有 Gateway v1 网络 listener。

Tauri 将 `ProviderHostState` 与 compat-v0 `RuntimeGatewayState` 并列管理。Provider 数据只有一条路径：

```text
Provider binary
  -> PluginProcess / PluginManager
  -> bounded single-consumer Host update queue
  -> ProviderGatewayService replay + subscribers
```

Provider 事件不进入 `SharedState` activity store、compat-v0 replay、Desktop Companion replay 或 companion Tauri channel，也没有失败后回退到桌宠 IPC 的路径。真实 fixture 测试会依次触发 Provider event、坏帧和进程崩溃，并固定这些隔离断言。

## 范围与非目标

本阶段实现：

- 持久设备身份和 manifest 实例的稳定 ID 映射；
- 显式插件目录、进程启动、协议协商、实例启动、能力查询和业务路由；
- 有界 JSON-lines、并发 request id 关联、事件分流、超时、崩溃隔离、stderr 诊断和有界 shutdown；
- 内部 Gateway v1 的 device/provider/capability/conversation/turn/approval 与 event replay 边界。

本阶段不实现：

- Codex、OpenCode 或 Claude Provider binary；
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

manifest 是本阶段配置的唯一权威：

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

`update_app_settings` 区分 `providerPlugins` 缺失和显式空数组：缺失保留现值，`{"providerPlugins":{"directories":[]}}` 才清空目录。前端完整 `AppSettings` 将该字段设为必填。

## 进程与 shutdown

每个 `PluginProcess` 独占一个子进程及其 stdin/stdout/stderr：

- writer task 只持有 `Weak<RpcShared>`，不会与共享状态形成强引用环；
- SDK 生成的 `ProtocolRequest::from_method_params` 负责把 typed method/params 构造成 wire request，Host 不枚举 envelope variant；
- pending map 按 request id 关联响应；event/notification 进入唯一有界 inbound consumer，不会被当作 response；
- frame、inbound/outbound queue 和 stderr 历史均有上限；坏帧、超限、EOF 或 backpressure 只终止对应进程；
- 所有 terminal path 都关闭 inbound、完成 pending、结束 writer 并关闭 stdin；进程 monitor 在发布 exit 前有界等待 writer、reader 与 stderr task，因此末尾 stderr 已进入退出快照；
- 进程只保留一个 `shutting_down` 状态，并统一使用配置的 `shutdown_timeout`。正常关闭先发 `provider.shutdown`，关闭 stdin，并允许 Provider 先关闭 stdout、延迟退出；到 deadline 才 kill。

Tauri 的 Exit/ExitRequested 和托盘退出共用 `ProviderHostState::shutdown_once`。第一次调用有界等待全部插件 shutdown；失败或超时后才执行有界 `kill_all`。并发/重复退出请求等待同一次结果，不重复发送 shutdown。

Manager 开始 shutdown 后拒绝新的 plugin start，避免后台启动任务在 Tauri 已完成进程枚举后再生成子进程。

## 状态与实例事实源

插件进程状态只有：

```text
discovered -> starting -> ready -> stopped
                    \-> crashed
disabled
```

不存在局部 `Degraded`。Gateway 状态即时由“插件进程状态 + SDK `ProviderInstance.status`”派生：`starting` 映射 Connecting，`ready` 但实例尚不存在或只有 Created 事实时映射 Unavailable，崩溃映射 Error。实例 create/start/stop/capability response 在同一写锁内替换 SDK instance，并只向 Host update queue 发布一次状态变更；失败不会留下 Host 自造的 Connecting 状态。

实例集合来自 manifest。Manager 启动 enabled 插件后，为每个 enabled manifest instance 执行 create/start；一个实例失败会让该实例保持 Unavailable，但不会阻止同插件的其他 manifest instance 启动。当前没有生产动态创建入口，也不从 Gateway 暴露 lifecycle。

## 路由与事件

远程资源必须完整携带：

```text
deviceId + providerInstanceId + nativeResourceId
```

三段任一为空即拒绝。registry 先核对 device、instance 与所属 plugin；Provider response/event 再核对完整 route。身份保持型 RPC 必须返回与 request 完全相同的资源 ID；`turn.start` 返回 turn 的 conversation 必须等于请求 conversation。route-less `conversation.list` 只在没有 Provider cursor 时聚合；带 cursor 直接返回 `aggregate_conversation_cursor_unsupported`，本阶段不定义复合分页。

Manager 到 Gateway 只有一个有界 `mpsc` receiver，且只能领取一次。Gateway 映射后写入单个有界 replay bus；订阅者 lag 会返回显式错误，旧 cursor 超出 replay 窗口会返回 `event_replay_unavailable`。Gateway 为事件分配 `event-<20 位序号>`，service 内严格单调，事件自身始终保留完整 route。

## SDK 边界

Host 只依赖 `codepet-provider-sdk` 和 `codepet-gateway-sdk`，不定义第二套 Provider/Gateway DTO。四个 Rust SDK 都具备 description/authors/repository metadata，不再 `publish = false`；内部 path dependency 同时声明 `version = "0.1.0"`，可用 `cargo package --allow-dirty` 检查包内容。仓库当前没有 LICENSE 文件，因此 manifest 不虚构 license 声明；正式发布前仍需仓库所有者补充许可证决策。

未来 Provider 包只依赖 Provider SDK，不能反向依赖 Host。默认 Provider 名称没有在 Host 预埋常量或注册框架；具体 `codepet-provider-codex`、`codepet-provider-opencode`、`codepet-provider-claude` 应在实现时以普通 manifest/binary 接入。

## 风险与验证证据

- 强引用环或 FD 泄漏：同一 current-thread Tokio runtime 循环制造坏帧，断言 task 计数、`Weak` 与 `/dev/fd` 回到基线。
- request/event 串线：真实 fixture 并发乱序响应并在 response 前发送 event/notification，断言按 id 和通道分类。
- shutdown 误杀：fixture 收到 shutdown 后关闭 stdout、输出末尾 stderr、等待 200ms 再成功退出；断言 marker、成功 exit 与完整诊断。
- 路由串流：两设备、两实例、错误 device/plugin/instance、空 ID、错误 response identity 和单调 cursor 均有定向测试。
- 状态重复：stop/start lifecycle 后每次只收到一个 ProviderStatusChanged。
- 通道污染：Tauri 真实 fixture 的 event、坏帧和 crash 都不改变 compat replay、companion replay、companion registry 或 `SharedState` activity。
- 机械漂移：验收检查工作树，不提交 `src-tauri/gen/schemas/macOS-schema.json` 或构建产物。

## 验证命令

```sh
cargo test --manifest-path crates/Cargo.toml -p codepet-host --all-targets
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
- 正式发布 SDK 前必须补齐仓库许可证决策，并确定 crate 发布顺序。
