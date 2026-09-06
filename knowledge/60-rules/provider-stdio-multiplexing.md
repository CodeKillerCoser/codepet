# Provider stdio 复用边界

- STDIO 默认且仅支持 mux；Host、SDK、manifest 与测试客户端必须一致。业务 initialize 前完成独立 bootstrap，显式旧 profile 或未知 profile 拒绝，不回退。
- 声明容量必须验证方向、取值和一致性。分片长度在 body 分配前校验；decoded 长度在解压过程中限制，再核对实际值。
- normal、small、control 的发送/接收额度独立；control 方法来自生成 manifest，不维护另一份手写方法表。
- SDK 依赖方向为 runtime → message → transport。`MessageCodec` 由传输层定义、Provider 消息层实现；传输生产代码只能依赖传输 DTO，不引用 Provider 业务消息、方法表或 runtime。额度取得与持有仍属于传输层，超限 RPC 响应属于消息层。依据：[SDK 分层决策](../50-decisions/provider-sdk-layered-runtime.md)。
- 接收整消息预算在 GRANT 前取得，生命周期结束才释放。取消后台编码/解码 waiter 时，worker 仍持有额度直到其自身结束。
- stream reset 终止传输等待，不能用作业务回滚承诺。重试非幂等请求前须按业务合同处理未知结果。
- 无事件的 Provider 也是合法实现。Runtime 自己持有发布器，不得因为 factory 忽略 sink 就关闭连接。回归：`mux_handshake_cancel_and_shutdown_use_the_production_sdk_runtime`。
- Host 向 inbound 消费者交付事件前必须持有其接收预算；事件 ACK 在消费者取走消息后发送，不能把大对象转到仅按条数限流的队列后提前归还字节额度。
- 有顺序要求的事件必须保序交付；队列接受不等于 Host 已处理。编码拒绝不应关闭健康连接，实际 I/O 故障应走统一清理。
- GOAWAY/EOF 后应允许消费 stream 已缓存的最后响应；runtime 成功退出前必须等待真实 stdout pump 完成，不能把 duplex flush 当作物理写入完成。回归：`shutdown_waits_for_the_blocking_output_pump_to_flush_and_finish`。
- 物理输入 EOF 必须显式传播到 duplex 写半端；物理输出泵失败必须即时通知 runtime，不能等待另一端 EOF。incoming 队列关闭不能替代 driver 错误结果。回归：`input_eof_and_output_failure_trigger_cleanup_without_waiting_for_other_pipe` 及三个 Provider 断管/坏帧进程测试。
- driver 所有权任务结束必须释放底层 I/O；任意 std Read 的阻塞泵不能占用 Tokio blocking pool，退出语义要覆盖 EOF 与进程终止。
- JSON byte 预算不是完整 RSS 限制。压测必须同时观察小 RPC、心跳、吞吐、事件、退出及异常恢复。

验证入口：[实现和测试矩阵](../10-architecture/provider-stdio-message-multiplexing.md)、[性能对照](../40-runbooks/provider-stdio-mux-validation.md)。修改生命周期或容量逻辑后，执行相关 mux 测试和 Host 子进程测试；更改 SDK 分发内容时额外执行导出包独立编译。
