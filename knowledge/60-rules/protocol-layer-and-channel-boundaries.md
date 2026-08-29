# 协议层依赖与运行通道边界

## 规则

协议事实必须从 `protocol/` 单向生成到 `sdk/`：schema 与 manifest 的全部 `$ref` 使用同一依赖审计，core 不依赖任何上层，pet/provider/gateway 只能引用显式声明的安全下层。每种输出语言必须有独立 target adapter，未实现 target 必须 fail closed。Provider plugin event 不得发布到 companion Tauri event、replay、Pet projection 或 activity store；Pet action 不得调用插件生命周期接口。

## 适用场景

- 修改任一 schema、method/event manifest、generator 或 SDK 包。
- 接入 Provider binary、Provider Host、Gateway remote transport 或 Pet Protocol adapter。
- 修改 Runtime Gateway registry/event bus/Tauri bridge 或桌宠 projection。

## 反例

- 在 Rust 业务模块手写一套与 IDL 相同的 DTO，再让 generator 追随 Rust。
- 在 core 放入 Conversation、Turn、Approval、PetTask 或插件进程状态。
- 只带 native conversation id 穿过 Remote Client 边界。
- 让 Provider SDK event enum复用 companion event sink，之后依赖前端过滤 remote task。
- 为方便启动插件，把 `instance.start` 或 `provider.shutdown` 暴露到 gateway manifest。
- 让 Dart/Python target 落入 TypeScript generator，或因为没有 output 而返回成功。
- 在 Provider Host 另写无界 `read_line`/宽松 JSON-RPC parser，接受同时包含 result 与 error 的 response。

## 推荐做法

- 先修改 `protocol/<layer>/vN`，运行 `npm run protocol:generate`，业务代码只 import SDK。
- 新语言实现 `codepet.protocol.codegen/v1` adapter；在 package outputs 与负向测试就绪前保持 planned。
- 共享资源使用 `RoutedResourceId`；实例级请求显式携带 deviceId 与 providerInstanceId。
- Provider 使用生成 `JsonLineCodec`、wire classifier、typed capability mapping 和 instance-kind validation helper。
- Provider Host 将插件事件映射到 application/gateway channel；Desktop Companion 单独映射到 pet channel。
- 兼容旧 wire 时，把 profile 放在同一 IDL 根并生成独立 module，业务侧只保留 re-export 或 adapter。

## 来源

- `../50-decisions/language-neutral-protocol-idl-and-sdk-boundary.md`
- `../50-decisions/codex-remote-and-desktop-companion-dual-channel.md`
- `../10-architecture/protocol-layers-and-device-routing.md`

## 验证方式

- `npm run protocol:check` 验证 schema/manifest 依赖方向、Pet 无 Provider ref、target fail-closed、capability metadata、fixture 与 freshness。
- `cargo test --manifest-path sdk/rust/Cargo.toml` 验证 SDK 编译、bounded framing、wire classification、标准 JSON-RPC error、dispatcher、instance kind 和路由模型。
- Runtime Gateway 双链路测试断言 remote event 不进入 companion replay。
- Review gateway manifest 不含 instance lifecycle/shutdown，Pet schema 不含 Provider/Conversation/Turn/Approval 类型引用。
