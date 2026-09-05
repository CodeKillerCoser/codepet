# Codex Remote App Server Provider

## 当前状态

2026-09-05：独立 Provider binary 为每个 manifest 实例持有一个共享 Codex App Server。list/get/create/resume/turn/approval 共用它，SDK 根据 Host 心跳携带的客户端集合协调生命周期。页面退出不续租、不释放；最后连接消失会停止 Server，包括 active turn。插件本身继续在线。模块划分与完整设计见 [连接架构](../../10-architecture/remote-and-provider-connections.md)，接口矩阵见 [插件运行时](../../10-architecture/codex-provider-plugin-runtime.md)。

## 背景与目标

Remote 需要项目/会话目录、创建、读取、继续、停止和审批。Desktop 私有 IPC 不提供完整的目录与创建能力，因此 Provider 使用官方 App Server；Desktop Companion 与桌宠保持自己的 socket、状态和事件链路。两条链路允许展示同名 thread，不做跨链路 owner 同步或隐式 fallback。

共享 Server 内仍以 conversation slot 串行操作，并以 Server generation 区分审批。两个手机经同一 Host 操作同一会话共用 Server；冲突与 active turn 约束仍返回真实错误，业务请求不自动重放。

## 非目标

不恢复 Hook、audit、transcript、文件监听、PetEvent driver 或全局 reply session。不上报虚构权限、模型、项目或审批能力。不把原生 thread/unsubscribe 当作立即交还 writer；已安装 0.153.1 的探针证据见 [详情与生命周期排查](../../40-runbooks/codex-detail-size-and-capability-loading.md)。

## 协议与读取边界

协议 DTO 以本机官方 `codex-cli 0.151.0` 的 `app-server generate-json-schema` 输出、官方 App Server 文档和同一 binary 的纵向 smoke 为证据；另一个实际故障环境的 `0.151.0-alpha.7.2` 已验证 `thread/turns/list` cursor/full 可用，而 `thread/items/list` 返回 `-32601`，因此当前实现不得依赖后者。上游 wire 不要求 `jsonrpc`：request 是 `id + method`，可带或不带 `params`/`trace`；notification 是 `method`，可带或不带 `params`/`emittedAtMs`；response 必须是 `id + result` 或 `id + error` 二选一，error 必须有整数 code 与非空 message，可选 `data` 会保留在内部错误证据中。历史 fixture 若携带 `jsonrpc`，只接受 `"2.0"`。缺失 `params` 由具体 method 的 typed DTO 决定是否成立。Provider 自己面向 Host 的 stdio Provider Protocol 仍严格使用 JSON-RPC 2.0，两层 envelope 不共用判定规则。


Provider→Host 是严格 JSON-RPC 2.0，经 SDK Frame V1 raw/zstd 传输；上游是官方 JSONL，两者不能共用 envelope 宽松判定。stdio 普通请求 16 active/32 pending，生命周期 control 2 active/4 pending；应用 ping 由 reader 直接确认，实例协调独立运行。EOF/fatal 先停止心跳协调、关闭 Provider，再有界 drain。

`initialize` 必须返回 codexHome/platformFamily/platformOs/userAgent。Thread、Turn、start/resume 的必需字段严格解码，SandboxPolicy 是实际权限证据；不能用默认值伪造。

conversation.get 是纯读：metadata thread/read(includeTurns=false) 加 turns/items 分页，不调用 resume。0.152+ 可用 item API 时读取 notLoaded turns，再以 item 页释放原生 DTO；-32601 环境回退 full turns。Gateway conversation.resume 复用 acquire/get，Codex 原生 resume 固定 excludeTurns=true。Remote 每页 20、自动拉完，只有 provider_response_too_large 使用 20→10→5→1 缩页，保持 cursor 不变直到成功。

App Server 原生单行与下游 encoded frame 各有 16 MiB 边界。上游读取失败不能靠下游压缩解决，单 turn/item 仍可能超限；不能由 Host/SDK 裁剪正文。metadata 与跨页历史没有共同原子 snapshot token，客户端继续依靠事件窗口和 generation fence 收敛，不能宣称跨页原子快照。

## 涉及模块

- Provider client/protocol/mapper：原生 wire、字段、Project 探测和标准领域映射。
- provider.rs / provider/server_events.rs：共享 Server、会话槽、事件与审批 generation。
- Provider SDK：stdio、心跳 admission、每实例 start/stop 协调；Host providers/remote：各自的权威状态。
- Tauri ProviderHostState/compat：resolver 配置注入和 Gateway 薄适配，Provider 事件不得进入 companion/Pet/activity。

## 风险与验证

- CLI wire 漂移：fixture 覆盖真实 envelope 和 typed DTO；升级时从目标 binary 生成 schema 并运行显式 executable smoke。
- 多会话串扰：64 会话同 PID、并发首次 resume、A 终态保留 B、command/file 审批与新旧 generation 测试。
- 初始化和取消窗口：延迟 Server initialize 连续五轮 start/stop，resume/cancel barrier、stdio saturation stop/ping/EOF、异步 Broken pipe 测试。
- 共享故障：Server crash 使整个实例 Error，所有会话槽失效；Host 在线时 SDK 后续 ping 可再次尝试 start，但不会重发 turn/start。
- 详情与项目恢复：Remote 测试等待完整 capabilities、Provider Ready 后补拉、旧 generation 丢弃、首屏复用及全历史缩页。
- 传输与身份：Host LAN 测试覆盖认证、撤销、pending get 下应用心跳；Tauri 双链路测试确认 Provider 不污染桌宠。

## 测试计划

运行 SDK Rust/Dart、Host workspace、Codex vertical、Remote Flutter 测试/analyze、Vitest/build、protocol freshness 和导出 SDK 编译。真实模拟器仍需验证新 Host/Remote 的成套安装、后台挂起与网络切换；不能把 fixture 通过等同于安装版验收。

## 知识沉淀

生命周期约束见 [交互与共享 Server](../../60-rules/codex-provider-turn-writer-lifecycle.md)、[子进程登记与回收](../../60-rules/codex-provider-instance-session-lifecycle.md)；双链路长期边界见 [决策](../../50-decisions/codex-remote-and-desktop-companion-dual-channel.md)。

## 未知项

跨 CLI 版本的 writer ownership 错误形状仍需持续实测。超长单 item 的原生响应与跨页并发变化没有额外协议承诺；Remote 当前自动拉完历史仍有总内存与总传输量成本。手机系统长期挂起会按心跳超时离线处理。
