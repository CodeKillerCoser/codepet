# Provider stdio 由 SDK Runtime 统一使用二进制帧

## 背景

Provider 通过 stdin/stdout 与 Host 交换 JSON-RPC。旧实现以物理换行分隔 UTF-8 JSON，并在 `conversation.get` 返回前由共享 SDK 反复序列化响应、按 7 MiB/5 MiB 内容预算静默裁剪，再由 JSON-line writer 执行 16 MiB 硬限制。该设计把领域内容策略、分页策略和传输保护混在一起：调用方无法通过 `provider_response_too_large` 缩小同一 cursor 的 `limit`，大型结构化内容还会因尺寸探测产生重复遍历和临时字节缓冲。

Provider 是可扩展边界。第三方实现者应只实现生成的强类型 `Provider` trait，并调用 SDK 的 `serve_stdio`；不应自行读取 stdin、写 stdout、选择压缩算法、维护帧边界或封装超限错误。

## 决策

Provider 业务协议继续使用 v1 JSON-RPC 2.0，序列化格式固定为 UTF-8 JSON。Provider Rust SDK Runtime 在业务接口之外统一实现 `CodePet Provider Frame V1`：固定头包含四字节 magic `CPRF`、一字节 frame version、一字节 encoding 和四字节大端 payload length，后接准确长度的 payload。V1 只支持 `raw` 与 `zstd` 两种 encoding；不携带 format 字段，未来引入 MessagePack、Protobuf 等格式时再升级 frame version。

Runtime 先把 wire message 序列化一次为 JSON bytes。小于 32 KiB 时使用 raw；达到 32 KiB 时尝试 zstd level 1，仅当至少节省 10% 且至少节省 1 KiB 时使用压缩，否则仍发送 raw。用于长度判断的 payload 就是最终写入通道的同一份 payload，不允许为了测量总尺寸再次序列化业务响应。

16 MiB 只约束最终在 stdio 通道上传输的完整 frame，即固定 header 与编码后 payload 的总字节数。不定义解压后尺寸上限，也不以压缩前 JSON 大小决定是否接受。Host 按 header 的 payload length 有界读取，raw 直接解析 JSON，zstd 通过解压 reader 解析 JSON。格式错误、未知 frame version、未知 encoding、截断 payload 和非法 JSON 必须 fail closed。

任意 Host→Provider 请求在编码后的 frame 超限时不得写入 stdin；任意 Provider→Host RPC 响应超限时，Runtime 丢弃原 frame，并以相同 request id 写一个小型 raw JSON `provider_response_too_large` 错误。Gateway 不探测尺寸、不修改业务对象、不自动缩页。最终调用方收到该错误后，保持 cursor 不变，按固定梯度 `40 → 20 → 10 → 5 → 1` 重试 `conversation.get`；成功后才推进 `nextCursor`，`limit=1` 仍超限则把错误交给上层。

`conversation.get` 不再拥有传输专用的内容预算、全响应预序列化或静默降级。Provider 必须真实遵守 cursor/limit，只生成当前页。工具输出在原生数据转换为领域对象时可以按明确的内容策略截断并携带 `originalBytes`、`retainedBytes` 和 `strategy`；一旦领域响应形成，Runtime 只能成功传输或返回超限错误，不能再删除 preview、summary、tool input/output 或其他业务内容。

## 备选方案

- 继续 JSON Lines 并把压缩字节放进 Base64 payload：外层仍是可读 JSON，但 Base64 增加约三分之一传输体积，并引入第二层 JSON envelope；既然 stdin/stdout 两端都由 SDK Runtime 控制，不采用。
- 使用 WebSocket 的 permessage-deflate：该能力由 WebSocket 实现管理，不能直接复用于独立进程的 stdio frame；Provider Runtime 采用单帧 zstd level 1。
- 同时支持多种压缩和序列化格式：扩展性更强，但会扩大当前协议、生成器和跨语言验证面；V1 只固定 JSON 与 raw/zstd。
- 同时限制压缩前和压缩后尺寸：能提供更严格的内存边界，但会把业务对象大小重新混入本次“保护通道”的限制；本次只限制最终 wire frame。
- Provider 自己实现 framing：可以针对原生 harness 定制，但会复制 stdin reader、writer、错误映射和压缩策略，第三方 Provider 也更难正确接入；不采用。

## 取舍理由

长度前缀允许 payload 包含任意二进制，不需要扫描换行或执行 Base64。固定 JSON 保留既有协议语义和生成 DTO；encoding 独立于业务对象，Runtime 可逐帧选择 raw/zstd。只检查最终 frame 使限制与目标一致：防止单次 stdio 写入占用过大的通道容量，而不是限制领域对象或让内容可压缩性之外的预估决定行为。

压缩前 JSON 仍需在发送端内存中形成，因为 Runtime 必须在写出任何字节前保留返回小型超限错误的能力。该成本由一次真实序列化承担，不再增加只用于尺寸探测的完整副本。解码端不设协议级 decoded-size cap 是有意取舍；实现仍必须拒绝损坏帧和解压错误，并持续观察异常压缩比造成的内存风险。

## 影响范围

- `protocol/provider/v1/manifest.json`：transport framing 改为 Provider Frame V1，业务版本仍为 v1。
- `sdk/rust/codepet-provider-sdk`：Runtime codec、stdio reader/writer、错误封装和 transport tests；公开 Provider trait 不增加 framing 参数。
- `tools/protocol-codegen`：生成/分发的 Provider Runtime 模板必须与 checked-in SDK 一致，避免重新导出时退回 JSON Lines。
- `crates/codepet-host/src/process.rs`：异步 stdin writer/stdout reader改为相同 Frame V1 codec，不复制压缩策略。
- `crates/providers/*`：继续只调用 `serve_stdio`；`conversation.get` 负责真实 cursor/limit 分页，不做 transport-size 探测。
- `codepet-remote`：只消费稳定的 Gateway 错误并负责缩页，不感知 Provider stdio framing 或 encoding。

## 后续观察

- 记录 raw/zstd 选择、JSON bytes、wire bytes、压缩耗时与 `provider_response_too_large` 次数，但诊断不得记录正文。
- 使用真实大历史复测 CPU、峰值内存、stdio wire bytes 和 Remote 缩页次数；WebSocket 压缩与 Provider stdio 压缩必须分别计量。
- 若出现可信但高压缩比的巨大 JSON 导致 Host 内存压力，再独立评估流式 JSON 解析、内容引用或 decoded-size guard，不能悄悄改变本决策的 16 MiB 语义。
- 若未来增加非 JSON format，先升级 frame version并补齐所有 SDK Runtime 与独立分发测试，不能在 V1 encoding 中偷渡格式变化。
