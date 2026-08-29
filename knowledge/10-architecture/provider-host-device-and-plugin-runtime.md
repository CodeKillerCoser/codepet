# Provider Host、设备身份与进程外插件运行时

## 背景与证据

协议第一阶段已经把 Host 与 Provider binary 的 stdio JSON-RPC 契约生成到 `codepet-provider-sdk`，并把 Host 与 Remote Client 的设备路由契约生成到 `codepet-gateway-sdk`。现有生产路径仍是两条隔离链路：Runtime Gateway compat-v0 连接 Codex App Server，Desktop Companion 连接 Codex Desktop 私有 IPC；二者拥有独立的 registry、event bus 和 replay。

本阶段新增 `crates/codepet-host`。它直接依赖生成 SDK，提供持久设备身份、显式插件目录、Provider 进程监管、实例注册和内部 Gateway v1 service。`runtime_gateway_core_tests::provider_plugin_events_stay_out_of_compat_companion_and_pet_activity_state` 验证 Provider event 只出现在新的 Gateway v1 replay 中，不进入 compat-v0 remote replay、Desktop Companion replay 或 pet activity store。

## 目标

- 把 `DeviceId` 作为稳定、持久化的一等身份，不以 hostname 代替。
- 只从显式配置和目录 manifest 发现进程外 Provider binary。
- 管理插件 initialize/describe、版本协商、instance 生命周期、capability、conversation/turn/approval 和 shutdown。
- 使用有界 JSON-lines codec、request id 关联、超时、stderr 诊断与 crash/EOF 隔离可靠监管每个子进程。
- 用 `deviceId + providerInstanceId + nativeResourceId` 在 Gateway v1 边界 fail closed 路由。
- 保持插件、现有 Codex Remote 和 Desktop Companion 三条生命周期与事件路径互不回退、互不污染。

## 非目标

- 不迁移 Codex、OpenCode 或 Claude 到插件 binary。
- 不实现 dylib/trait ABI、插件签名、沙箱、市场或自动下载。
- 不实现 Gateway v1 LAN listener、配对、认证或持久 event cursor。
- 不改变桌宠 UI、`PetProjection`、activity store、companion Tauri event/replay 或桌宠动作。
- 不在 Host crate 手写 Provider/Gateway 的第二套传输 DTO。

## 模块与公开边界

- `device.rs`：`DeviceIdentity`、`DeviceRegistry`。首次启动生成 `device-<uuid>` 并持久化；损坏文件先隔离为 `.corrupt-<timestamp>`，再安全重建并保留诊断。
- `catalog.rs`：`PluginCatalogConfig`、`PluginDescriptor`、`PluginCatalog`。接受显式 descriptor 或目录；目录只读取 `codepet-provider.json` 与 `*.codepet-provider.json`，相对 executable 按 manifest 目录解析。重复 plugin id 被诊断并拒绝。
- `instance_registry.rs`：`ProviderInstanceRegistry`。持久保存稳定 instance id、plugin id、instance kind、device id、native settings 与 enabled；错误设备、实例或预期 plugin 的 route 均拒绝。
- `process.rs`：`PluginProcess` / `ProviderRpcClient`。每个插件进程独占 stdin/stdout；单 writer、单 reader 和 pending request map 保证并发请求按 id 关联，event/notification 不会被当 response。
- `manager.rs`：`PluginManager`。提供注册、启停、动态创建/销毁 instance、capability 与 Provider 业务方法；单插件故障只改变该插件及其实例快照。
- `gateway.rs`：`ProviderGatewayService`。实现生成的 `codepet_gateway_sdk::ProtocolServer`，将 device/provider list、capability、conversation、turn 与 approval 映射到 Plugin Manager，并把 Provider event 映射为带 route/cursor 的 Gateway event。

Host crate 只消费 `codepet-provider-sdk` 和 `codepet-gateway-sdk`；未来 `codepet-provider-codex`、`codepet-provider-opencode`、`codepet-provider-claude` 只能依赖 Provider SDK，不能依赖 Host。

## 配置与持久化

Tauri 以已解析的应用数据目录为根：

- `provider-host/device-identity.json`：稳定设备身份。
- `provider-host/provider-instances.json`：该设备的 Provider instance registry。
- `provider-plugins/`：默认显式插件目录。
- `settings.providerPlugins.directories`：额外插件目录；相对路径按应用数据目录解析，绝对路径保持不变；旧设置缺失该字段时默认空数组。

manifest 最小格式如下：

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

`args` 与 `env` 只来自显式 manifest/注册 API；Host 不扫描 dylib，也不从 executable 输出推导注册信息。instance 未显式给出 id 时生成 `instance-<uuid>`，之后按持久化记录复用。instance registry 损坏或 device id 不匹配时 fail closed，避免把旧实例路由到新设备身份。

## 运行时状态机

插件状态至少包含：

```text
discovered -> starting -> ready -> stopped
                 |          |
                 v          v
              degraded   crashed

disabled --(enable)--> discovered
```

启动顺序是 spawn、initialize、版本区间核对、describe 与 descriptor/plugin id 核对，然后才进入 ready。已配置 instance 在插件 ready 后依次 create/start；某个 instance 失败只记录该 instance 的诊断，不阻止其他实例。坏帧、超限帧、EOF 或非零退出会完成该进程所有 pending request，并将对应插件标记为 degraded/crashed；不会切换到 Desktop Companion 或 pet IPC。

shutdown 先发 `provider.shutdown`，关闭 stdin 并等待进程；超时后 kill。`Drop` 仍有 kill-on-drop 兜底。stderr 由独立任务读取到有界诊断历史，不参与协议 framing；进程退出后历史会复制到插件快照，Tauri 边界会记录 degraded/crashed 状态及最后一条 stderr 诊断。

## Gateway v1 接线

`ProviderGatewayService` 是内部 service/API，当前没有网络 listener。它实现生成的 v1 trait，并完成：

- `handshake`、`device.list`、`providerInstance.list` 与 capabilities；
- `conversation.list/get/create`；
- `turn.send`（start/steer）、`turn.interrupt`；
- `approval.resolve`；
- Provider event 到 Gateway event 的 typed 映射。

所有资源都先经 instance registry 校验 device + instance，再经 manager 校验 instance 所属 plugin。Gateway 为事件分配规范化 `event-<20 位序号>` cursor；序号在 service 内全局单调，因此不同设备/实例不会各自复用 cursor，event route 仍保留完整三段资源身份。replay 当前为有界内存窗口，过期、超前或非法 cursor 均 fail closed。

Tauri 的 `RuntimeGatewayState` 可选持有 `PluginManager` 和 `ProviderGatewayService`，在 setup 后台启动 enabled 插件。现有 compat-v0 Gateway、Codex App Server 与 Desktop Companion 无法访问该 manager；Provider 失败也没有回退到这些 IPC 的代码路径。

## 风险与验证

- 风险：多个 caller 共享 stdio 导致 response 串线。验证：fixture binary 上并发请求乱序返回，测试断言按 request id 关联。
- 风险：event 被 pending response reader 消费。验证：fixture 在 response 前发 notification/event，测试分别从 event channel 接收。
- 风险：无界 frame 或 stderr 消耗内存。验证：codec frame 上限、async reader 上限与有界 stderr 历史；测试覆盖超限和坏帧。
- 风险：超时或崩溃拖垮其他插件。验证：单进程 pending request 独立完成；测试在一个 fixture timeout/crash 后继续使用另一插件。
- 风险：device/instance/native id 不完整导致跨实例串流。验证：registry 与 Gateway 测试覆盖两设备、两实例、错误 device/plugin/instance 以及完整 event route。
- 风险：插件事件重新进入桌宠。验证：Tauri core test 同时断言 provider v1 replay 非空而 compat-v0、companion replay 与 activity store 为空。
- 风险：Tauri schema 构建产生机械漂移。验证：验收后检查并恢复 `src-tauri/gen/schemas/macOS-schema.json`，不把漂移提交。

## 验证命令

```sh
cargo test --manifest-path crates/Cargo.toml -p codepet-host
cargo test --manifest-path sdk/rust/Cargo.toml -p codepet-provider-sdk -p codepet-gateway-sdk
npm run protocol:check
cargo test --manifest-path src-tauri/Cargo.toml --test settings_tests --test runtime_gateway_protocol_tests --test runtime_gateway_core_tests
cargo check --manifest-path crates/Cargo.toml -p codepet-host --all-targets
cargo check --manifest-path src-tauri/Cargo.toml --lib
git diff --check
```

## 剩余未知项

- Gateway event cursor 仍只在内存中，进程重启后的持久化和 session 恢复策略待下一阶段定义。
- 插件进程当前按显式配置启动，尚无自动重启/backoff、签名、权限隔离或资源配额。
- Gateway v1 网络 transport、认证与远程客户端接线尚未实现。
- 三个预留默认 binary 仅固定命名和注册接口，具体 Provider adapter 尚未迁移。
