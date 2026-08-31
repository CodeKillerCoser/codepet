# 协议层依赖与运行通道边界

## 规则

协议事实必须从 `protocol/` 单向生成到 `sdk/`：schema 与 manifest 的全部 `$ref` 使用同一依赖审计，core 不依赖任何上层，pet/provider/gateway 只能引用显式声明的安全下层。每种输出语言必须有独立 target adapter，未实现 target 必须 fail closed。Provider plugin event 不得发布到 companion Tauri event、replay、Pet projection 或 activity store；Pet action 不得调用插件生命周期接口。Gateway 快照响应的 `snapshotCursor` 必须在发起 Provider 查询前捕获；`EventCursor` 对客户端始终 opaque，只能原样保存和回传，不能解析、排序或执行 `+1`。返回完整会话历史的 Provider 与 Host reader 必须显式使用同一个有界帧上限；只放宽一端会让大历史在响应阶段退出 Provider。

## 适用场景

- 修改任一 schema、method/event manifest、generator 或 SDK 包。
- 接入 Provider binary、Provider Host、Gateway remote transport 或 Pet Protocol adapter。
- 修改 Runtime Gateway registry/event bus/Tauri bridge 或桌宠 projection。
- 修改 Gateway snapshot、replay、live event 或 `turn.outputDelta` 的同步流程。
- 修改 `conversation.list/get` 的完整历史映射、Provider JSON-line codec 或 Host process frame 配置。

## 反例

- 在 Rust 业务模块手写一套与 IDL 相同的 DTO，再让 generator 追随 Rust。
- 在 core 放入 Conversation、Turn、Approval、PetTask 或插件进程状态。
- 只带 native conversation id 穿过 Remote Client 边界。
- 让 Provider SDK event enum复用 companion event sink，之后依赖前端过滤 remote task。
- 为方便启动插件，把 `instance.start` 或 `provider.shutdown` 暴露到 gateway manifest。
- 让 Dart/Python target 落入 TypeScript generator，或因为没有 output 而返回成功。
- 在 Provider Host 另写无界 `read_line`/宽松 JSON-RPC parser，接受同时包含 result 与 error 的 response。
- Provider 查询完成后才读取 `snapshotCursor`，导致查询期间的事件可能落在客户端订阅边界之前。
- 客户端把 `EventCursor` 当数字递增，或让 `conversation.get` 快照携带仍由 `turn.outputDelta` 修改的进行中正文。
- Provider 仍用 1 MiB 默认 codec，而 Host 单独放宽到 16 MiB；超过 1 MiB 的合法历史会在 Provider 写响应时直接终止进程。

## 推荐做法

- 先修改 `protocol/<layer>/vN`，运行 `npm run protocol:generate`，业务代码只 import SDK。
- 新语言实现 `codepet.protocol.codegen/v1` adapter；在 package outputs 与负向测试就绪前保持 planned。
- 共享资源使用 `RoutedResourceId`；实例级请求显式携带 deviceId 与 providerInstanceId。
- Provider 使用生成 `JsonLineCodec`、wire classifier、`ProtocolRequest::from_method_params`、typed capability mapping 和 instance-kind validation helper；Host 不枚举 request envelope variant。
- Provider Host 将插件事件映射到 application/gateway channel；Desktop Companion 单独映射到 pet channel。
- 兼容旧 wire 时，把 profile 放在同一 IDL 根并生成独立 module，业务侧只保留 re-export 或 adapter。
- Gateway 在调用 Provider `conversation.list/get` 前捕获 `snapshotCursor`；客户端把该值原样传给 `event.subscribe.afterCursor`，由 Gateway 返回相同的 `subscribedAfterCursor`。
- Conversation snapshot 只承载元数据和稳定内容，进行中正文只由 live `turn.outputDelta` 承载；不要为同步方便临时引入权威正文投影或 revision delta。
- 完整历史确需超过默认帧时，在共享 SDK 定义一个受限常量，并让目标 Provider encoder 与生产 Host reader 同时 opt in；保留默认小帧供其他通道使用。

## 来源

- `../50-decisions/language-neutral-protocol-idl-and-sdk-boundary.md`
- `../50-decisions/codex-remote-and-desktop-companion-dual-channel.md`
- `../10-architecture/protocol-layers-and-device-routing.md`
- `../../protocol/README.md`
- `../../crates/providers/codepet-provider-codex/src/main.rs`
- `../../src-tauri/src/runtime_gateway/tauri_bridge.rs`

## 验证方式

- `npm run protocol:check` 验证 schema/manifest 依赖方向、Pet 无 Provider ref、target fail-closed、capability metadata、fixture 与 freshness。
- `cargo test --manifest-path sdk/rust/Cargo.toml` 验证 SDK 编译、bounded framing、wire classification、标准 JSON-RPC error、dispatcher、instance kind 和路由模型。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test manager_gateway` 验证 `event.subscribe` dispatch，并用 Provider 查询期间先发事件的场景锁定 `snapshotCursor` 捕获顺序。
- Runtime Gateway 双链路测试断言 remote event 不进入 companion replay。
- `provider_binary_transports_a_complete_history_larger_than_one_mebibyte` 验证超 1 MiB 的完整历史成功返回且 Provider 仍可响应；`provider_host_accepts_bounded_complete_conversation_history_frames` 验证生产 Host 使用相同上限。
- Review gateway manifest 不含 instance lifecycle/shutdown，Pet schema 不含 Provider/Conversation/Turn/Approval 类型引用。
