# Provider stdio 消息复用与启动握手

## 背景

2026-09-06 用户确认需要完整的稳定性保障，并要求 Host↔Provider 启动握手携带协商信息。实现已接入 SDK、Host 与三个内置 Provider 的 manifest，业务 JSON-RPC 版本仍为 1。

依据：[单帧基线](../40-runbooks/provider-stdio-contention.md)、[并发与内存基线](../40-runbooks/provider-stdio-burst-contention.md)、[跨进程隔离基线](../40-runbooks/provider-stdio-process-isolation.md)。单向大帧会使其他 RPC 等待到整帧结束。新旧 profile 使用同一节流器的[对照测试](../40-runbooks/provider-stdio-mux-validation.md)已验证分片可以改善等待。

## 目标

启动阶段确认版本、必需功能及双方接收限制；传输层负责分片、顺序、流控、预算、取消与关闭。业务实现继续使用生成的 typed Provider trait 和事件接口。

## 非目标

不增加业务流式 API，不提供消息重传或 exactly-once，不保证所有业务对象的总 RSS，也不保证操作系统暂停或一方完全停止读取时仍有进展。传输取消不等于撤销已产生的业务副作用。

## 现状理解

新 profile 为 `stdio-codepet-mux-v1`，采用固定版本的 Rust Yamux 0.14。其 stream ID、SYN/FIN/RST、分片调度及窗口由库管理。连接 driver 独立持续轮询，不在读取整个大 JSON 或执行业务时停止推动其他 stream。

`stdio-codepet-mux-v1` 现为默认且唯一 STDIO profile；旧 `runtime/legacy` 与 Host 整帧收发分支已删除。CPRF V1 header 保留为 stream 内正文格式，单独的 `ProviderFrameCodec` 是消息编解码工具，不代表支持旧 STDIO 服务。

## 实现路径

### 显式选择和启动顺序

插件 manifest 新增可选 `transport`。Catalog 校验并转成 Host 注入的 `CODEPET_PROVIDER_TRANSPORT`，SDK 校验该值后运行 mux。未知 profile、manifest 与 env 冲突均拒绝。Host 总是注入所选值，避免继承外部环境导致两端模式不同。

| 启动配置 | 行为 |
| --- | --- |
| 新 Host + `transport: stdio-codepet-mux-v1` | 先传输握手，再业务 initialize |
| 新 Host + 未声明 transport | 默认 mux，先握手再业务 initialize |
| 显式旧 profile、未知 profile 或冲突 env | 拒绝启动，无回退 |
| 旧 Host + 新 manifest | 旧严格 manifest 解析拒绝新字段；不承诺兼容 |
| 新 SDK 单独启动、无环境变量 | 默认 mux，等待 Host 传输握手 |

新 bootstrap 是 `CPMX[4] + version:u8(1) + phase:u8 + jsonLength:u16BE + JSON`，每个 JSON 至多 4096 字节，不压缩；整个阶段限时 3 秒。

1. HELLO（phase 1）：Host 发送 `ProviderTransportHello`。
2. SELECT（2）：Provider 发送 `ProviderTransportSelection`。
3. CONFIRM（3）：Host 验证后发送 JSON 数字 `1`。
4. READY（4）：Provider 写出 JSON 数字 `1` 后进入 Yamux；Host 完整读完 READY 后进入 Yamux。

Hello/Selection 为 canonical schema 生成 DTO，不扩展业务 initialize。v1 必须具备完整 features 集合：raw-json、zstd-json、yamux-1、message-grant、reserved-lanes、bounded-decode、reset、ordered-events。无公共版本、缺必要 feature、未知 JSON 字段、非法容量或错序均失败；无同一连接上的猜测回退。

### 方向性限制

Host 的 receive 约束 Provider→Host，Provider 的 receive 约束 Host→Provider。实际发送分片取对端接收上限与本端分片策略的较小值；容量不混成一个全局 min。

| 字段 | 默认值/计量 |
| --- | --- |
| maxFramePayloadBytes | 16 KiB，单个 Yamux DATA payload；读取 header 后、分配 body 前校验 |
| maxEncodedMessageBytes | 16 MiB，完整 CPRF header + encoded payload |
| maxDecodedMessageBytes | 128 MiB，完整 JSON bytes；解压时有硬边界并核对声明长度 |
| receiveBudgetBytes | 256 MiB，普通消息 encoded + decoded 预留额度 |
| normalStreams / smallStreams / controlStreams | 24 / 16 / 4；Provider 会按 StdioServerOptions 总容量进一步收紧 |
| smallMessageBytes / smallReceiveBudgetBytes | 64 KiB / 1 MiB |
| controlMessageBytes / controlReceiveBudgetBytes | 16 KiB / 256 KiB |
| connectionWindowBytes | 32 MiB，Yamux 接收窗口总上限；包含初始 stream 窗口所需空间 |
| idleTimeoutMs / streamTimeoutMs | 10 秒 / 60 秒；Host RPC deadline 仍独立生效 |

小/控制消息的编码与解码长度都受对应 messageBytes 限制；控制请求只能使用生成 manifest 中的 control dispatch lane，普通请求不能占用其额度。发送编码工作分 lane 限并发，并在编码前取得字节预算；取消中的 blocking worker 继续持有自己的额度直到结束。等待响应首字节受 RPC/stream deadline 管理，正文开始后的 read/write/flush 才用进度 idle deadline。物理 Yamux 帧若只收到部分 header/body 后停止推进，也会按 idle deadline 关闭连接；没有未完成物理帧的空闲连接不受此计时器影响。

### 消息生命周期

一条双向 Yamux stream 承载一组请求/响应，Host 奇数、Provider 偶数 ID 在连接中单调分配。事件使用 Provider 发起的独立 stream。

每条消息 envelope 为 `version:u8(1) + class:u8(0 normal/1 small/2 control) + encodedLength:u32BE + decodedLength:u32BE`。接收方校验并取得 stream slot 与整消息 encoded + decoded 预算后写单字节 GRANT `1`，发送方才写 CPRF 正文。预算不可能满足的声明立即失败；暂时不足只等待有界时间。无二进制尾终止符，边界依赖长度与 FIN。

请求发送后保留写方向，供响应接收方返回 GRANT。响应完毕关闭 stream；超时、取消、非法消息或截断丢弃该 stream，Yamux 发送 RST。请求 dispatch 期间监听 reset，取消等待中的业务 future；已发生副作用不能回滚。接收预算持有到 decoded message 在受控 dispatch 中消费完成，不能在进入排队任务前就归还。

事件由有界队列串行编码与发送；Host 先把完整事件交给 inbound 消费者，再返回单字节 ACK `2`，下一事件才发送，维持事件顺序。等待消费者期间继续持有接收预算；不能在入队时提前归还额度，累积大量不计费的大对象。`publish` 的成功表示队列接受，异步编码拒绝记录 stderr 后继续；队列满同步返回错误，底层 I/O 失败触发清理。默认最多 256 个排队事件，并按其序列化大小收取 256 MiB 预算。

Provider 在连接结束或 shutdown 后关闭事件入口、停止 heartbeat；连接丢失立即取消待执行 dispatch，正常 shutdown 有限时间 drain，随后执行清理。输入泵显式 shutdown duplex 写半端传播 EOF，输出泵完成/失败由 runtime 主循环监听；接收队列关闭后仍收取 driver 结果，防止异常退出被报告为成功。Yamux 在 GOAWAY/EOF 后仍允许读取 stream 中已缓存的数据，避免丢弃最后的 shutdown 响应；runtime 返回前等待真实 stdout pump 排空并报告结果，不能把内存 duplex 的 flush 当作物理 flush。driver 的 RAII guard 在所有权任务退出时 abort，避免 Host 已销毁但 driver 仍持有 stdio。任意 blocking Read 无法通用中断，SDK 的独立 stdin pump 在管道 EOF 或进程退出后结束，不占用 Tokio blocking pool。

### 编码和边界

新 profile 小于 32 KiB 使用 raw，达到阈值使用有界 zstd level 1。超限 RPC 响应在尚未写 envelope 时改发相同 id 的 `provider_response_too_large`；不裁剪业务字段。JSON 序列化和解压均有上限。业务对象、serde 对象开销、调用方在进入传输层前持有的参数以及 allocator overhead 不等于 JSON bytes，不能把接收预算宣传成整个进程 RSS 上限。事件发布前的尺寸计数和小消息分类仍需遍历数据，分片不消除全部 CPU 成本。

## 涉及模块

- `protocol/provider/v1` 与 codegen：定义新 profile 和独立握手 DTO，确保生成 Rust/TypeScript 合同一致。
- Provider SDK `transport/`：bootstrap、物理帧保护、Yamux driver 与流/额度生命周期，通过 `MessageCodec` 接口调用上层策略。
- Provider SDK `message/`：Provider JSON 编解码、控制方法分类和超限响应；`runtime/` 负责 mux STDIO 接入、业务派发、事件和 heartbeat。具体依赖规则见 [SDK 分层决策](../50-decisions/provider-sdk-layered-runtime.md)。
- Host `providers/process.rs`：握手就绪门禁、每 RPC stream、事件交付、进程清理；`catalog.rs` 校验并填入默认 profile。
- 三个内置 Provider manifest：显式开启新 profile，业务实现无需写 framing。
- `cp-sdk-gen`：通过 `provider-runtime.mjs` 静态嵌入并导出嵌套源码与依赖，完整性测试覆盖清单；SDK 作者文档解释二进制协议、队列语义与兼容矩阵。

## 风险与测试计划

- 大流阻塞其他流：停读一个 stream，验证另一小 RPC 完成；节流真实子进程比较请求/响应两个方向。
- 预算/解压越界：超大声明、压缩伪造 decoded 长度、DATA header 越界、部分正文失活测试；检查额度归还。
- 控制饥饿：耗尽 normal/small 预算时 control 请求与响应仍完成。
- 状态错误：不兼容握手、方向性容量、编码失败小错误、取消后继续请求及正常 shutdown 测试。
- 导出/迁移遗漏：protocol check、SDK 全测试、Host 回归、内置 Provider all-target 检查和导出 SDK 独立编译。

## 知识沉淀

旧 Frame V1 决策继续记录历史边界；新约束见 [stdio 复用规约](../60-rules/provider-stdio-multiplexing.md)，测试方法与证据见 [验证 runbook](../40-runbooks/provider-stdio-mux-validation.md)。改变默认容量、feature 集合或事件交付语义时同步更新合同与测试。

## 未知项

当前性能结果来自 macOS arm64 的受控夹具，不是生产 p99/SLO。Windows/Linux 原生 pipe 行为、真实会话长期运行 RSS、全部业务 CPU 干扰及多 Provider 生产负载仍需要专项测量。新 profile 已验证公平进展，不承诺任意低带宽下毫秒级响应。
