# Provider / Gateway v2 前向兼容规则

本目录是当前启用的完整功能 IDL，`protocol/codegen.json.sourceVersion=2`。v1 源码与生成视图冻结；Rust、TypeScript、Dart 使用当前 v2 输出，Provider 公开 SDK 按 v2 发布，Gateway 保持内部使用。业务首发范围是 2/2，不声明兼容旧严格 v1。

## version 与 minVersion

两层分别维护连续区间 `[minVersion, version]`：version 是当前实现的最高业务修订号，minVersion 是最低仍能正确理解和执行的修订号；区间内不能有空洞。两者为正整数，且 minVersion ≤ version。不再携带 maxVersion 或 supportedVersions。软件版本仍在 hostVersion、clientVersion、插件 descriptor.version，不能与 protocol.version 混用。

Provider Initialize 与 Gateway handshake 的请求都在 `protocol` 中发送本端范围；响应的 `protocol` 携带响应方范围和 selectedVersion。选择公式为：

```text
selectedVersion = min(local.version, peer.version)
compatible = selectedVersion >= max(local.minVersion, peer.minVersion)
```

例如 Host `{version:5,minVersion:2}` 和插件 `{version:3,minVersion:2}`，选择 3；`{version:5,minVersion:4}` 对 `{version:3,minVersion:2}` 无交集，初始化失败。请求方重新计算并验证响应，不能只相信 selectedVersion。Provider 返回的 plugin.protocol 必须与响应 protocol 的 version/minVersion 一致。Gateway 的协议范围属于 Host，不属于其 Provider 清单。

握手必须使用稳定、允许未知字段的基础信封。连接选定版本后不在心跳里重新协商；能力目录可以更新，但不能暗中改变连接版本。协商失败不得创建业务实例或提交写操作。v1 的旧 VersionRange 继续服务旧协议，不全局修改它，以免影响 desktop/v0 等调用者。

## 升级规则

- 说明、目录、生成器优化或符合既定合同的实现修复：不改业务版本。
- 新增可选字段、方法、能力、事件或已声明可扩展的枚举/联合分支：增加 version，保持 minVersion。新增字段缺失须有明确且保持旧行为的处理；新方法/行为必须由能力声明及调用方支持共同决定。可解码不表示客户端能理解所有新语义。
- 某插件开始实现已有方法，或模型/权限/推理选项发生变化：不改业务版本，更新能力与 capabilityRevision。
- 删除/重命名字段、改变类型或必填性、收紧到排除旧合法值、改变既有分页/事件顺序/幂等语义：默认增加 version 并提高 minVersion。若保留旧行为适配，可保留旧下界，但须有测试证明。
- 安全关键枚举/联合若无法为旧端提供保守且可用的降级，不能仅按“新增分支”处理，应能力门控或提高 minVersion。
- 传输帧、压缩、mux、profile ID 的变化使用传输版本；安装清单变更使用 manifestVersion；SDK 布局变更使用其发布格式版本。它们不因业务 version 改变而自动升级。

## 接收与发送规则

未知字段忽略，已知必填字段缺失、类型不符、约束不合法仍报错；身份、授权和跨实例校验保持。未知字段与未知载荷仍计入大小、深度和队列限制，不能因“忽略”绕过预算。未知数据默认不写日志或原样回显。

能力数组保留可选的原始名称，但业务仅调用本端认识且对端明确声明支持的能力；不能把本地解码兜底当作支持。未知 RPC 方法返回标准 -32601，不断开正常连接；通知事件不回 RPC 错误。

未知事件先验证基础信封，再跳过内容；如果 Gateway 信封带合法 cursor，继续推进该事件流的恢复位置，避免同一事件无限重放。不把未知事件投影成任务或触发动作。已知事件的非法字段不是“未知事件”。

字符串枚举保留 unknown(raw) 表示；判别联合保留 Unknown(kind, raw) 表示。已知 kind 的非法载荷继续报错，不能退回 Unknown 掩盖数据错误。未知运行状态按未知/不可操作处理，不能推断空闲、完成或自动派发。授权、安全决策和任务执行保持保守；需要新的语义时由能力控制或提高 minVersion。

发送端只主动编码本端实现的字段与能力，不向旧端强推依赖新字段才能理解的行为。解析器可以保留未知枚举以供展示，但普通业务请求编码器不能自动发送收到的未知联合；扩展透传须是显式合同。旧端对新可选展示信息的忽略必须不影响原操作语义。

写请求断线或超时仍保留结果未知语义，不能因为升级握手、未知响应或版本不匹配自动重发。

## 首次迁移

现有 v1 使用 supportedVersions/minVersion/maxVersion、Provider selectedVersion 和严格对象/枚举解码；它不能直接接收v2。因此v2首次业务版本为 2、minVersion 为 2。后续 v3、v4 的纯增量扩展可继续 minVersion=2。不能把“新端能处理旧字段”误当成双向兼容。

若决定继续支持 v1，需要明确旧入口或安装配置选择的 v1 adapter，并验证两套握手；不能在同一连接猜字段、先发送新握手再静默回退，更不能重放已提交业务。Provider 三 profile、CPMX 与 Gateway 通道保持各自既有 transport 版本，不将业务握手改名误当成传输升级。

v2 保留自己的完整模块快照并生成依赖闭包，避免运行时回读可变的 v1 类型。旧 VersionRange 仅为 pet/v1、desktop/v0 与独立传输握手保留，不能拿来协商 v2 业务版本。

## 代码生成与运行时边界

当前实现与验证边界如下：

1. 加载器与规范化 IR：支持独立服务 version/minVersion、兼容策略和可扩展枚举/联合；将输入层目录版本与每层业务版本解耦。识别并校验 minVersion ≤ version 等跨字段约束，未实现的策略明确报“不支持”，不能输出看似兼容的代码。
2. Rust：按接收策略替换业务 DTO 的 deny_unknown_fields，已知字段校验保留；为字符串枚举、联合和 capability 生成 Unknown 变体，保留原值。dispatcher 分开处理未知请求、未知通知和非法已知数据。
3. TypeScript：类型联合和实际 decoder 一起生成未知值表示；不能只给 interface 添加字符串兜底，却仍在运行时拒绝。发送 API 保持受支持类型，接收类型允许扩展。
4. Dart：移除未知对象字段的硬拒绝，为枚举/联合生成带原值的 Unknown 类型，并让 UI switch 显式处理；保留 required、范围、格式和判别唯一性校验。
5. 握手与会话：生成 offer/selection DTO、版本常量与协商校验；Provider 的响应和 descriptor 一致，Gateway 每连接选一次。Host/SDK/客户端同步适配后才启用。
6. 事件与能力：未知事件分类、合法 cursor 推进、未知 capability 过滤和安全兜底全部进入 SDK 测试，不在 App 各写一遍。
7. 发布和生成：v1 兼容视图继续冻结；v2 输出不能覆盖 v1 文件。Provider 公开 SDK 只导出 Provider 依赖闭包，Gateway 留内部；两仓锁文件记录来源，但摘要不代替兼容判断。
8. 验证：区间交集/无交集/非法选择、未知字段、未知事件与 cursor、未知枚举/联合、已知 kind 非法载荷、非法基础信封、预算、重复初始化、无写重试，以及 Rust/TS/Dart 同 fixtures。验证 v2 端协商及旧 1/1 业务范围明确拒绝；不把未实现的 v1 adapter 算成已兼容；minVersion 不能只在文档里承诺。

版本范围只能帮助双方选择合同；Provider 支持 Agent、Observation 或运行时管理等能力的设计仍应独立完成，不借本次握手草案隐式新增业务功能。
