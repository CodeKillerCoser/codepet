# 使用语言无关分层 IDL 与独立 SDK 包

## 背景

旧 v0 由一份 JSON Schema 同时描述 Runtime Gateway 的 Provider、conversation、turn、approval 与远程事件，Rust 生成物直接放在 Tauri 业务模块。该方式已证明 schema/manifest/codegen/fixture 流程可行，却无法清晰区分 Desktop Companion、独立 Provider 插件和 Remote Client，也没有设备与 Provider instance 的全局路由身份。

## Dart 工具证据

在引入 Dart target 前，使用当前 `quicktype 26.0.0` 对 Gateway Draft 2020-12 schema 做了真实 CLI 实验。它可以生成 null-safe Dart，并能沿相对路径解析 Gateway→Core `$ref`；Apache-2.0 许可证和生成代码无额外知识产权限制也符合仓库使用前提。但实验输出把共享 `DeviceDescriptor`、`VersionRange` 重命名为使用点局部类，忽略 `minLength/minimum/maximum/pattern`、`additionalProperties: false` 与 `x-codepet-sensitive`，并把 `ModelCatalog.oneOf` 合并为同时带多个可选字段的单一 class，无法保持 `kind` 分支互斥。该结果不能安全解码 hostile LAN/WSS 输入。

`json_serializable 6.14.x` 能开启 unknown-key 检查，Freezed 4.x 能表达 tagged union，但两者都要求先维护带 annotation 的 Dart class，再由 Dart build runner 生成辅助代码；这会把 Dart class 变成第二份模型事实。OpenAPI Generator 的稳定 `dart-dio` target 支持 discriminator/oneOf，但输入边界与输出重点是 OpenAPI HTTP/Dio client；CodePet 当前是 JSON Schema + 自定义 method/event manifest + transport-defined envelope，把 IDL 转成 OpenAPI 会新增一层有语义的中间事实和不需要的 HTTP runtime。

因此现阶段不存在能直接保留 CodePet 有限 schema/manifest 全语义、同时又不产生第二事实源的成熟 drop-in generator。重新评估第三方工具的门槛是：能消费规范化 IR 或原始 IDL、保留稳定类型身份与所有受检约束、允许自定义 method/event service 层，并能在 unsupported semantics 上 fail closed。

## 决策

协议长期事实只存在于 `protocol/` 的语言无关 IDL，并按 `core`、`pet`、`provider`、`gateway` 四层独立版本化。生成包统一命名 `codepet-*-sdk` 并放在 `sdk/<language>/`；Rust struct、Tauri DTO 和任何语言侧 model 都不能成为第二事实来源。

Provider v1 固定为 Host ↔ 独立 Provider 二进制的 JSON-RPC 2.0/stdio 协议，拥有插件描述与 instance 生命周期。Gateway v1 固定为 Host ↔ Remote Client 协议，只暴露设备、Provider instance、远程资源与 event cursor，不暴露插件进程控制。Pet v1 固定为 Desktop Companion 驱动的任务、审批、动作、快照与 patch，不引用 Provider 领域对象。

每种输出语言必须通过 `codepet.protocol.codegen/v1` target adapter registry 显式接入。校验后的 schema/manifest 先归一化成唯一 typed IR；Dart 的 definition、constraint、union discriminator、method、event、capability 和 transport metadata 只能从该 IR 投影，不得维护第二份 model/method/event 表。Rust、TypeScript、Dart 是已实现 adapter；现有 Rust/TypeScript emitter 暂保留 validated model 输入，迁移到 normalized IR 必须另做等价 fixture/diff 验证。Python 在实现前保持 planned 且选择即失败，不能落入另一个语言 generator 或静默跳过。schema 与 manifest 的 `$ref` 使用同一依赖审计，capability type/container/method mapping 也是生成器受检契约。

Dart target 采用只覆盖当前受检 schema 子集的结构化 emitter，不引入 quicktype、Freezed、json_serializable 或 build_runner 依赖。它生成 core 与 Gateway v1 两个 null-safe package、closed-object/约束校验、显式 sealed union、敏感字段 redaction、manifest metadata、envelope codec 与 transport-neutral typed client。无法找到共同 required singleton-enum discriminator 的 `oneOf` 在生成前失败；不会退化为 `dynamic` 或宽松可选字段模型。

既有 Tauri Runtime Gateway 调用面使用同一 gateway SDK 内的生成 compat v0 module。compat profile 仍来自 `protocol/gateway/v1` 下的 IDL，不允许在 Tauri 源码复制 DTO。它现在薄适配到 Host 的 Gateway v1 application service；Provider v1 已用于真实插件进程，但这仍不代表 Gateway v1 网络 transport 已实现。

## 备选方案

- 继续以 Tauri Rust struct 为主并导出其他语言：短期简单，但 Rust 特性会决定 wire 语义，并形成与 JSON Schema 并列的事实来源，因此不采用。
- 为 pet/provider/gateway 各自手写重复 ID、错误和 envelope：能快速隔离目录，但公共语义会漂移，无法可靠生成多语言 SDK，因此不采用。
- 立即把现有 Runtime Gateway 全量升级到 gateway v1：可移除 compat profile，但会把设备 registry、event cursor store 和网络 session 等未实现能力混入本阶段，并扩大双链路回归面，因此不采用。
- 把 Desktop IPC adapter 直接实现为 Provider plugin：会让 Provider event 有机会进入桌宠投影，破坏已验证的 remote/companion channel isolation，因此不采用。
- 直接采用 quicktype Dart renderer：跨文件 ref 可用，但会丢失约束、共享命名、closed object、敏感字段和判别联合，因此不采用。
- 生成 annotated Dart source 再运行 json_serializable/Freezed：codec 生态成熟，但会增加第二阶段生成链、运行依赖和一份 Dart 侧模型事实，因此本阶段不采用。
- 把 Gateway IDL 转为 OpenAPI 后使用 dart-dio：oneOf 支持较强，但会把 transport-defined WebSocket envelope 错配为 HTTP client，并新增 OpenAPI 映射事实，因此不采用。

## 取舍理由

JSON Schema Draft 2020-12 加 method/event manifest 能同时表达跨语言 DTO、版本、方向、幂等性、capability、transport 和 discriminator，且现有生成逻辑已有成熟基础。独立 Rust SDK 让 Provider Host、Gateway、Provider binary 与桌面应用通过普通 crate dependency 使用生成接口，而不是复制代码。compat v0 module 把迁移风险集中在可删除边界：现有 serde fixtures 和调用形状保持不变，而实际 runtime 已收敛到 Provider/Gateway v1。

把 `RoutedResourceId` 置于 core 是一个有意的最小共享选择：它只组合稳定 ID，不携带 Provider conversation、Pet task 或 Gateway session 状态，因而可安全被 provider 与 gateway 同时引用。所有更高层的资源、生命周期和 UI 字段仍留在各自 schema。

## 影响范围

- 协议改动必须先修改 `protocol/`，运行生成器，再提交生成物和 fixture。
- `tools/protocol-codegen` 必须拒绝未声明依赖、未知 schema 关键字与 stale output。
- target adapter 未实现、没有 package output 或 interface/status 不一致时必须在写入前失败。
- normalized IR 是新 target 的 typed compiler boundary；Dart DTO、route、metadata、codec 与 typed client 必须从同一 IR 生成。现有 Rust/TypeScript emitter 迁移前仍以 validated model 保持兼容，不能借迁移改变 wire。
- Dart core/gateway package 不依赖 Flutter；Gateway 只依赖 core，消费者负责 transport、TLS、credential、重连、请求关联与事件持久化。
- Rust 业务 crate 依赖 `codepet-*-sdk`；不得新增 `codepet-*-protocol` crate。
- Provider Host 使用生成的有界 JSON-line codec 与统一 wire classifier，不自行实现另一套宽松 response/notification parser。
- Provider Host 使用生成的 `ProtocolRequest::from_method_params` 构造 JSON-RPC request enum，不在 Host 维护第二份 method/envelope 枚举。
- Provider 事件与 Pet 事件使用不同生成 enum 和 server/client contract，不能通过同一个 event sink 互换。
- 新远程资源必须包含 device、provider plugin、provider instance 与 native resource 四段 identity；单独 native ID 只允许在已经绑定并逐跳校验完整 route 的 Provider 内部使用。
- Python generator 可以后加，但只能消费 `protocol/codegen.json` 声明的同一接口和 normalized IR。

## 后续观察

- Provider Host 已使用 generated `ProtocolClient` 覆盖 initialize、describe、instance lifecycle、业务请求与 shutdown；进程启动、超时和生命周期监管仍属于 Host，而不是协议 SDK。
- `codepet-provider-codex` 与 `codepet-provider-claude` 已使用 generated `ProtocolServer`、dispatcher 和 JSON-line codec 实现 production Provider v1；各自的 App Server / CLI 私有协议只存在于对应插件内部。
- Gateway v1 transport 对 event cursor、分页 cursor、断线恢复和版本不重叠错误的实现是否与 IDL 一致。
- compat v0 使用点是否持续收敛；在 Pet v1 adapter 与真实 gateway v1 session 均完成前，不应提前删除。
- 如出现 schema 子集不足，应先评估所有目标语言的可生成性，再扩展 generator；不得为单个 Rust 需求加入只能由 serde 表达的语义。
- 四个 Rust SDK 已移除永久私有标记，并为 path dependency 同时声明版本。仓库尚无 LICENSE，正式发布前必须由所有者决定许可证，不能由实现阶段猜测。
- Dart package 当前版本为 `0.1.0`，Gateway 用 `dependency_overrides` 指向同仓 core 进行本地验证；发布时必须先发布同版本 core，并移除消费方的本地 override。仓库 LICENSE 决策同样仍是发布阻塞项。
- 当前 Dart 不生成 gateway compat-v0、Pet 或 Provider JSON-RPC；未来扩展前必须为相应 transport semantics 增加 IR 与 fixture 证据，不能机械复制 Gateway v1 emitter。
- Rust/TypeScript 尚未改为完全从 normalized IR 渲染；这是生成器内部一致性的剩余迁移项，不影响 `protocol/` 作为唯一手写事实，但迁移时必须先锁定当前生成 diff 与全部 SDK fixture。

## 工具来源

- quicktype 官方仓库与 CLI：<https://github.com/glideapps/quicktype>
- json_serializable：<https://pub.dev/packages/json_serializable>
- Freezed：<https://pub.dev/packages/freezed>
- OpenAPI Generator dart-dio：<https://openapi-generator.tech/docs/generators/dart-dio/>
