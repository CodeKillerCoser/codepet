# Gateway v2

Gateway 定义 UI、手机或其他操作端与 Host 的交互。它与 Provider 协议分别编解码，通过 common 和 agent-domain 复用对象结构。

protocol.json 登记功能模块入口。session 负责握手、描述和应用心跳；subscription 负责事件恢复；provider 提供客户端可见的 Provider 信息；project、conversation、input-queue、turn、approval、usage 各自维护交互定义。每个模块只在自己的 methods.json、events.json 登记方法与事件，messages.schema.json 描述载荷；跨模块直接引用，不再手写总方法表。

pairing 属于 Gateway，但在授权前独立交换，不要求通过已认证业务 RPC。其 messages.schema.json 维护配对与凭据对象，bindings.json 记录已有 HTTP 绑定。当前保持历史字段、端点和公开 SDK 类型名，不把 LAN/TLS 接入字段误认为所有通道的固有要求，也不声称已统一所有云邀请/信令 DTO。通道选择、发现和连接安全由适配器实现。

远程连接完成相应授权后，客户端首先调用 protocol.handshake，Host 校验 clientId 与凭据绑定、选择版本并返回 Host、Provider 摘要及事件边界。客户端读取业务快照，原样把 snapshotCursor 交给 event.subscribe，随后合并重放与实时事件。protocol.ping 的递增序号用于本连接保活，响应回显序号和 Provider 摘要；底层 WebSocket Ping/Pong 另行处理。心跳周期、失联和请求超时策略在 session Schema 的生命周期注解定义，并生成到 SDK。

握手每连接只成功一次，事件订阅每连接只成功一次；重连建立新会话并恢复不透明游标，无法重放时重新取业务快照。断开后的写操作不能自动重放，应保留结果不确定语义。本地 IPC 通过 Host 本地入口调用业务，不要求执行远程配对或 socket 握手。

Schema 说明字段与约束，服务元数据说明 RPC 扩展，注册表说明幂等性和投递语义。生成器生成客户端、服务端接口、dispatcher、类型、事件元数据和校验器。Gateway 是内部操作端协议，不进入对外 Provider SDK。


## 版本与前向兼容

握手请求在 protocol 中发送本端 version/minVersion；响应发送 Host 区间及双方最高共同 selectedVersion。客户端必须复核选择结果。当前范围 3/2，兼容既有 2/2 操作端，断线重连重新协商，心跳不更改已选版本。SDK 提供统一范围校验辅助函数，业务版本和 WSS 路径、RTC 帧、配对格式各自独立。

未知字段忽略，未知枚举/联合保留原值，已知非法数据拒绝。未知事件验证 JSON-RPC 信封和 cursor 后跳过并推进游标；未知状态保持不可操作，未知能力不调用。请求编码不得复用收到的未知选项。没有共同版本时不进入订阅/业务阶段，也不自动重放写入。完整规则见上层 v2 README。

## Provider 身份与清单同步

ProviderSummary.id 和业务路由中的 providerId 都是实例身份，等同 Provider 的 instanceId；新增可选 pluginId 标识所属插件包，多个实例可共用一个 pluginId。features 复制插件明确声明的功能，不由 Host 或 UI 从方法名前缀猜测；缺失表示旧端未提供。这些信息不改变实例 methods 的执行授权边界。桌面管理桥接的历史 providerId 字段仍是插件身份的兼容别名，新连接快照明确提供 pluginId、instanceId；不要将这个局部兼容名用于 Gateway 路由。

handshake、provider.list 和 protocol.ping 返回当前全量实例清单及 inventoryEpoch / inventoryRevision。两字段同时提供或同时缺失。完整快照替换集合，不能只合并新增。Host 在一个 Gateway 生命期内按摘要变化或成员移除单调推进 revision；每次 provider.changed 或 provider.removed 使用各自提交后的 revision，与快照使用同一序列。进程 generation 仅表示 Provider 重启代次，不能替代清单版本。停止、禁用但仍在清单中的实例通过 changed 更新状态；卸载及不再可见的实例通过 removed 剔除。一个插件卸载会移除其全部可见实例，纯通知插件不伪造可操作 Agent 实例。

客户端忽略不高于已应用 revision 的事件、低于已应用 revision 的全量响应；游标仍推进。相同 revision 的完整快照可重新校正集合。移除时清理实例和能力缓存、停止对应观察，迟到的 describe 响应不能写回。心跳期间收到更晚事件，旧心跳不得恢复已删除条目。新安装复用旧实例 ID 时允许新的清单 revision，不能用旧 generation 阻挡新实例。来自不同非空 epoch 的事件不能用于局部更新，应重新握手并建立完整快照；不能只把 revision 归零。

inventoryEpoch 标识当前 Gateway 生命期，重启后改变；revision 仅在相同 epoch 中比较。重连以 handshake 全量快照重建 epoch/revision 基线，再订阅其 eventCursor 后的事件；缓存游标超出重放范围时重新取快照。revision 缺失表示旧 Host，保留旧 changed 合并与心跳全量校正路径；旧操作端忽略未知 removed 事件，下一次心跳完成移除。不要在已经收到修订号的新连接上接受无修订号的迟到更新。通道只负责有序交付同一事件流，不自行分配清单版本。自动切换通道时，Gateway 会话适配器检查新握手的 epoch；同 epoch 按现有游标续传，不同 epoch 必须重建业务快照，即使旧数字游标在新 Host 上恰好有效，也不能复用，更不能重放写请求。
