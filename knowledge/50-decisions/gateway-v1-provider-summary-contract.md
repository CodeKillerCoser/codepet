# Gateway V1 使用精简 Provider Summary

## 背景

旧 Gateway V1 握手同时暴露 `device`、`devices`、Provider plugin identity、instance route、Harness descriptor、Provider plugin version 和完整 capabilities。Remote 只需要展示当前 Host 与可用 Agent runtime，这些内部层级既增加握手体积，也迫使 Remote 理解 Host 的 plugin/instance/harness 组合方式。项目尚未正式发布，因此直接修改现有 V1，不保留旧字段兼容或双读双写。

## 决策

Gateway V1 握手固定为 `protocol/device/providers/eventCursor` 四部分。`device` 只含 `name/operatingSystem/systemVersion`；稳定 device ID 和证书指纹属于 LAN admission/transport。

每个 `ProviderSummary` 只暴露：

- opaque `id`：Remote 后续调用 Provider 级接口的唯一标识；Host 内部解析为 plugin/instance/route。
- `identity`：`displayName`、可选 HTTPS icon，以及 Provider 报告的可选 `defaultWorkspaceRoot`。该字段表示无项目任务的托管父目录；Remote 为每次创建生成唯一子目录，Provider 在创建会话前负责建立实际目录。项目会话只发送项目引用，不重复发送路径。
- `runtime`：状态、实际 runtime version/executable path、可选 authentication 和 usage 摘要。
- `capabilities.revision`：完整能力通过 `provider.describe({ providerId })` 按需读取；响应同时返回最新 `ProviderSummary`，避免摘要与能力来自不同快照语义。

所有 Remote-facing Provider 选择统一使用必填 opaque `providerId`。所有 Project、Conversation、Turn、Approval 等资源引用统一为 `{ providerId, nativeResourceId }`，Gateway V1 不暴露 `deviceId/providerPluginId/providerInstanceId` 组合 route。Provider stdio 协议保留私有完整路由类型，Host 只在 Gateway 边界做双向解析。

Authentication 只允许 `unknown/signed-in/signed-out/expired/error/unsupported` 与可选展示文本，不传 email、account ID、plan 或凭据。`runtime.usage` 保留 Provider 用量概览，包含 displayText、observedAt 和带 namespace/schemaVersion 的详情。Host 转发时递归脱敏，每条详情最多 16 KiB、合计最多 32 KiB、最多 8 条。它与强类型 `codepet.usage.query` 是独立能力；后者由 `usageDatasets` 广告，提供筛选、聚合和分页。详见 [用量与存储](../10-architecture/provider-usage-query-and-storage.md)。

握手和 `provider.changed` 事件复用同一 `ProviderSummary`。Host 保存最新可重建 runtime snapshot；实例快照任一字段变化都推送完整 summary，完全相同的重复快照不推送。握手先读取 event cursor，再读取 Provider snapshot，使并发变化要么已反映在 snapshot，要么能从该 cursor replay。

## 备选方案

- 保留旧扁平 `ProviderInstance`：兼容成本低，但继续泄露 plugin/instance/harness 与组合 route，且完整能力会放大握手。
- 新建 Gateway V2：能保留 V1，但项目没有线上兼容负担，会产生无价值的双协议维护。
- 用详情查询替代 runtime usage：不采用。Provider 概览和筛选后的用量明细用途不同，必须同时保留；额度和成本不混入 token 指标。

## 取舍理由

Remote 只依赖稳定 Provider ID、展示身份、当前 runtime 状态和能力 revision。将内部 route 留在 Host，可以独立演进 Provider 实现；将完整能力移出握手，可以避免模型/权限选项放大首包。用量明细按需查询，runtime 快照只携带有界概览，不承担历史明细分页。

## 影响范围

- `protocol/core/v1`、`protocol/gateway/v1` 与生成的 Rust/Dart SDK：直接替换 V1 handshake、opaque resource ID、Provider list/describe 和 changed event。
- `protocol/provider/v1`：Provider instance snapshot 增加实际 executable path、authentication 来源字段，并拥有不对 Remote 暴露的完整 resource route 类型。
- `crates/codepet-host`：维护内部 ID→route 映射、summary 投影、事件与握手 replay 边界。
- `src-tauri`：构造 transport identity，并让进程内旧 Desktop gateway 仅通过 Host 内部 route resolver 获取路由，不把组合 route重新放回 Gateway V1。
- `codepet-remote`：必须与 Host 同步升级到同一 V1；不提供旧字段兼容。

## 后续观察

- 各 Provider 需要从真实 runtime 填充 authentication，并在原生信息变化时发送完整 instance snapshot；未知时字段应缺省，不能伪造成功状态。
- executable path 只发送给已配对授权的 Remote，不能进入发现广播或日志。
- `npm run protocol:check`、Dart SDK tests、Host manager/LAN/full integration tests共同守住 schema、生成物、映射、事件和连接行为。
