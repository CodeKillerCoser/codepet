# 协议层依赖与运行通道边界

## 规则

Gateway v1 业务协议、channel 和 admission 必须保持三层独立：channel 只传输 JSON-RPC object/bytes，admission 只建立设备级信任，Gateway 只处理业务方法。证书指纹、pairing secret、bearer 和 mDNS TXT 不得进入 Gateway schema；Gateway method、Provider capability、conversation/turn DTO 不得进入 channel 或 LAN admission schema。准入成功默认可访问 Host 全部 Provider，不做 Provider ACL，但每个 Provider 范围的请求、响应与事件必须保留 `deviceId + providerPluginId + providerInstanceId`。LAN pairing QR 的 `version` 必须读取 LAN types SDK 从 manifest 生成的 `CHANNEL_LAN_SCHEMA_VERSION`，不得复用 Gateway `PROTOCOL_VERSION`；二者即使曾经数值相同也不是同一个版本空间。types-only SDK 的 schema version 常量必须带 package 前缀，避免 Dart SDK re-export 依赖包时产生命名冲突。

长连接的 transport control plane 不得等待业务 RPC：WebSocket reader 必须持续 poll Ping/Pong/Close 和 credential cancellation，握手后的 Gateway 请求进入每连接有界队列与有界并发 dispatcher，再按 JSON-RPC id 独立返回。响应允许乱序，不能为了保持请求顺序让 `conversation.get` 阻塞 socket reader。writer 必须有界，但单次发送 deadline 要覆盖真实移动网络上的合法大响应；过载由请求队列、并发数和 outbound queue 共同 fail closed，不能用亚秒级 send timeout 把正常背压误判成断联。

协议事实必须从 `protocol/` 单向生成到 `sdk/`：schema 与 manifest 的全部 `$ref` 使用同一依赖审计，core 不依赖任何上层，pet/provider/gateway 只能引用显式声明的安全下层。每种输出语言必须有独立 target adapter，未实现 target 必须 fail closed。Provider plugin event 不得发布到 companion Tauri event、replay、Pet projection 或 activity store；Pet action 不得调用插件生命周期接口。Gateway 快照响应的 `snapshotCursor` 必须在发起 Provider 查询前捕获；`EventCursor` 对客户端始终 opaque，只能原样保存和回传，不能解析、排序或执行 `+1`。完整历史要分别守住 App Server→Provider 与 Provider→Host 两段有界 JSON-line：上游有 cursor API 时必须分页读取并逐页释放原生 DTO；最终投影仍超过 Provider/Host 共享上限时，Provider 必须按原请求返回稳定的小错误并继续服务，不能只放宽某一端或让写响应失败终止进程。Host 发出 `provider.shutdown` 后必须继续读取 stdout，丢弃不再有消费者的晚到 event/notification，但必须保留 shutdown response 与进程退出监管。

Conversation 工具载荷还必须遵守 [`conversation-tool-payload-ownership.md`](conversation-tool-payload-ownership.md)：互斥关系由 v1 schema 的判别联合强制表达，工具输入与结果各只有一个完整载荷所有者，截断发生在 Provider stdout 序列化之前。Host 投影器或 Remote 的文本去重只能作为迁移诊断，不能成为协议正确性机制；抽屉等 UI 展示位置不进入协议。

## 适用场景

- 修改任一 schema、method/event manifest、generator 或 SDK 包。
- 接入 Provider binary、Provider Host、Gateway remote transport 或 Pet Protocol adapter。
- 修改 Runtime Gateway registry/event bus/Tauri bridge 或桌宠 projection。
- 修改 Gateway snapshot、replay、live event 或 `turn.outputDelta` 的同步流程。
- 修改 `conversation.list/get` 的完整历史映射、Provider JSON-line codec 或 Host process frame 配置。
- 修改 Provider Host 的 inbound consumer、stdout reader、shutdown 或进程退出顺序。

## 反例

- 在 Rust 业务模块手写一套与 IDL 相同的 DTO，再让 generator 追随 Rust。
- 在 core 放入 Conversation、Turn、Approval、PetTask 或插件进程状态。
- 只带 native conversation id 穿过 Remote Client 边界。
- 让 Provider SDK event enum复用 companion event sink，之后依赖前端过滤 remote task。
- 为方便启动插件，把 `instance.start` 或 `provider.shutdown` 暴露到 gateway manifest。
- 让 Python target 落入其他 generator，或因为没有 output 而返回成功；让 Dart 为 DTO、metadata 和 client 另写 method/event 表。
- 在 Provider Host 另写无界 `read_line`/宽松 JSON-RPC parser，接受同时包含 result 与 error 的 response。
- 在各 Provider `main.rs` 复制 stdin reader、stdout writer、dispatcher queue、overload 或 EOF/fatal cleanup，并再次手写哪些 method 属于 control lane。
- Provider 查询完成后才读取 `snapshotCursor`，导致查询期间的事件可能落在客户端订阅边界之前。
- 客户端把 `EventCursor` 当数字递增，或让 `conversation.get` 快照携带仍由 `turn.outputDelta` 修改的进行中正文。
- Provider 仍用 1 MiB 默认 codec，而 Host 单独放宽到 16 MiB；超过 1 MiB 的合法历史会在 Provider 写响应时直接终止进程。
- observer 用 `thread/read(includeTurns=true)` 把全部历史聚成一条 App Server JSONL，导致尚未反序列化就触发 physical-line 上限。
- 为绕过大历史同时抬高所有 frame limit，或让最终 `conversation.get` 编码超限沿 fatal 路径关闭 Provider。
- 在 WebSocket `source.next()` 循环里直接 `await conversation.get`，使慢 Provider 查询期间无法读取 Ping 并导致移动端按心跳超时关闭；或给所有响应统一设置 1 秒发送超时，让真机链路上的大响应被误判为断连。
- shutdown 一开始就因关闭 inbound consumer 而退出 stdout reader；Provider 清理期间发布状态事件后会遇到 Broken pipe，shutdown response 无法送达。

## 推荐做法

- 先修改 `protocol/<layer>/vN`，运行 `npm run protocol:generate`，业务代码只 import SDK。
- 新语言实现 `codepet.protocol.codegen/v1` adapter；在 package outputs 与负向测试就绪前保持 planned。
- 已实现的 Dart target 必须消费 normalized IR，并对无共同 required singleton-enum discriminator 的 `oneOf` fail closed；不得退化为 `dynamic` 或多个互不约束的 optional 字段。
- 共享资源使用 `RoutedResourceId`；实例级请求显式携带 deviceId 与 providerInstanceId。
- Provider 使用生成 `JsonLineCodec`、wire classifier、`ProtocolRequest::from_method_params`、typed capability mapping 和 instance-kind validation helper；Host 不枚举 request envelope variant。
- Provider binary 通过 SDK `serve_stdio` 和 typed `ProviderEventSink` 接入；控制通路分类来自 manifest `dispatchLane` 生成 metadata。
- App bundle 必须把完整接入 README、canonical `protocol/{core,provider,gateway,channel}` 资源、fixtures、`codepet-sdk.json` 和平台原生 `cp-sdk-gen` 作为同一版本资源分发；分发 schema 的相对 `$ref` 必须在 Resources 内可解析。bundle 前先做 generated freshness 检查，并从最终 App Resources 黑盒导出、编译 SDK。
- Gateway client 必须由生成的 typed wrapper 构造 JSON-RPC 信封；Gateway server 必须实现生成 trait 并使用生成 dispatcher。channel adapter 不得维护 method string 表，server trait 实现不得解析原始 WebSocket frame。
- Gateway WebSocket 在完成 handshake 后，把普通业务请求投递到有界 dispatcher；reader 只负责协议帧、握手/订阅状态机与入队，独立 writer 统一发送 response/event。回归测试必须让一个 Provider 请求保持 pending，并断言同一 socket 仍能在心跳 deadline 内返回 Pong。
- SDK 识别 stdin EOF 或 fatal frame 后必须先把 event output 标记为不可用并丢弃 cleanup event，再执行 Provider cleanup；标准 terminal error 只能在 cleanup 后按同一个有界 drain deadline 尝试写出，不能让不可读 stdout 的背压阻塞子进程回收，也不能把不可观测的状态事件误报成业务清理失败。
- Host shutdown 可以先关闭面向 Gateway 的 inbound consumer，但 wire reader 必须活到 shutdown response/EOF；`shutting_down` 状态下只忽略 event/notification，不忽略 response，也不主动关闭 stdout。
- Provider Host 将插件事件映射到 application/gateway channel；Desktop Companion 单独映射到 pet channel。
- Harness 的 permission profile、Section 等版本化概念只允许留在 Provider adapter 内部；除非产品协议本身需要该概念，不得为了追随原生 wire 把它暴露到 Host/Remote schema，也不得把 Section 冒充 Project。
- Gateway 在调用 Provider `conversation.list/get` 前捕获 `snapshotCursor`；客户端把该值原样传给 `event.subscribe.afterCursor`，由 Gateway 返回相同的 `subscribedAfterCursor`。
- Conversation snapshot 只承载元数据和稳定内容，进行中正文只由 live `turn.outputDelta` 承载；不要为同步方便临时引入权威正文投影或 revision delta。
- `conversation.get.limit` 表示单次最多读取的原生 turn 数，不是必须填满的数量；Provider/Gateway 只返回这一页并透传准确的 `pageInfo.nextCursor`，任何中间层都不得重新聚合全部历史。
- Codex observer 先用 `thread/read(includeTurns=false)` 读取 metadata，再用 `thread/turns/list(itemsView=full, sortDirection=desc)` 按 opaque cursor 分页；单次上游请求最多 10 turns，Provider 将当前响应页恢复为时间正序，并检测返回数量、重复 cursor 与页数上限。
- Provider 在 stdout 写入前对最终序列化帧执行共享 16 MiB 检查；超限返回 non-retryable `provider_response_too_large`，details 记录 `maxFrameBytes`，原请求 id 不变。客户端只对该错误保持原 cursor、缩小 limit 后重新请求；成功响应后才推进 cursor，相同参数不得盲目重试。
- 若 `limit=1` 仍超过帧限制，客户端缩页已经无解。Provider 必须按最终序列化尺寸在共享层截断正文；仍超限时返回通用 History omitted 占位，同时尽量保留 `nextCursor`。能提供原生 item 分页的 adapter 应先逐 item 拉取；原生页失败时返回有界 partial/placeholder，不能让读取失败阻断后续写入。不得靠继续增大统一 frame limit 掩盖。

## 来源

- `../50-decisions/language-neutral-protocol-idl-and-sdk-boundary.md`
- `../50-decisions/codex-remote-and-desktop-companion-dual-channel.md`
- `../10-architecture/protocol-layers-and-device-routing.md`
- `../../protocol/README.md`
- `../../sdk/rust/codepet-provider-sdk/src/stdio.rs`
- `../../crates/providers/codepet-provider-codex/src/main.rs`
- `../../crates/providers/codepet-provider-codex/src/client.rs`
- `../../crates/providers/codepet-provider-claude/src/main.rs`
- `../../crates/providers/codepet-provider-opencode/src/main.rs`
- `../../src-tauri/src/runtime_gateway/tauri_bridge.rs`
- `../../crates/codepet-host/src/process.rs`

## 验证方式

- `npm run protocol:check` 验证 schema/manifest 依赖方向、Pet 无 Provider ref、target fail-closed、capability metadata、fixture 与 freshness。
- `dart analyze sdk/dart/codepet-core-sdk sdk/dart/codepet-gateway-sdk` 与 Gateway package 的 `dart test` 验证 Dart null safety、canonical fixture、约束/unknown-field、union、敏感字段和 typed client。
- `cargo test --manifest-path sdk/rust/Cargo.toml` 验证 SDK 编译、bounded framing、wire classification、标准 JSON-RPC error、dispatcher、instance kind 和路由模型。
- `npm run sdkgen:test`、Bun compiled binary 黑盒测试、App resource staging 测试与导出目录的 `cargo check` 验证 schema/manifest → normalized IR → Rust SDK 的真实生成、protocol digest lock、freshness 和独立分发资源；`scripts/test_codex_provider_stdio.py` 验证独立进程只经公共 JSON-RPC wire 完成主要生命周期与 conversation 查询。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test manager_gateway` 验证 `event.subscribe` dispatch，并用 Provider 查询期间先发事件的场景锁定 `snapshotCursor` 捕获顺序。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test remote_lan_listener` 验证慢 `conversation.get` 期间 control frame 仍可收发，并继续覆盖请求/响应背压、credential revoke 与有界 shutdown。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test process_rpc shutdown_discards_late_notifications_without_closing_provider_stdout -- --exact` 锁定 shutdown 晚到消息规则；Claude fatal/backpressure 纵向测试锁定 terminal cleanup 不依赖 stdout 可写；`cargo test --manifest-path crates/Cargo.toml -p codepet-host --test builtin_provider_integration` 验证 CodePet Host/Gateway、SDK runtime 与三个内置 Provider fixture 的完整链路。
- Runtime Gateway 双链路测试断言 remote event 不进入 companion replay。
- `provider_binary_transports_a_complete_history_larger_than_one_mebibyte` 验证 1–16 MiB 的完整历史成功返回；`provider_binary_conversation_get_pages_history_without_resuming` 验证分页顺序、稳定 ID 与纯读语义；`provider_binary_returns_a_stable_error_for_oversized_history_and_keeps_serving` 验证最终投影超限时稳定报错且 Provider 继续响应；`provider_host_accepts_bounded_complete_conversation_history_frames` 验证生产 Host 使用相同上限。
- Review gateway manifest 不含 instance lifecycle/shutdown，Pet schema 不含 Provider/Conversation/Turn/Approval 类型引用。
