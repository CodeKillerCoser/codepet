# 使用语言无关分层 IDL 与独立 SDK 包

## 背景

旧 v0 由一份 JSON Schema 同时描述 Runtime Gateway 的 Provider、conversation、turn、approval 与远程事件，Rust 生成物直接放在 Tauri 业务模块。该方式已证明 schema/manifest/codegen/fixture 流程可行，却无法清晰区分 Desktop Companion、独立 Provider 插件和 Remote Client，也没有设备与 Provider instance 的全局路由身份。

## Dart 工具证据

在引入 Dart target 前，使用当前 `quicktype 26.0.0` 对 Gateway Draft 2020-12 schema 做了真实 CLI 实验。它可以生成 null-safe Dart，并能沿相对路径解析 Gateway→Core `$ref`；Apache-2.0 许可证和生成代码无额外知识产权限制也符合仓库使用前提。但实验输出把共享 `DeviceDescriptor`、`VersionRange` 重命名为使用点局部类，忽略 `minLength/minimum/maximum/pattern`、`additionalProperties: false` 与 `x-codepet-sensitive`，并把 `ModelCatalog.oneOf` 合并为同时带多个可选字段的单一 class，无法保持 `kind` 分支互斥。该结果不能安全解码 hostile LAN/WSS 输入。

`json_serializable 6.14.x` 能开启 unknown-key 检查，Freezed 4.x 能表达 tagged union，但两者都要求先维护带 annotation 的 Dart class，再由 Dart build runner 生成辅助代码；这会把 Dart class 变成第二份模型事实。OpenAPI Generator 的稳定 `dart-dio` target 支持 discriminator/oneOf，但输入边界与输出重点是 OpenAPI HTTP/Dio client；CodePet 当前是 JSON Schema + 自定义 method/event manifest + transport-defined envelope，把 IDL 转成 OpenAPI 会新增一层有语义的中间事实和不需要的 HTTP runtime。

因此现阶段不存在能直接保留 CodePet 有限 schema/manifest 全语义、同时又不产生第二事实源的成熟 drop-in generator。重新评估第三方工具的门槛是：能消费规范化 IR 或原始 IDL、保留稳定类型身份与所有受检约束、允许自定义 method/event service 层，并能在 unsupported semantics 上 fail closed。

## 决策

协议长期事实只存在于 `protocol/` 的语言无关 IDL，并按 `core`、`agent`、`pet`、`provider`、`gateway` 分层独立版本化。Agent 是 Provider/Gateway 共同引用的 types-only 领域包，不是 transport 或 service。生成包统一命名 `codepet-*-sdk` 并放在 `sdk/<language>/`；Rust struct、Tauri DTO 和任何语言侧 model 都不能成为第二事实来源。

Provider v1 固定为 Host ↔ 独立 Provider 二进制的 JSON-RPC 2.0/stdio 协议，拥有插件描述与 instance 生命周期。Gateway v1 固定为 Host ↔ Remote Client 的 JSON-RPC 2.0 业务协议，只暴露设备、Provider instance、远程资源与 event cursor，不暴露插件进程控制。LAN 的 discovery/channel/admission DTO 独立位于 `channel/lan/v1`；证书指纹、pairing secret 和 bearer 不进入 Gateway 业务 service。Pet v1 固定为 Desktop Companion 驱动的任务、审批、动作、快照与 patch，不引用 Provider 领域对象。

Provider SDK 的公共边界由两部分组成：schema/manifest 生成的 DTO、method/event enum、server trait、dispatcher 与 typed client，以及各语言稳定维护的 transport runtime。Rust runtime 统一拥有 JSON-RPC 2.0/stdio JSON-lines reader、串行 writer、event sink、普通/控制双通路、过载拒绝、frame limit、EOF/fatal shutdown 与有界 drain；Provider 实现不得复制这些机制。`dispatchLane` 是 manifest 事实，`instance.stop`、`instance.destroy` 与 `provider.shutdown` 生成到 control lane，runtime 不维护第二份 method 分类表。

公开 CLI `cp-sdk-gen` 复用 JavaScript normalized-IR 协议编译器，并由 Bun `--compile` 构建为随 App 分发的单体原生 executable。运行时以 schema、manifest 和 fixtures 为输入，执行引用解析、完整校验、IR 构建与 language emitter；Rust package scaffold、稳定 stdio runtime 和接入说明作为编译器资产内嵌，但生成的 DTO、trait、method/event、codec 必须现场从协议输入产生，不能嵌入 checked-in `generated.rs`。输出写入 generator/protocol/runtime 版本与输入 digest。最终 executable 不依赖 Bun、Node 或源码仓库。首个 Provider server target 只实现 Rust；Dart 当前用于 Gateway/Remote client，不因为仓库使用 Dart 就生成没有消费场景的 Provider server runtime。未实现语言继续 fail closed。

每种输出语言必须通过 `codepet.protocol.codegen/v1` target adapter registry 显式接入。校验后的 schema/manifest 先归一化成唯一 typed IR；Dart 的 definition、constraint、union discriminator、method、event、capability 和 transport metadata 只能从该 IR 投影，不得维护第二份 model/method/event 表。Rust、TypeScript、Dart 是已实现 adapter；现有 Rust/TypeScript emitter 暂保留 validated model 输入，迁移到 normalized IR 必须另做等价 fixture/diff 验证。Python 在实现前保持 planned 且选择即失败，不能落入另一个语言 generator 或静默跳过。schema 与 manifest 的 `$ref` 使用同一依赖审计，capability type/container/method mapping 也是生成器受检契约。

Dart target 采用只覆盖当前受检 schema 子集的结构化 emitter，不引入 quicktype、Freezed、json_serializable 或 build_runner 依赖。它生成 Core、Agent、Gateway v1 client 与 LAN admission models null-safe package、closed-object/约束校验、显式 sealed union、敏感字段 redaction、manifest metadata、JSON-RPC codec 与 transport-neutral typed client。无法找到共同 required singleton-enum discriminator 的 `oneOf` 在生成前失败；不会退化为 `dynamic` 或宽松可选字段模型。

既有 Tauri Runtime Gateway 调用面使用同一 SDK 内生成的 `desktop_v0` module。该 profile 来自 `protocol/desktop/v0`，不允许在 Tauri 源码复制 DTO；它是独立的桌面内部协议，不再占用 Gateway 版本空间。Remote 网络 transport 只使用 Gateway v1 JSON-RPC。

## 备选方案

- 继续以 Tauri Rust struct 为主并导出其他语言：短期简单，但 Rust 特性会决定 wire 语义，并形成与 JSON Schema 并列的事实来源，因此不采用。
- 为 pet/provider/gateway 各自手写重复 ID、错误和 envelope：能快速隔离目录，但公共语义会漂移，无法可靠生成多语言 SDK，因此不采用。
- 立即把 Desktop 内部协议改成 Gateway v1：会把 Desktop Companion/Pet 的本地状态语义误并入远程 Gateway，因此不采用；本次只把它移出 Gateway 命名空间。
- 把 Desktop IPC adapter 直接实现为 Provider plugin：会让 Provider event 有机会进入桌宠投影，破坏已验证的 remote/companion channel isolation，因此不采用。
- 直接采用 quicktype Dart renderer：跨文件 ref 可用，但会丢失约束、共享命名、closed object、敏感字段和判别联合，因此不采用。
- 生成 annotated Dart source 再运行 json_serializable/Freezed：codec 生态成熟，但会增加第二阶段生成链、运行依赖和一份 Dart 侧模型事实，因此本阶段不采用。
- 把 Gateway IDL 转为 OpenAPI 后使用 dart-dio：oneOf 支持较强，但会把 transport-defined WebSocket envelope 错配为 HTTP client，并新增 OpenAPI 映射事实，因此不采用。

## 取舍理由

JSON Schema Draft 2020-12 加 method/event manifest 能同时表达跨语言 DTO、版本、方向、幂等性、capability、transport 和 discriminator，且现有生成逻辑已有成熟基础。独立 Rust SDK 让 Provider Host、Gateway、Provider binary 与桌面应用通过普通 crate dependency 使用生成接口，而不是复制代码。桌面 v0 module 保持本地调用形状不变；Gateway runtime 已唯一收敛到 v2。

把两段 `RoutedResourceId { providerId, nativeResourceId }` 置于 Core 是一个有意的最小共享选择：它只组合 opaque Provider identity 与 native identity，不携带 Provider process route、Pet task 或 Gateway session 状态。Project、Conversation、Turn、Approval 和消息/工具对象位于 Agent；Provider 私有四段请求资源位于 Provider。详细取舍见 `shared-agent-domain-between-provider-and-gateway.md`。

## 影响范围

- 协议改动必须先修改 `protocol/`，运行生成器，再提交生成物和 fixture。
- `tools/protocol-codegen` 必须拒绝未声明依赖、未知 schema 关键字与 stale output。
- target adapter 未实现、没有 package output 或 interface/status 不一致时必须在写入前失败。
- normalized IR 是新 target 的 typed compiler boundary；Dart DTO、route、metadata、codec 与 typed client 必须从同一 IR 生成。现有 Rust/TypeScript emitter 迁移前仍以 validated model 保持兼容，不能借迁移改变 wire。
- Dart Core/Agent/Gateway package 不依赖 Flutter；Agent 只依赖 Core，Gateway 依赖 Core 与 Agent，消费者负责 transport、TLS、credential、重连、请求关联与事件持久化。
- Rust 业务 crate 依赖 `codepet-*-sdk`；不得新增 `codepet-*-protocol` crate。
- Provider Host 使用生成的有界 JSON-line codec 与统一 wire classifier，不自行实现另一套宽松 response/notification parser。
- Provider Host 使用生成的 `ProtocolRequest::from_method_params` 构造 JSON-RPC request enum，不在 Host 维护第二份 method/envelope 枚举。
- Provider binary 使用 `codepet-provider-sdk::serve_stdio` 与 typed `ProviderEventSink`；业务 crate 只实现生成 `Provider` trait 和上游 harness adapter，不读取 stdin、不写 stdout、不维护 request queue。
- `cp-sdk-gen --package <package> --role <role> --lang <language> --output <sdk-dir>` 按需导出 client、server 或 models SDK 与 `cp-sdk-gen.lock.json`；`--check` 对已导出目录执行 freshness 检查，不删除目标目录中的其他文件。
- App Resources 必须同时携带 `provider-sdk/codepet-sdk.json`、兼容索引、完整接入 README、canonical `protocol/{core,agent,provider,gateway,channel}` 资源树与平台原生 `cp-sdk-gen(.exe)`；分发视图内的 JSON Schema `$ref` 必须可直接解析。只打包内置 Provider binary 不算完成 SDK 分发。
- Provider 事件与 Pet 事件使用不同生成 enum 和 server/client contract，不能通过同一个 event sink 互换。
- 新 Remote 资源必须包含两段 opaque identity；Host→Provider 请求必须使用四段 `ProviderResourceId`。Host 负责解析和逐跳校验，不能把 Remote ID 当作进程路由，也不能只传 native ID。
- 新语言 generator 可以后加，但只能消费 `protocol/codegen.json` 声明的同一接口和 normalized IR；没有实际 Provider server 消费者时不因示例语言提前扩张 target。

## 后续观察

- Provider Host 已使用 generated `ProtocolClient` 覆盖 initialize、describe、instance lifecycle、业务请求与 shutdown；进程启动、超时和生命周期监管仍属于 Host，而不是协议 SDK。
- Codex、Claude 与 OpenCode 三个内置 Provider 已统一使用公共 Rust Provider runtime：入口只构造各自 Provider 并调用 `serve_stdio`；App Server、CLI stream-json、HTTP/SSE client、mapper 与业务生命周期仍完全属于各自 adapter。
- `scripts/test_codex_provider_stdio.py` 从独立 Python 进程执行公共 wire smoke；SDK 的通用 transport 行为仍由 Rust SDK tests 与 Codex 饱和/EOF/坏帧/断管纵向测试共同守护。
- `codepet-provider-codex`、`codepet-provider-claude` 与 `codepet-provider-opencode` 已实现 generated `Provider` trait，并共享 SDK dispatcher、JSON-line codec 与 stdio runtime；各自的 App Server / CLI / HTTP+SSE 私有协议只存在于对应插件内部。
- Gateway v1 transport 对 event cursor、分页 cursor、断线恢复和版本不重叠错误的持续兼容性。
- desktop v0 使用点是否持续收敛；它只能服务 Desktop Companion/Pet 本地链路，不得重新成为 Remote Gateway 兼容层。
- 如出现 schema 子集不足，应先评估所有目标语言的可生成性，再扩展 generator；不得为单个 Rust 需求加入只能由 serde 表达的语义。
- 六个 Rust SDK 已移除永久私有标记，并为 path dependency 同时声明版本。仓库尚无 LICENSE，正式发布前必须由所有者决定许可证，不能由实现阶段猜测。
- Dart package 当前版本为 `0.1.0`，Gateway 用 `dependency_overrides` 指向同仓 core 进行本地验证；发布时必须先发布同版本 core，并移除消费方的本地 override。仓库 LICENSE 决策同样仍是发布阻塞项。
- 当前 Dart 不生成 gateway desktop-v0、Pet 或 Provider server；未来扩展前必须为相应 transport semantics 增加 IR 与 fixture 证据，不能机械复制既有 emitter。
- Rust/TypeScript 尚未改为完全从 normalized IR 渲染；这是生成器内部一致性的剩余迁移项，不影响 `protocol/` 作为唯一手写事实，但迁移时必须先锁定当前生成 diff 与全部 SDK fixture。

## 工具来源

- quicktype 官方仓库与 CLI：<https://github.com/glideapps/quicktype>
- json_serializable：<https://pub.dev/packages/json_serializable>
- Freezed：<https://pub.dev/packages/freezed>
- OpenAPI Generator dart-dio：<https://openapi-generator.tech/docs/generators/dart-dio/>
