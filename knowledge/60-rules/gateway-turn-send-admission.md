# Gateway turn.send 受理与去重规约

## 规则

Gateway `turn.send` 只为已有空闲 conversation 启动新 turn。立即响应中的 `userItem` 可以为 `null`；去重只能描述为按调用方隔离、当前进程与缓存窗口内的 best-effort，不能声明 exactly-once 或无条件幂等。

## 适用场景

适用于 Provider/Gateway IDL、`ProviderGatewayService`、Remote WSS dispatch、Tauri compat、Provider capability 目录，以及 Remote 客户端对发送结果未知时的恢复策略。

## 反例

- Codex App Server 已接受 `turn/start`，但官方响应为 `items=[]`；若 Host 强制要求 user message，会向客户端报失败，造成“实际已发送但看起来失败”。
- 多个 model 的 reasoning effort 做并集后，客户端可能为选中 model 发送其不支持的 effort。
- 全局缓存只用 route 与 `clientRequestId`，两个已认证 Remote 会互相命中或冲突。
- 把 1,024 条进程内缓存写成协议幂等保证，会诱导客户端在 Host 重启、缓存淘汰或网络结果未知时盲目重试。

## 推荐做法

- Provider start ack 始终返回权威 turn；仅在 ack 确实携带 user message 时返回对象，否则返回 `userItem: null`。对象仍需校验 route、conversation、turn、角色和内容；真实 item 依赖通知与后续 snapshot 收敛。
- 不能表达 per-model reasoning 时，只广告所有可见 model 的 enabled effort 交集；默认值不在交集时省略，交集为空时省略整个 control。
- 去重键包含稳定的 opaque caller scope。WSS 使用已认证逻辑 `clientId` 的命名空间，同一 Remote 重连复用；不同 Remote 隔离。local/compat 使用独立固定 scope，不传 bearer。
- manifest 将 `turn.send` 标为 `nonIdempotent`。结果未知时先刷新 conversation 与 live state；显式复用同 ID 只能作为当前进程/缓存窗口内的 best-effort retry。
- Gateway 与 compat 只消费协议 discriminator 和 capability，不按 Provider、plugin 或 harness 名称推断 catalog shape 或 steering。

## 来源

2026-09-01 对提交 `1685f2c` 的独立审查发现：Codex 官方 start ack 可为空 items、reasoning 支持属于单 model、Gateway cache 被所有 WSS 共享，以及有界内存缓存不足以支撑协议级幂等声明。

## 验证方式

- Codex fixture 以 `turn/start.items=[]` 返回并仍得到 accepted；后续通知/snapshot 路径保持可用。
- Codex mapper 覆盖多 model 不同 effort 的交集、空交集省略和非广告 effort 拒绝。
- Host Gateway 覆盖同 caller 同 ID dedupe/conflict，以及不同 caller 使用相同 ID 相互隔离。
- `npm run protocol:check` 验证 nullable `userItem`、`nonIdempotent` manifest 和生成文件无漂移；Host/Provider 定向测试验证边界行为。
