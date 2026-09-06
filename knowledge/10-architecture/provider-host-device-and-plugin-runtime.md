# Provider Host、设备身份与进程外插件运行时

## 当前结论

`crates/codepet-host` 已提供可复用的 Rust Provider Host：它从显式目录读取 manifest，为本机持久化稳定 `DeviceId`，按 manifest 启动独立 Provider 二进制，并通过生成的 `codepet-provider-sdk` 在独占 stdio 上通信。它同时实现内部 `codepet-gateway-sdk::ProtocolServer`、`RemoteAccessManager` 安全核心，以及共用同一 TLS identity 的 Gateway v1 HTTPS/WSS LAN listener。Tauri 后端现已通过单个 `RemoteAccessRuntime` 接入 listener、mDNS、pairing watch 与退出生命周期；连接页已展示 Provider 连接健康与 Harness 状态。

Code Pet 发行包内置 `codepet-provider-codex`、`codepet-provider-opencode` 和 `codepet-provider-claude` 三个独立 adapter 二进制及其 manifest。内置的是 Code Pet 自有的 Provider adapter，不是 Codex、OpenCode 或 Claude runtime；runtime 仍由用户本机安装和配置，Provider 的 `runtime.getInstalled/select` 返回结果是 executable 权威，Host 只持久化选择。

Tauri 由 `ProviderHostState` 管理 Plugin Manager 与 Gateway service；desktop-v0 `RuntimeGatewayState` 只持有同一个 service 的薄适配引用。Provider 数据只有一条远程路径：

```text
Provider binary
  -> PluginProcess / PluginManager
  -> bounded single-consumer Host update queue
  -> ProviderGatewayService replay + subscribers
  -> runtime_gateway_* / runtime-gateway-event
```

Provider 事件进入 Gateway v1 replay，并由兼容适配发布到远程 `runtime-gateway-event`；它们不进入 `SharedState` activity store、Desktop Companion replay、`codex-desktop-companion-event` 或 `pet-event`，也没有失败后回退到桌宠 IPC 的路径。真实 fixture 与 Tauri mock `AppHandle` 测试断言 remote event/replay 收到数据，同时 companion、Pet 与 Desktop adapter spy 保持不变。

连接与 IPC 已按 `remote/`、`remote/channels/lan/`、`providers/` 归组；具体状态权威、两段 SDK 心跳、最后客户端断开后的 Harness 清理见 [Remote 与 Provider 连接架构](remote-and-provider-connections.md)。Tauri 启用连接心跳后，manifest 初始化只创建实例，由 SDK 根据连接集合协调 start/stop。

## 范围与非目标

本阶段实现：

- 持久设备身份和 manifest 实例的稳定 ID 映射；
- 显式插件目录、进程启动、协议协商、实例启动、能力查询和业务路由；
- 有界 CodePet Provider Frame V1、JSON/raw-zstd codec、并发 request id 关联、事件分流、超时、崩溃隔离、stderr 诊断和有界 shutdown；
- 内部 Gateway v1 的 device/provider/capability/conversation/turn/approval 与 event replay 边界。
- 从 App Resources 的单一 `provider-plugins/` 目录自动发现三个默认 Provider adapter，并在 Tauri 开发态及 macOS/Windows 发布构建中 staging。
- 持久化自签 LAN TLS identity、五分钟内存配对 session、只保存 bearer SHA-256 的可撤销远程 credential store，以及固定 pairing/current-credential REST 与 Gateway WSS route。

本阶段不实现：

- dylib/trait ABI、签名、沙箱、市场、下载、自动重启或 backoff；
- 动态注册插件、动态创建生产实例或 Gateway instance lifecycle；
- frontend Remote UI、远程 frame/E2EE、持久 cursor 或权限模型；
- 桌宠 UI、Pet 投影或桌宠动作接入 Provider Manager。

## 配置与持久化

Tauri Catalog 合并以下目录来源：

- App Resources 的 `provider-plugins/`：发行包内置的三个默认 Provider adapter；
- 应用数据目录的 `provider-plugins/`：既有本地 manifest 目录；
- `settings.providerPlugins.directories`：附加显式目录，绝对路径保持不变，相对路径按应用数据目录解析；
- `provider-host/device-identity.json`：稳定本机设备身份；
- `provider-host/provider-instances.json`：manifest 实例的稳定身份映射。

`RemoteAccessConfig::for_data_directory` 在调用方指定的 App 数据目录下使用 `lan-tls-identity.json` 与 `remote-credentials.json`。`RemoteAccessManager` 持有 `Arc<DeviceRegistry>` 并只转发其 identity；它不生成、复制或持久化第二份 `deviceId/displayName`。TLS 文件包含 leaf certificate DER 与 PKCS#8 private key DER，Unix 原子替换后的权限为 `0600`；证书 SHA-256 指纹使用 64 位小写 hex。载入时会校验证书 DER、自签签名、私钥 DER、证书/私钥匹配与 checksum，损坏文件先隔离为 `.corrupt-<timestamp>` 再重建。

远程 credential 文档绑定当前 TLS certificate fingerprint。TLS identity 丢失、损坏或变化时，旧 credential 文档会被隔离并重建为空，因此必须重新配对。credential store v2 只保存 opaque 256-bit bearer 的 SHA-256，不保存 bearer 本身；每条记录保存 `credentialId/clientId/descriptor/createdAt/lastSeenAt/revokedAt`，descriptor 使用 Gateway 生成的 `deviceName/operatingSystem/systemVersion`。旧 v1 文档不做字段兼容，会被隔离并要求重新配对。pairing 成功写入初始 descriptor；同一 credential 的已认证 handshake 在发送成功响应前原子刷新 descriptor，相同值不重写文件。配对 session 只存在于 Host 内存：本地显式开启后生成随机 pairing id 与 32-byte secret，五分钟过期，重启即失效，并在一次成功 credential 交换后由同一互斥区原子作废。

Catalog 只接受目录，不接受运行时 descriptor 注入。每个目录可包含根 `codepet-provider.json`、子目录中的 `codepet-provider.json`，或 `*.codepet-provider.json`。`read_dir` 的目录错误和逐项读取错误都会形成诊断；不会静默丢弃条目。相对 executable 按 manifest 所在目录解析。

manifest 是插件进程和普通实例设置的配置权威：

```json
{
  "manifestVersion": 1,
  "pluginId": "example-provider",
  "displayName": "Example Provider",
  "icon": "https://example.com/provider-icon.png",
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

`icon` 是可选、不含凭据的绝对 HTTPS URL，不接受本地路径、`file://`、明文 HTTP 或 URL userinfo。Host 从 manifest catalog 将它带入 runtime snapshot，并投影到 Gateway v1 的每个 `ProviderInstance.icon`；Gateway client 负责异步加载、缓存，并在地址缺失、无效或加载失败时回退到通用 Provider 图标。内置 Provider 使用各产品官方站点提供的图片地址。

`provider-instances.json` 只镜像 `instanceId + pluginId + instanceKind + displayName`，用于在重启后复用未显式给出的 `instanceId`。`settings` 与 `enabled` 每次都来自当前 manifest，不作为第二配置源；manifest 删除的实例会从映射中 prune。registry 损坏或 device id 不匹配时 fail closed。设备身份损坏时，原文件先隔离为 `.corrupt-<timestamp>`，再生成新的 `device-<uuid>` 并保留诊断。macOS 的设备显示名由 Tauri 通过原生 ComputerName API 读取并在打开 registry 时刷新，但保留原 `deviceId/createdAt`；hostname 和 display name 都不充当稳定 ID。

Runtime 探测属于 Provider：插件级 `runtime.getInstalled` 不接收 Host 候选，由每个 Provider 自主搜索、验证并返回自己的安装项；`runtime.select` 校验并应用选择。Host 只按任意 `pluginId` 转发这两个 RPC、持久化 Provider 返回的选择，并在 Provider 重启后的 instance create 之前恢复选择，不包含 Codex、Claude、OpenCode 或未来 Helms 的路径/版本规则。Codex 与 OpenCode manifest 只保留固定 Server args，三个内置 Provider 分别实现自己的 PATH、登录 Shell、应用安装与版本兼容策略。详见 `codex-provider-plugin-runtime.md`、`claude-provider-plugin-runtime.md` 与 `opencode-provider-plugin-runtime.md`。

`update_app_settings` 区分 `providerPlugins` 缺失和显式空数组：缺失保留现值，`{"providerPlugins":{"directories":[]}}` 才清空目录。前端完整 `AppSettings` 将该字段设为必填。

## 内置资源与构建

`scripts/stage_provider_plugins.mjs` 是唯一 staging 入口。它用 Cargo 构建三个 Provider binary，并用 Bun `--compile` 把 JavaScript 协议编译器构建为原生 `cp-sdk-gen`；Provider binary/manifest 放入 `src-tauri/resources/provider-plugins/<provider>/`，生成器、Core/Provider JSON Schema、method/event manifest、Provider fixtures、完整接入 README 与 `codepet-provider-sdk.json` 资源索引放入 `src-tauri/resources/provider-sdk/`。分发协议采用 version-first 布局：共享定义是 `protocol/v1/core/`，Provider schema/manifest/fixtures 是 `protocol/v1/schema/`；staging 会把 Provider 文档中的 Core `$ref` 重定位为 `../core/schema.json`，避免复制后出现断链。`tauri.conf.json` 分别映射到 App Resources 根下的 `provider-plugins/` 和 `provider-sdk/`。staging 产物被 Git 忽略，只提交脚本、源协议/manifest 和两个目录占位。脚本默认使用 release profile；`--profile debug` 使用 `crates/target/debug`，供开发态快速增量构建。

- macOS universal：分别构建 `aarch64-apple-darwin` 与 `x86_64-apple-darwin`，再逐个 `lipo -create`；App 主程序、三个 Provider 和 `cp-sdk-gen` 都必须通过双架构检查。
- Windows x86_64：构建 `x86_64-pc-windows-msvc`，staging 时把 manifest 的相对 executable 和生成器资源索引改为对应 `.exe` 文件名；NSIS 继续使用同一 Tauri resources 配置。
- 普通 Tauri bundle：`beforeBuildCommand` 调用 `npm run build:bundle`，先执行 `protocol:check` 锁定 schema/generated freshness，再 staging 和构建前端。release workflow 通过 `CODEPET_PROVIDER_TARGET` 明确目标，避免交叉构建时猜架构。
- Tauri 开发态：`beforeDevCommand` 先调用 `npm run providers:stage:dev`，确保 `tauri dev` 使用与发行包相同的 App Resources 发现链路，而不是依赖工作区中手工残留的 Provider binary。
- 独立开发测试：可用 `--staging-dir` 写入临时目录，再以绝对路径设置 `CODEPET_BUNDLED_PROVIDER_PLUGINS_DIR`。未设置时，运行态使用 Tauri `BaseDirectory::Resource` 解析资源，不硬编码安装位置或用户目录。

Catalog 仍只消费普通目录和 manifest，没有默认 Provider registry、市场、签名、沙箱、安装器或自动更新分支。Provider Protocol 事件仍只走 Host/Gateway；bundled discovery 不改变 Pet Protocol、Desktop IPC、companion 或 activity 链。

## 进程、Listener 与 shutdown

`RemoteLanServer::start` 的默认 bind address 是 `0.0.0.0:0`，但 wildcard bind 只有在调用方另行提供具体 advertised IP/DNS host 时才允许启动；实际端口附到 advertised host 后形成 HTTPS base URL 与 WSS Gateway URL，因此不会发布 `0.0.0.0`。Tauri v1 只选择经 active 本机接口复核的 IPv4，并以环境覆盖或 no-send route probe fail-closed。listener 启动时核对 transport 注入的 `RemoteHostIdentity` 与 `RemoteAccessManager` 的 device/TLS fingerprint 完全一致；生产接线只构造一个 `ProviderGatewayService::with_remote_identity`，供 compat 和 LAN 共用。认证、credential clientId 与本地 session cancellation 不进入 `ProviderGatewayService`。

每条 WSS socket 独立完成 handshake、Gateway SDK dispatch、显式 event subscription 与有界 send loop；固定 writer/send-queue timeout 会关闭不读取或队列满的单个 session，全局 semaphore 把并发 session 限制为 32，超限 upgrade 返回 503。订阅前不推 event，订阅后复用现有 replay/live cursor 语义。DELETE current 持久撤销发起 bearer，并向相同 credential 的全部 socket 广播取消；Tauri revoke command 同样先持久撤销，再有界等待该 credential 的 session group 归零。listener shutdown 取消全部 socket并等待 server task，Tauri runtime 持有 handle 并调用该入口。真实 loopback 测试覆盖 binary、超限 text、安全 backpressure 关闭、健康客户端隔离与同 credential 双 socket 撤销。

每个 `PluginProcess` 独占一个子进程及其 stdin/stdout/stderr：

- `PluginProcess::spawn` 是同步入口；Manager 在持有插件表写锁并复核 shutdown gate 后完成 spawn，立即把进程写入对应 entry，再释放锁执行 initialize/describe，因此不存在已 spawn 但 shutdown 不可见的窗口；
- writer task 只持有 `Weak<RpcShared>`，不会与共享状态形成强引用环；
- SDK 生成的 `ProtocolRequest::from_method_params` 负责把 typed method/params 构造成 wire request，Host 不枚举 envelope variant；
- pending map 按 request id 关联响应；event/notification 进入唯一有界 inbound consumer，不会被当作 response；
- shutdown 开始后业务 inbound consumer 会关闭，但 stdout reader 必须继续读取 wire：晚到 event/notification 直接丢弃，`provider.shutdown` response 仍按 pending id 交付。不能因为晚到事件没有消费者而提前关闭 Provider stdout；
- frame、inbound/outbound queue 和 stderr 历史均有上限；坏帧、超限、EOF 或 backpressure 只终止对应进程；
- 所有 terminal path 都关闭 inbound、完成 pending、结束 writer 并关闭 stdin；进程 monitor 在发布 exit 前有界等待 writer、reader 与 stderr task，因此末尾 stderr 已进入退出快照；
- 进程只保留一个 `shutting_down` 状态，并统一使用配置的 `shutdown_timeout`。正常关闭先发 `provider.shutdown`，关闭 stdin，并允许 Provider 先关闭 stdout、延迟退出；真实 fixture 在 Unix 直接关闭 fd 1，并用独立 marker 证明 EOF 后 shutdown future 仍未完成，直到 200ms 后子进程正常退出；
- malformed、oversized、EOF、crash、timeout、normal shutdown 和 Drop 共用同一张 current-thread Tokio case table，逐项验证 task、`Weak`、FD、pending 与 inbound 回收。

Tauri 的 Exit/ExitRequested 和托盘退出先共用 `RemoteAccessRuntime::shutdown_once`，有界停止 pairing monitor、mDNS 和 listener，再进入 `ProviderHostState::shutdown_once`。Provider shutdown 直接以配置的 `shutdown_timeout` 包住 Manager future；失败或超时取消后，调用绕过进程 shutdown gate 的 `force_kill_all`，逐个等待子进程退出后才标记完成。并发/重复调用都复用各自的 shutdown gate。

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

Host 只直接依赖 `codepet-provider-sdk` 和 `codepet-gateway-sdk`，二者共同重新导出 Agent 业务类型；Host 不定义第二套 Provider/Gateway DTO，也不逐字段复制共享业务对象。六个 Rust SDK 都具备 description/authors/repository metadata，不再 `publish = false`；内部 path dependency 同时声明 `version = "0.1.0"`，可用 `cargo package --allow-dirty` 检查包内容。仓库当前没有 LICENSE 文件，因此 manifest 不虚构 license 声明；正式发布前仍需仓库所有者补充许可证决策。

App 内 `provider-sdk/cp-sdk-gen` 是 JavaScript 编写并由 Bun `--compile` 产出的单体原生协议编译器。它运行时读取相邻 Core/Provider schema、manifest 与 fixtures，经与仓库生成流程相同的校验和 normalized typed IR 生成 `generated.rs`，再写入 Bun executable 内嵌的 Cargo package scaffold、稳定 stdio runtime 和接入 README；它不嵌入或复制仓库预生成的 `generated.rs`。`cp-sdk-gen.lock.json` 的 digest 来自本次实际输入协议，`--check` 重新编译并比对所有已知输出。相邻 `provider-sdk/protocol/v1/{core,schema}/` 是面向插件作者、引用可直接解析的 JSON-RPC 2.0 输入事实。

第三方 Provider 的公开接入说明以随 App 和生成 SDK 同时分发的 `provider-sdk/README.md` 为准。它覆盖生成器构建原理、SDK 导出与 freshness、`Provider` trait/`ProviderEventSink`/`serve_stdio` 边界、manifest 字段、独立进程 JSON-RPC 测试、应用数据目录安装以及 `settings.providerPlugins.directories` 发现方式。内置 Codex、Claude、OpenCode 仍是同一边界的实现证据，不形成第三方插件的特殊入口。

Provider 包只依赖 Provider SDK，不能反向依赖 Host。Host 启动 manifest binary 的纵向测试位于 `codepet-host`；Codex、Claude 与 OpenCode Provider 的 all-target/dev dependency graph 都不含 Host、Gateway、Tauri 或 Pet SDK。三个默认 Provider 的名称只存在于各自普通 manifest 与 Tauri runtime-setting 映射中，Host 没有预埋第二套 registry；后续 Provider 仍应以普通 manifest/binary 接入。

## 风险与验证证据

- 强引用环或 FD 泄漏：同一 current-thread Tokio runtime 循环制造坏帧，断言 task 计数、`Weak` 与 `/dev/fd` 回到基线。
- request/event 串线：真实 fixture 并发乱序响应并在 response 前发送 event/notification，断言按 id 和通道分类；shutdown fixture 在 response 前发送 notification，断言 Host 丢弃晚到消息但仍接收 response，Provider 正常退出。
- shutdown 误杀：fixture 响应 shutdown 后真实关闭 stdout fd，写 close marker、输出末尾 stderr、等待 200ms 再成功退出；marker 出现时及 100ms 后 future 均必须未完成，最终断言成功 exit 与完整诊断。
- 启停竞态：initialize 延迟 fixture 与两个并发 `shutdown_once` 触发外层 timeout；断言两次调用都等到 force kill 完成、子进程已结束（Unix 额外用 PID 复核），且 shutdown gate 阻止后续 spawn。
- 路由串流：两设备、两实例、错误 device/plugin/instance、空 ID、错误 response identity 和单调 cursor 均有定向测试。
- 远程身份与凭据：定向测试断言 TLS identity 重启稳定、损坏轮换会清空旧 credential、pairing secret 并发只成功一次且过期/重启失效、bearer 明文不落盘、hash 校验与两种撤销路径持久生效。
- 状态重复：stop/start lifecycle 后每次只收到一个 ProviderStatusChanged。
- 通道污染：Tauri mock runtime 真实监听三条 Tauri event；fixture 的 Provider event 只增加 remote event/replay，不改变 companion replay、`SharedState` activity、companion/Pet event 或 Desktop adapter spy。
- bundled 资源缺失、引用断链或架构不一致：staging 测试断言三 Provider manifest/binary、`protocol/v1/{core,schema}` schema/manifest/fixture、重定位后的 Core `$ref`、完整 README、资源索引和平台 `cp-sdk-gen`；macOS 产物检查 Resources 清单、可执行位与 `lipo -archs`，再从实际 App Resources 导出 SDK 并执行 `cargo check`，DMG 用 `hdiutil verify`。
- 内置 Provider 接入漂移：`builtin_provider_integration` 从三份正式 manifest 启动 SDK 化 Codex、Claude、OpenCode Provider 与各自 fixture，经同一个 Plugin Manager/Gateway 验证三者 ready、provider list、代表性 conversation 操作和有序 shutdown。
- 机械漂移：验收检查工作树，不提交 `src-tauri/gen/schemas/macOS-schema.json` 或构建产物。

## 验证命令

```sh
cargo test --manifest-path crates/Cargo.toml -p codepet-host --all-targets
cargo test --manifest-path crates/Cargo.toml -p codepet-host --test builtin_provider_integration
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --all-targets
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-claude --all-targets
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-opencode --all-targets
npm run sdkgen:test
cargo test --manifest-path sdk/rust/Cargo.toml
npm run protocol:check
npm run providers:test
CODEPET_PROVIDER_TARGET=universal-apple-darwin npm run providers:stage
"src-tauri/target/release/bundle/macos/Code Pet.app/Contents/Resources/provider-sdk/cp-sdk-gen" --lang rust --output /tmp/codepet-provider-sdk
cargo check --manifest-path /tmp/codepet-provider-sdk/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --test runtime_gateway_core_tests --test runtime_gateway_protocol_tests --test settings_tests --test tray_tests
cargo check --manifest-path src-tauri/Cargo.toml --lib
cargo package --manifest-path sdk/rust/Cargo.toml --workspace --allow-dirty
git diff --check
```

## 剩余风险

- macOS 把 App 直接 `posix_spawn` 的 Provider/runtime 后代归因给 Code Pet responsible process，因此受保护目录的 TCC 弹窗显示 Code Pet；进程/协议隔离不等于 TCC 身份隔离。真正拆分权限主体需要 XPC Service 或 `SMAppService` Launch Agent。普通本地 bundle 构建显式 ad-hoc 签名并固定 code-sign identifier 为 `com.codepet.desktop`，但跨构建保留 TCC 决策仍需稳定的 Apple Developer ID 签名。
- Gateway event replay 和 device last-seen 仍为进程内状态，重启恢复未定义。
- Host listener 与 Tauri 后端生命周期已完成真实 loopback TLS/WSS、mDNS 状态同步、连接关闭联动和证书 pin 测试；pairing UI 与真实移动设备跨 LAN pinning 尚待后续阶段。
- manifest 的 executable、args 和 env 是受信任本地配置；签名、权限隔离与资源配额尚未实现。
- 内置 Provider adapter 随 App 一起打包，但不会安装或更新底层 Codex/OpenCode/Claude runtime；用户本机配置与版本兼容性仍决定实例能否启动。
- 没有自动重启/backoff；故障实例需要显式重启 Host/插件。
- Codex Provider 的 App Server 协议覆盖与明确限制见 `codex-provider-plugin-runtime.md`。
- Claude Provider 的 CLI stream-json 能力与明确限制见 `claude-provider-plugin-runtime.md`。
- OpenCode Provider 的 Server API 覆盖与明确限制见 `opencode-provider-plugin-runtime.md`。
- 正式发布 SDK 前必须补齐仓库许可证决策，并确定 crate 发布顺序。
