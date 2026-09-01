# 插件化协议分层与多设备路由基础

## 背景

2026-08-30 的当前实现中，Provider Host 可启动独立 `codepet-provider-codex` 与能力较小的 `codepet-provider-claude`。Codex 另有完全隔离的 `CodexDesktopCompanionState` 私有 IPC 链路；Claude Provider 不消费 Hook/transcript，也不把 Provider event 写入 Pet 链。`runtime_gateway_core_tests::real_provider_events_only_emit_remote_tauri_channel_and_never_call_desktop_adapter` 证明任意 Provider event 只进入 remote replay/event，companion/Pet/activity 与 Desktop adapter 不变；`frontend/PetApp.svelte` 只消费 companion channel。Claude CLI 继承的本机 Hook 若自行调用旧 collector，仍属于独立 Hook 数据源。

旧协议事实集中在 `protocol/schemas/v0.json`，Rust 生成物位于 Tauri 源码目录，且一份模型同时承担现有进程内 gateway 和未来 Provider/Remote/Pet 契约。它已有可复用的 JSON Schema 子集校验、manifest method/event 配对、fixture 验证和 Rust/TypeScript 生成逻辑，但不能表达独立插件生命周期、设备路由或 Pet 与 Provider 的强隔离。

## 目标

- 让 `protocol/` 成为唯一语言无关的手写协议事实来源。
- 建立 `core`、`pet`、`provider`、`gateway` 四个 v1 层，并用生成检查守住依赖方向。
- 把 `DeviceId`、`ProviderPluginId`、`ProviderInstanceId`、`ClientId` 和完整 `RoutedResourceId` 提升为显式类型。
- Provider v1 形成 Host 与独立二进制之间的 JSON-RPC 2.0/stdin-stdout 契约；gateway v1 形成 Host 与 Remote Client 的设备/实例路由契约。
- 生成可独立编译的 Rust SDK，包含 DTO、异步 server trait、dispatcher、typed client/transport、method/event enum、wire envelope 和 codec。
- 生成纯 null-safe Dart core/Gateway SDK，供未来 `codepet-remote` 复用 DTO、strict codec、method/event metadata 与 typed client。
- 保持现有 Runtime Gateway v0 wire 与双链路行为可编译、可测试，不把 Desktop IPC 迁入 Provider 协议。

## 非目标

- 不实现自动安装、签名、市场或沙箱；Plugin Manager、显式目录发现与 Provider 进程生命周期已落到 `crates/codepet-host`，开发安装仍为显式复制 manifest/binary。
- 不迁移 Desktop IPC adapter 到独立 Provider 二进制；Codex App Server 已迁到独立 Provider。
- LAN identity、QR 与 pairing/credential REST DTO 仍只由 Gateway SDK 生成；Host/Tauri 后端已接入 TLS/HTTP/WSS listener 与 mDNS 生命周期，但不实现持久 event cursor 或 frontend Remote UI。
- 不改桌宠展示、交互或 activity projection；本阶段只提供未来 Pet Protocol 的生成 SDK。
- 不生成 Python、Dart Pet/Provider 或 Gateway compat-v0 SDK；Python 只固定可复用的 generator interface 与 target manifest。

## 现状理解

协议布局现在是 `protocol/{core,pet,provider,gateway}/v1`。`protocol/codegen.json` 记录包、依赖、输出与 `codepet.protocol.codegen/v1` target adapter 接口。Rust、TypeScript、Dart 有显式 adapter；Python 只有 fail-closed 的 planned registry entry，显式选择时在写文件前失败。Rust 输出位于 `sdk/rust/codepet-*-sdk`；TypeScript 生成 core、Gateway v1 和现有 Runtime Gateway v0 兼容面；Dart 生成 `sdk/dart/codepet-{core,gateway}-sdk`。

`core/v1` 只包含可安全共享的 ID、版本范围、时间戳、分页、错误、JSON 对象、JSON-RPC error 和 `RoutedResourceId`。`pet/v1` 只出现 PetTask/PetApproval/PetAction/Snapshot/Patch；schema 和 manifest 不引用 provider/gateway。`provider/v1` 拥有 initialize、describe、instance create/start/stop/destroy/capabilities、conversation、turn、approval、event 和 shutdown。`gateway/v1` 拥有 handshake、device/provider 枚举、conversation/turn/approval 与 replayable event cursor，不包含 instance 生命周期或 shutdown。

旧 Runtime Gateway 契约没有被复制回 Tauri 源码。它作为 `protocol/gateway/v1/compat-v0.*` 的 IDL profile 生成到 `codepet-gateway-sdk::compat_v0`；`src-tauri/src/runtime_gateway/generated.rs` 和 `frontend/lib/generated/runtimeGateway.ts` 只是 re-export 薄层。这样兼容 profile 仍由同一个 IDL 根和同一个生成器约束，同时 v1 不必伪装成已接线的生产网络协议。

## 实现路径

1. `tools/protocol-codegen/generate.mjs` 读取所有包，统一审计 schema 和 manifest 中的全部 `$ref`，验证 Draft 2020-12 子集、依赖声明、method/event/capability metadata、transport discriminator 和 fixture。
2. schema/manifest 校验后先生成唯一 normalized typed IR；target registry 把 Rust、TypeScript、Dart、Python 映射到独立 adapter，未实现 adapter 不允许降级到其他语言。Dart 的 DTO、constraint、sealed union、CodePet envelope、method/event metadata 与 typed client 都从同一 IR 生成；无共同 required singleton-enum discriminator 的 union 在写文件前失败。
3. Provider 根据 JSON-RPC 2.0/stdio-json-lines 生成有界 `JsonLineCodec`、request/response/notification/event 入站分类、标准错误映射、含入站接口的 transport，以及 `ProtocolRequest::from_method_params` typed request-to-wire 入口。Host 调用该入口，不再手写 method 到 JSON-RPC envelope variant 的枚举。未知 method 与非法 params 保留 request id，分别映射 `-32601` 与 `-32602`；response 必须满足 result/error XOR。
4. v1 initialize/handshake 通过 `VersionRange` 提交支持范围，并返回 selected version。manifest version、wire version 与生成常量由同一输入产生。
5. Provider 和 gateway 的资源 ID 均使用 `RoutedResourceId { deviceId, providerPluginId, providerInstanceId, nativeResourceId }`；Provider descriptor 的 `instanceKinds` 非空，create request/instance 都携带稳定 `instanceKind`，生成 helper 供服务实现 fail closed 选择。请求、响应、事件和审批必须逐跳校验四段身份，不能事后回查 plugin id 补齐 route。
6. capability enum、capability container 与 method mapping 同时受 manifest/schema 校验，并生成 typed `ProtocolMethod::capability()`。
7. Tauri 依赖 `codepet-host` 与 `codepet-gateway-sdk`，并继续通过 compat v0 re-export 使用原类型。`RuntimeGatewayState` 只适配 `ProviderHostState` 创建的 Gateway service；remote 与 companion 保持各自 EventBus/replay/Tauri event，Provider v1 event 经 Gateway 只发布到 remote channel。
8. `crates/codepet-host` 已消费生成 SDK 实现进程外 Provider client、Plugin Manager 和 `codepet-gateway-sdk::ProtocolServer` application boundary；Manager 到 Gateway 是只能领取一次的有界单消费者队列，不指向 companion bus。详见 `provider-host-device-and-plugin-runtime.md`。
9. `crates/providers/codepet-provider-codex` 使用生成 Provider SDK 实现全部 v1 lifecycle/业务方法和事件，官方 App Server client 不再位于 Tauri。详见 `codex-provider-plugin-runtime.md`。
10. `crates/providers/codepet-provider-claude` 使用同一生成 Provider SDK 与 Host lifecycle，只适配官方 CLI `stream-json` 可验证的 create/start/result/Unix interrupt；list/get/steer/approval 明确关闭。详见 `claude-provider-plugin-runtime.md`。
11. `crates/providers/codepet-provider-opencode` 同样只使用生成 Provider SDK；OpenCode HTTP/SSE DTO 留在插件内部，Host resolver 是 Server executable 的唯一来源。详见 `opencode-provider-plugin-runtime.md`。

## 涉及模块

- `protocol/`：唯一手写 schema、service manifest、transport discriminator、fixture 和 target manifest。
- `tools/protocol-codegen/`：生成与 freshness/边界检查；不包含业务 handler。
- `sdk/rust/`：四个可独立编译、可执行 `cargo package` 检查的 SDK；Provider 与 gateway 只依赖 core，pet 只依赖 core。path dependency 同时声明发布 version。
- `sdk/typescript/`：生成 Gateway v1 Remote Client 契约与现有前端 compat 输入，不承担 Pet 或 Provider 业务。
- `sdk/dart/`：生成 core/Gateway v1 null-safe package；transport、TLS、credential、重连与事件持久化留给 `codepet-remote`。
- `src-tauri/src/runtime_gateway/generated.rs`：只把 compat v0 SDK 暴露给现有手写 gateway。
- `frontend/lib/generated/runtimeGateway.ts`：只把 compat v0 TypeScript 类型暴露给当前前端。
- `crates/providers/codepet-provider-codex/`：首个 production Provider binary；运行依赖只有 Provider SDK 与纯 Rust App Server adapter。
- `crates/providers/codepet-provider-claude/`：Claude production Provider binary；运行依赖只有 Provider SDK 与纯 Rust CLI adapter。
- `crates/providers/codepet-provider-opencode/`：OpenCode Server HTTP/SSE adapter 与独立 Provider binary；不依赖 Host、Gateway、Tauri 或 Pet。
- `src-tauri/src/runtime_gateway/provider_host_compat.rs`：compat remote 到 Gateway v1 的薄适配。
- `src-tauri/src/runtime_gateway/{gateway,event_bus,tauri_bridge}.rs`：companion 业务与 Host/Tauri bridge；`ProviderHostState` 负责启动和一次性有界 shutdown。

## 风险

- 风险：在 core 放入 Pet/Provider/Gateway 领域对象，导致层间重新耦合。验证：codegen dependency audit 与 Node 测试断言 core 无上行依赖。
- 风险：Pet schema 或 manifest 偷用 Provider conversation/approval，未来再次把 remote 事件投影到桌宠。验证：统一 `$ref` audit 和 manifest 反向负例；现有双 transport Rust 测试继续运行。
- 风险：资源漏掉 plugin identity，在相同 device/instance/native 组合间误路由。验证：core/provider/gateway fixture、生成 SDK 与 Host 负例测试断言四段身份同时存在且错误 plugin fail closed。
- 风险：生成代码被手改或 Rust/TypeScript/Dart 输出漂移。验证：`npm run protocol:check` 比较完整内容并报告 stale file。
- 风险：planned target 被错误交给其他语言或静默无输出，或 Dart 为 model/service 维护两份映射。验证：fake/Python fail-closed、normalized IR route/metadata、deterministic output 与多包 cross-schema 测试。
- 风险：Dart 把 `oneOf` 降级为多个 optional 字段、接受未知字段/非法约束或日志泄露 pairing secret。验证：ambiguous union 生成负例、canonical fixture round-trip、constraint/closed-object 和 redaction 测试。
- 风险：Provider stdio reader 无界增长、混淆 notification/event 或吞掉 JSON-RPC request id。验证：真实 line framing、超限、坏包、XOR、标准错误和 transport inbound 测试。
- 风险：instance kind 或 capability 继续成为自由字符串并在实现间漂移。验证：schema/manifest contract audit、lifecycle fixture、typed mapping 和服务侧选择失败测试。
- 风险：compat v0 搬迁改变 serde/wire 行为。验证：共享 v0 fixtures round-trip、generated dispatcher 和 Runtime Gateway core tests。
- 风险：Tauri 构建机械改写 `macOS-schema.json`。验证：测试后检查 `git diff`；若漂移，只从本次基线精确恢复该文件。

## 测试计划

- `npm run protocol:check`：schema/manifest/fixture、自洽、联合层依赖、capability contract、target registry 与 freshness。
- `cargo test --manifest-path sdk/rust/Cargo.toml`：四个 SDK 生成/编译、version、typed request-to-wire、JSON-RPC dispatcher/line framing/标准错误、instance kind、event cursor 和 route round-trip。
- `cargo package --manifest-path sdk/rust/Cargo.toml --workspace --allow-dirty`：Cargo 建立临时本地 registry，按依赖顺序打包并验证四个 SDK，无需先上传 core。
- `cargo test --manifest-path src-tauri/Cargo.toml --test runtime_gateway_protocol_tests --test runtime_gateway_core_tests`：v0 wire 与双链路隔离。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --all-targets`：Provider v1、真实 App Server fixture 与实际 Provider 二进制回归。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-claude --all-targets`：Claude CLI fixture、真实输出映射、stdio framing、能力负例与依赖隔离。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-opencode --all-targets`：Provider framing、OpenCode V2 fixture 垂直映射与 Pet 隔离。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --all-targets`：Host 从 manifest 启动 Provider binary、Gateway 纵向 RPC、四段路由与 restart/fault isolation。
- TypeScript 对兼容 SDK 执行独立 `tsc --noEmit`，并运行现有前端 protocol/component tests。
- Dart 对 core/Gateway package 执行 `dart analyze`；Gateway package runtime tests 覆盖 canonical fixture、nullable、discriminated union、约束/unknown field、redaction、manifest metadata 与 typed client。
- 测试后确认 `src-tauri/gen/schemas/macOS-schema.json` 无提交差异。

## 知识沉淀

本页记录可执行架构与验证路径。长期取舍见 `../50-decisions/language-neutral-protocol-idl-and-sdk-boundary.md`；禁止跨层依赖与 channel 污染的 review 规则见 `../60-rules/protocol-layer-and-channel-boundaries.md`。Codex binary 的实现边界见 `codex-provider-plugin-runtime.md`，remote/companion 隔离规则由 `../60-rules/codex-provider-channel-isolation.md` 约束；Claude 的能力降级与机器接口见 `claude-provider-plugin-runtime.md` 和 `../60-rules/claude-provider-machine-interface.md`。

## 未知项

- Provider Host 的进程生命周期与 manifest 身份映射已经实现；自动重启/backoff、签名和 sandbox 尚未实现。实例 settings/enabled 以 manifest 为基础且不持久为第二配置源；Codex 的 `appServerExecutable` 与 OpenCode 的 `serverExecutable` 由 Host resolver 在内存中唯一覆盖。
- 仓库当前没有 LICENSE 文件；SDK manifest 不虚构许可证，正式发布前需要所有者补充 license 决策。
- Gateway v1 event cursor 的持久化格式、过期窗口和远程 session 恢复策略尚未确定。
- Python adapter 尚未实现；registry 会拒绝显式选择。Dart 当前对未知 enum 和不支持 union fail closed，并只提供 transport-neutral request API；async event stream、重连/重试和持久 cursor 属于 `codepet-remote` runtime。
- 现有 Desktop Companion 何时从 compat v0 映射到 Pet v1，需要独立阶段验证，不能顺带进入 Provider 协议。
