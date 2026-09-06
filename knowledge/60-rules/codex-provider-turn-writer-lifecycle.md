# Codex 交互权跟随连接与共享 Server 生命周期

## 规则

支持 Server 模式的 Harness，每个 Provider 运行实例只有一个 Server。Codex list/get/create/resume/turn/approval 共用该 Server；会话槽只负责串行、配置与路由，不创建独立进程。删除会话租约、续租 timer 和过期 reaper。

SDK 根据 Host 心跳携带的客户端连接集合协调实例：仍有任意客户端时保留所有已 resume 会话；最后连接消失或 Host 心跳超时后停止 Server，包括 active turn。页面退出不会改变连接集合。Provider 插件进程与 Harness 状态分开，插件保持在线等待重连。

## 适用场景

Codex lifecycle、conversation acquire/create、turn、approval、事件分发、Remote 详情与 SDK 心跳改动。其他 Server 模式 adapter 遵循相同实例与 Server 一对一约束。

## 反例

- 十个会话启动十个 App Server，或保留另一个只读 observer。
- Remote 每 10 秒续租、页面退出释放 writer，导致仍在线时丢失后续变化。
- A 的终态或明确业务 RPC reject 关闭共享进程，破坏 B 的事件与审批。
- 只按 clientId 删除在线记录，使同一客户端重连时旧 socket 关闭误删新 socket。
- 把 thread/unsubscribe 成功当作立即交还 writer；实测 0.153.1 仍持有已加载 thread。
- resume 默认返回全部 turns，绕开 caller 选择的分页并重复读取大历史。

## 推荐做法

- instance.start 是唯一 Server 启动入口；每会话 Creating/Ready/Failed/Closed 槽合并首次 acquire，同一个 conversation 的操作持 operation lock 并复核 generation。
- 原生 thread/resume 固定 excludeTurns=true。Gateway conversation.resume 顺序 acquire/get，首屏结构复用 ConversationGetResponse。Remote 首屏只取一页，用户手动加载更早消息，每页初值 20；只对尺寸错误按 20→10→5→1 缩页，不重做已成功的 acquire。
- create 使用共享 Server 的 thread/start，响应直接建立槽，首条 turn/start 复用。未 materialized 的精确官方响应只对有 create 证据的当前 generation 映射空历史，不能吞掉近似错误。
- 一个 Server reader 按 conversation ID 分发事件。start/resume 安装槽前到达的通知暂存，不能丢弃其他会话事件；同会话终态在 operation lock 内完成映射与发布后，才允许下一操作。
- turn 完成只清理 active turn，不释放 writer。响应超限、单条协议错误、RPC reject/timeout 只影响对应请求；坏事件或日志不得关闭共享进程。原生 stdout EOF/I/O 断开、Server crash 才使整个实例不可用并清理全部映射。未知发送结果不得自动重发 turn/start。
- approval 句柄包含 Server generation 和原始 request ID，回写还要校验 conversation 与 pending 记录；不能让旧审批命中新进程复用的 request ID。
- 只把 code=-32600、空 data、message 精确匹配当前 thread 的官方 active-writer 错误映射为 conversation_write_conflict；近似 message 不猜测。
- Remote 成功获取交互权后开放 composer，成功后不定时 acquire；失败重试属于恢复流程。重连、Provider generation/可用状态改变后重新检查。首次 selection 可初始化 UI，后续重试不得覆盖用户修改。
- SDK 应用 ping 直接处理，生命周期协调与普通 RPC 分离。普通请求保持 16 active + 32 pending，生命周期 control 保留 2 active + 4 pending；队列满按原 id 返回 provider_overloaded。EOF/fatal 先停止心跳协调再关闭 Provider，不能在清理时再次启动 Server。
- `leaseExpiresAt` 暂留为可选 wire 兼容字段；Codex 返回 None，客户端不使用它调度或决定 Server 存活。

## 来源

- `../10-architecture/remote-and-provider-connections.md`：2026-09-05 确认的连接级生命周期。
- `../10-architecture/codex-provider-plugin-runtime.md`：共享进程与协议映射。
- `../40-runbooks/codex-detail-size-and-capability-loading.md`：resume 超限和原生 unsubscribe 证据。

## 验证方式

[单条错误隔离回归](../40-runbooks/codex-request-errors-must-not-stop-server.md)覆盖错误响应后的同 PID、并发 B 请求/事件、晚到响应和事件编码失败；[文本与分页规约](provider-item-text-and-pagination.md)覆盖超大原生响应、tool 截断与非 tool 保真；生命周期测试继续验证真实 EOF/Broken pipe 的清理。

Codex vertical 覆盖 64 会话同 PID、并发首次 acquire、两端连接与最后断开、重连仅新建一个进程、A 终态保留 B、审批隔离、terminal publication barrier、resume/cancel barrier、慢输出、共享 Server crash、明确 reject 和未知结果。SDK 测试拒绝旧 sequence/revision；Host LAN 和 Provider saturation 测试证明慢 RPC 不阻塞应用 ping。Remote 测试覆盖成功后不续租、能力恢复、Provider 隔离、旧 generation 丢弃与项目补拉。
