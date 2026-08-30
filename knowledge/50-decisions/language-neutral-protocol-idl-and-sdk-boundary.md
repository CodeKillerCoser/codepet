# 使用语言无关分层 IDL 与独立 SDK 包

## 背景

旧 v0 由一份 JSON Schema 同时描述 Runtime Gateway 的 Provider、conversation、turn、approval 与远程事件，Rust 生成物直接放在 Tauri 业务模块。该方式已证明 schema/manifest/codegen/fixture 流程可行，却无法清晰区分 Desktop Companion、独立 Provider 插件和 Remote Client，也没有设备与 Provider instance 的全局路由身份。

## 决策

协议长期事实只存在于 `protocol/` 的语言无关 IDL，并按 `core`、`pet`、`provider`、`gateway` 四层独立版本化。生成包统一命名 `codepet-*-sdk` 并放在 `sdk/<language>/`；Rust struct、Tauri DTO 和未来 Dart/Python model 都不能成为第二事实来源。

Provider v1 固定为 Host ↔ 独立 Provider 二进制的 JSON-RPC 2.0/stdio 协议，拥有插件描述与 instance 生命周期。Gateway v1 固定为 Host ↔ Remote Client 协议，只暴露设备、Provider instance、远程资源与 event cursor，不暴露插件进程控制。Pet v1 固定为 Desktop Companion 驱动的任务、审批、动作、快照与 patch，不引用 Provider 领域对象。

每种输出语言必须通过 `codepet.protocol.codegen/v1` target adapter registry 显式接入。Rust、TypeScript 是已实现 adapter；Dart/Python 在实现前保持 planned 且选择即失败，不能落入另一个语言 generator 或静默跳过。schema 与 manifest 的 `$ref` 使用同一依赖审计，capability type/container/method mapping 也是生成器受检契约。

既有 Tauri Runtime Gateway 调用面使用同一 gateway SDK 内的生成 compat v0 module。compat profile 仍来自 `protocol/gateway/v1` 下的 IDL，不允许在 Tauri 源码复制 DTO。它现在薄适配到 Host 的 Gateway v1 application service；Provider v1 已用于真实插件进程，但这仍不代表 Gateway v1 网络 transport 已实现。

## 备选方案

- 继续以 Tauri Rust struct 为主并导出其他语言：短期简单，但 Rust 特性会决定 wire 语义，并形成与 JSON Schema 并列的事实来源，因此不采用。
- 为 pet/provider/gateway 各自手写重复 ID、错误和 envelope：能快速隔离目录，但公共语义会漂移，无法可靠生成多语言 SDK，因此不采用。
- 立即把现有 Runtime Gateway 全量升级到 gateway v1：可移除 compat profile，但会把设备 registry、event cursor store 和网络 session 等未实现能力混入本阶段，并扩大双链路回归面，因此不采用。
- 把 Desktop IPC adapter 直接实现为 Provider plugin：会让 Provider event 有机会进入桌宠投影，破坏已验证的 remote/companion channel isolation，因此不采用。

## 取舍理由

JSON Schema Draft 2020-12 加 method/event manifest 能同时表达跨语言 DTO、版本、方向、幂等性、capability、transport 和 discriminator，且现有生成逻辑已有成熟基础。独立 Rust SDK 让 Provider Host、Gateway、Provider binary 与桌面应用通过普通 crate dependency 使用生成接口，而不是复制代码。compat v0 module 把迁移风险集中在可删除边界：现有 serde fixtures 和调用形状保持不变，而实际 runtime 已收敛到 Provider/Gateway v1。

把 `RoutedResourceId` 置于 core 是一个有意的最小共享选择：它只组合稳定 ID，不携带 Provider conversation、Pet task 或 Gateway session 状态，因而可安全被 provider 与 gateway 同时引用。所有更高层的资源、生命周期和 UI 字段仍留在各自 schema。

## 影响范围

- 协议改动必须先修改 `protocol/`，运行生成器，再提交生成物和 fixture。
- `tools/protocol-codegen` 必须拒绝未声明依赖、未知 schema 关键字与 stale output。
- target adapter 未实现、没有 package output 或 interface/status 不一致时必须在写入前失败。
- Rust 业务 crate 依赖 `codepet-*-sdk`；不得新增 `codepet-*-protocol` crate。
- Provider Host 使用生成的有界 JSON-line codec 与统一 wire classifier，不自行实现另一套宽松 response/notification parser。
- Provider Host 使用生成的 `ProtocolRequest::from_method_params` 构造 JSON-RPC request enum，不在 Host 维护第二份 method/envelope 枚举。
- Provider 事件与 Pet 事件使用不同生成 enum 和 server/client contract，不能通过同一个 event sink 互换。
- 新远程资源必须包含 device、provider instance 与 native resource 三段 identity；单独 native ID 只允许在已绑定 route 的内部 adapter 中使用。
- Dart/Python generator 可以后加，但只能消费 `protocol/codegen.json` 声明的同一接口和 IDL。

## 后续观察

- Provider Host 已使用 generated `ProtocolClient` 覆盖 initialize、describe、instance lifecycle、业务请求与 shutdown；进程启动、超时和生命周期监管仍属于 Host，而不是协议 SDK。
- `codepet-provider-codex` 已使用 generated `ProtocolServer`、dispatcher 和 JSON-line codec 实现 production Provider v1；App Server 私有协议只存在于该插件内部。
- Gateway v1 transport 对 event cursor、分页 cursor、断线恢复和版本不重叠错误的实现是否与 IDL 一致。
- compat v0 使用点是否持续收敛；在 Pet v1 adapter 与真实 gateway v1 session 均完成前，不应提前删除。
- 如出现 schema 子集不足，应先评估所有目标语言的可生成性，再扩展 generator；不得为单个 Rust 需求加入只能由 serde 表达的语义。
- 四个 Rust SDK 已移除永久私有标记，并为 path dependency 同时声明版本。仓库尚无 LICENSE，正式发布前必须由所有者决定许可证，不能由实现阶段猜测。
