# Gateway v1

Gateway 定义 UI、手机或其他操作端与 Host 的交互。它与 Provider 协议分别编解码，通过 common 和 agent-domain 复用对象结构。

protocol.json 登记功能模块入口。session 负责握手、描述和应用心跳；subscription 负责事件恢复；provider 提供客户端可见的 Provider 信息；project、conversation、input-queue、turn、approval、usage 各自维护交互定义。每个模块只在自己的 methods.json、events.json 登记方法与事件，messages.schema.json 描述载荷；跨模块直接引用，不再手写总方法表。

pairing 属于 Gateway，但在授权前独立交换，不要求通过已认证业务 RPC。其 messages.schema.json 维护配对与凭据对象，bindings.json 记录已有 HTTP 绑定。当前保持历史字段、端点和公开 SDK 类型名，不把 LAN/TLS 接入字段误认为所有通道的固有要求，也不声称已统一所有云邀请/信令 DTO。通道选择、发现和连接安全由适配器实现。

远程连接完成相应授权后，客户端首先调用 protocol.handshake，Host 校验 clientId 与凭据绑定、选择版本并返回 Host、Provider 摘要及事件边界。客户端读取业务快照，原样把 snapshotCursor 交给 event.subscribe，随后合并重放与实时事件。protocol.ping 的递增序号用于本连接保活，响应回显序号和 Provider 摘要；底层 WebSocket Ping/Pong 另行处理。心跳周期、失联和请求超时策略在 session Schema 的生命周期注解定义，并生成到 SDK。

握手每连接只成功一次，事件订阅每连接只成功一次；重连建立新会话并恢复不透明游标，无法重放时重新取业务快照。断开后的写操作不能自动重放，应保留结果不确定语义。本地 IPC 通过 Host 本地入口调用业务，不要求执行远程配对或 socket 握手。

Schema 说明字段与约束，服务元数据说明 RPC 扩展，注册表说明幂等性和投递语义。生成器生成客户端、服务端接口、dispatcher、类型、事件元数据和校验器。Gateway 是内部操作端协议，不进入对外 Provider SDK。
