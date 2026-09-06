# OpenCode Provider 插件运行时

## 审计结论

2026-08-30 的验证基线是 OpenCode 正式发行版 `1.18.25`；2026-09-06 补充实际发送验证并修正完成等待，同日 Windows 实测 `1.18.29` 的发现、启动和会话列表。Provider 不再按 Harness 版本号限制准入，兼容性由实际接口探测判断。Provider 使用 `GET /api/health`、`/api/session` 的 list/get/create/active/prompt/interrupt、`/api/event` SSE 和 permission reply。正式二进制的 `/api/session/:id/wait` 虽存在于 schema，但实际返回 503 `Session wait is not available yet`，不能调用。它不调用 `/global/*`，不使用 V1 DTO fallback，也不把 `session.idle`、`session.error` 当作 V2 turn 终态。

正式 V2 shape 对本实现有三项直接约束：

- `Session.location.directory` 是必填；缺失或非绝对路径属于 shape error，不能回退到旧 `directory` 字段。
- 成功执行先取得非工具续接的 `session.next.step.ended`，再由 `GET /api/session/active` 确认 session 已退出 active 集合；一个 Provider turn 只发布一次 terminal upsert。
- `permission.v2.asked` 没有上游请求时间。Provider Protocol、Gateway Protocol 与 compat v0 的 `requestedAt` 因此改为可选；OpenCode Provider 返回 `None`，不以本机 `now` 冒充上游时间。

本机可复现基线是 `/opt/homebrew/bin/opencode --version` 输出 `1.18.25`。对该二进制的启动审计还确认：`--port 0` 仍输出并使用 `4096`，不是内核临时端口；不传 `--port` 时，正式 CLI 由 child 自己从 4096 起尝试绑定，并在 stdout 输出实际 `opencode server listening on http://127.0.0.1:<port>`。Provider 因此采用后者，不把 `--port 0` 当作不存在的能力。最初 smoke 仅执行 health/list/shutdown，未覆盖发送，不能证明 wait 可用。现在隔离配置与本地模拟模型覆盖创建、连续两轮发送、工具审批、完成、历史读取和重新获取交互权，见 [修复记录](../40-runbooks/opencode-history-controls-and-completion.md)。

## 架构边界

OpenCode binary 实现生成的 `Provider` trait，`main.rs` 只构造 `OpenCodeProvider` 并调用公共 `codepet-provider-sdk::serve_stdio`。SDK Runtime 统一负责 JSON-RPC JSON、CodePet Provider Frame V1、raw/zstd、普通/控制双通路、有界 queue、过载响应、typed event、串行 stdout、terminal cleanup 和 shutdown drain；本 crate 不感知 framing，只负责 OpenCode Server HTTP/SSE adapter、实例状态与子进程生命周期。

运行数据链只有：

```text
OpenCode Server（child 自行绑定并报告的 127.0.0.1 端口）
  ↔ codepet-provider-opencode（正式 V2 HTTP + SSE）
  ↔ Provider Protocol v1（JSON-RPC 2.0 / CodePet Provider Frame V1）
  ↔ PluginManager / ProviderGatewayService
```

独立 manifest 固定 `serverArgs: ["serve"]`，可以额外配置绝对 `dataDirectory` 存储根。运行时由 Provider 的 `runtime.getInstalled/select` 发现和校验，Host 负责转发及持久化选择；Windows PATH 中的 npm shim 通过 package.json 解析到原生可执行文件。版本仅用于展示，不作精确或范围限制；路径仍须为绝对路径，Server 参数仍为 `serve`，不附着外部 Server。数据目录与安装位置独立，详见 [Windows Provider 验证路径](../40-runbooks/windows-provider-runtime.md)。

Provider crate 的生产依赖只包含生成的 Provider SDK 和 Server adapter 所需库。它不依赖 Host、Gateway、Pet SDK、Tauri、Desktop IPC、companion 或 activity store，也不发布任何 OpenCode 专用 Tauri/Pet 事件。

## NO-GO 根因与修复

### 正式终态

旧实现把 prompt HTTP、SSE step 与 waiter 分散在多个 map 中，prompt 返回还会用 `entry/or_insert` 复活已经完成的 turn；只按 session 关联 Step.Ended 也会让上一轮延迟事件完成下一轮。现在每个 conversation 只有一个锁内 `ActiveTurnState`，其中原子保存 turn epoch、Provider turn、prompt message ID、当前 assistant message ID 和 wait 状态。prompt HTTP 只能 `get_mut` 仍匹配 epoch/resource/message 且仍 active 的项，找不到就返回 `turn_not_active`，绝不重建状态。

`session.next.step.started` 是 `startedAt` 的唯一来源，并原子绑定当前 assistant message ID；delta、Step.Ended、Step.Failed 必须匹配该 ID。`finish: "tool-calls"` 只结束当前 step，允许下一 assistant step；非 continuation Step.Ended 才启动唯一完成确认循环，每 100ms 查询 `/api/session/active`，直到该 session 不再 active。查询循环与事件都再次匹配 generation、conversation、epoch 和 turn resource；完成/失败通过一次 remove 发布唯一 terminal upsert。上一轮延迟 Step.Ended、旧 waiter 或迟到的 prompt response 都不能命中下一轮。

### 生命周期与进程树

实例继续只持有一个 OpenCode Server，多会话共享。公共 Provider SDK 现按 Host 心跳的客户端集合协调幂等 start/stop：任一客户端在线时保持 Server，最后连接离线或 Host 心跳过期则停止，包括 active turn；插件仍继续服务。页面退出不控制该生命周期。两段心跳、状态归属及 Host 目录分组见 [连接架构](remote-and-provider-connections.md)。三个内置 Provider 的 Host 集成测试覆盖此路径。

Server startup 带当前 attempt/generation 的取消检查；stop/shutdown 使 attempt 失效，健康检查轮询会停止并回收尚未登记的 child。spawn blocking task 在返回前登记 session，模型发现期间取消等待也能由 stop 找到并关闭它；旧 future 不得覆盖新代状态。`cancelling_start_before_health_or_during_discovery_cannot_orphan_server` 覆盖这两个窗口，断言 PID 退出且无晚到 Ready。

Server startup 使用总计 10 秒 deadline；每次 health probe 的 timeout 是剩余预算与 250ms 的较小值，成功后只做至多 100ms 的 child 存活确认。startup 失败会在同一有界路径回收 child。

普通 V2 请求和单次 active 查询仍有 30 秒上限、3 秒 connect timeout；完成确认循环没有任务总 timeout，每次查询前检查当前 generation/turn 是否有效。stop/restart/shutdown 先撤销 generation 与 active state，再直接终止自有 child，循环会退出，进行中的 HTTP 连接随 child 关闭。shutdown 不调用不存在的 V2 dispose。Provider 直接 kill/wait 自己持有的 child；SDK 在 stdio EOF 或 fatal frame 后先禁用 event output，再进入同一个 `provider_shutdown` cleanup，并对 dispatch drain 与 terminal error write 使用有界 deadline。Unix Host 把 Provider 放进独立进程组，外层 timeout/force-kill 一次终止 Provider 及其 Server 后代；Windows 沿用 `taskkill /T /F` 的最小进程树终止。

### 路由资源与重复操作

每个 Provider boot 生成随机 UUID；每次 instance start 再递增本 instance counter，二者共同组成 Server generation。turn native ID 绑定该 generation、per-turn epoch、conversation 和 `clientMessageId`；OpenCode prompt message ID 绑定相同输入。approval native ID 绑定 generation、conversation 和上游 permission request ID。即使 route/session/clientMessageId 全部相同，两个独立 Provider 进程也不会生成相同 turn handle。相同 client ID 在不同 session、stop/restart 后的 stale turn/approval，以及已成功 resolve 的 approval 都会被拒绝；旧 handle 遇到同 conversation 的新 active turn 明确返回 `stale_turn`。approval resolve 在 HTTP 调用前从 pending map 原子占用，避免两个并发 resolve 同时发往上游；请求失败且 generation 未变化时才回填以允许重试。

### 身份与资源上限

Basic auth 只认证 client，不能证明 server 身份。每次启动仍生成随机 `OPENCODE_SERVER_PASSWORD` 并显式注入 child，但端口所有权来自更直接的事实：Provider 不预选、不释放端口，也不传 `--port`；它只从自己刚启动 child 的有界 stdout pipe 接受正式 loopback listening URL，再对该 URL 做带认证的 `/api/health`。foreign process 即使先占候选端口、读取 Authorization 并返回 `200 {"healthy":true}`，也不会收到 Provider 请求；自有 child 会跳到下一端口并报告它实际持有的地址。

资源上限保持固定且简单：

- Server JSON success body：4 MiB；error/no-content body：16 KiB。
- SSE 单行：1 MiB；单 event 累计 data：4 MiB。
- child startup stdout 单行：16 KiB；只接受正式 loopback listening 行。
- Server→Provider event queue：64 项，使用非阻塞固定容量 channel。
- HTTP connect timeout：3 秒；普通 request timeout：30 秒；完成确认循环无任务总 timeout，由 lifecycle 取消。超限、慢消费者、断连或 shape error 均使当前 instance fail closed 并清理 active/pending state。

这些限制同时覆盖 Content-Length 与 chunked body；SSE fixture 使用真正的 chunked transfer 并跨 chunk 拆分 frame。

### 时间与 interrupt 语义

prompt admission 的 `timeCreated` 只说明 prompt 被 Server 接纳，不能证明模型 step 已开始，因此只可推进 `updatedAt`，不能写入 `startedAt`。未知开始时间保持 `None`，直到正式 `session.next.step.started.timestamp` 到达。

interrupt 在 V2 HTTP 成功前不修改本地 turn。它先读取正式 `/api/session/active`：Server 已 idle 时，后续 204 视为 no-op，返回仍匹配的本地 turn，不伪造 Interrupted；Server 确认 active 时，也只在 HTTP 返回后、同一 epoch/resource 仍 active 的条件下原子移除并标记 Interrupted。若完成/失败事件已先移除该 turn，interrupt 返回 `turn_not_active`，不能用旧快照覆盖 terminal 状态。

## 能力与明确不支持项

Provider 诚实声明 session list/get/create、turn start/steer/interrupt 和 approval resolve。list/get/create 需要正式 V2 `location`；turn start 只用 `delivery: "queue"`，steer 只用 `delivery: "steer"`。无法关联本地 active turn 的 permission 会立即回复 `reject`，不会虚构 turn。

创建和发送都公开原生 build/plan、模型与 reasoning variant 选择。创建请求既有 `model` 为字符串，因此 create capability 使用 `<providerID>/<modelID>` 的 flat catalog，服务端拆分后传给原生 session.create；发送继续使用 grouped catalog。旧调用方的 `opencode-default` 保留原生默认行为。此次只更换 capability revision 指纹，Provider/Gateway 仍为 v1。

当前不支持 create title、question 回答、历史 delta replay、断线恢复、自动重启、持久 `always` approval、V1 compatibility 或未来 OpenCode 版本。未知事件可忽略；已支持事件形状错误、SSE 断开、active 查询失败或资源超限会让 instance 进入 Error。升级到 `1.18.26+` 前必须重新验证正式 tag schema 与实际流程，不能按 semver 猜测兼容。

## 受影响模块

- `crates/providers/codepet-provider-opencode/`：V2 client、正式 shape、generation state、stdio cleanup、fixture 与边界测试。
- `protocol/{provider,gateway}/v1/` 与生成 SDK：把 approval `requestedAt` 改为可选，并通过既有 generator 更新 Rust/TypeScript。
- `crates/codepet-host/src/providers/process.rs`：Provider 进程组/进程树的有界强杀。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs`：只注入 resolver 的 executable/version，不承载 OpenCode 数据。
- Gateway/compat/现有 Codex 调用方：适配可选 approval timestamp；已有真实 timestamp 的 Provider 继续返回 `Some`。

## 可复现验证

以下命令在 2026-08-30 当前 worktree 实际执行：

```sh
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-opencode --all-targets
cargo test --manifest-path crates/Cargo.toml -p codepet-host --test builtin_provider_integration
cargo test --manifest-path crates/Cargo.toml -p codepet-host process::tests::force_kill_terminates_the_provider_process_group -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib runtime_gateway::tauri_bridge::tests::catalog_runtime_executables_are_overridden_by_resolver_values -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --test runtime_gateway_core_tests
cargo check --manifest-path crates/Cargo.toml --workspace --all-targets
cargo check --manifest-path src-tauri/Cargo.toml --lib
cargo clippy --manifest-path crates/Cargo.toml -p codepet-provider-opencode --all-targets -- -D warnings
npm run protocol:check
CODEPET_OPENCODE_EXECUTABLE=/opt/homebrew/bin/opencode cargo test --manifest-path crates/Cargo.toml -p codepet-provider-opencode --test provider_vertical provider_real_opencode_server_smoke -- --ignored --nocapture
```

fixture 覆盖正式 chunked SSE/framing、prompt response 在 SSE 前后两种重排、延迟上一 turn Step.Ended、多 assistant step、Step.Ended→active 查询完成、单次 terminal、下一 turn、idle interrupt no-op、stop 时取消完成确认、跨 restart stale handle、重复 resolve、startup deadline、端口抢占服务读取 Basic 后返回健康响应仍不被接入、chunked JSON 超限、bounded body/line/event/queue、坏帧 cleanup 和 Pet/Desktop/Tauri 生产依赖隔离。2026-09-06 将 fixture 的 `/wait` 改为与正式版一致的 503；client 测试用 20ms 单次请求上限与 100ms 查询间隔确认循环可跨过单次请求时限且支持取消。两个独立 Provider 二进制进程使用完全相同 route/session/clientMessageId 时，turn resource 仍不同。

2026-09-06 真实发送验证命令：`python3 scripts/test_opencode_observation.py --executable /opt/homebrew/bin/opencode --remote-turns`。隔离配置及本地模型验证两轮带工具调用的发送、审批、完成、历史 content ID 唯一以及每轮后重新取得交互权。普通 smoke 模式继续验证两个独立进程的桌宠插件成功、权限、失败事件。

`cargo check --manifest-path src-tauri/Cargo.toml --all-targets` 在当时仍被仓库既有缺失文件 `src/macos_window.rs` 阻断；该轮受影响的 Tauri library 与 resolver 定向测试已通过。当前统一打包/发现由 `provider-host-device-and-plugin-runtime.md` 的 staging/resources 流程负责，发行包内置的是 OpenCode Provider adapter，不是 OpenCode runtime。

## 回归防线与未知项

- 版本不参与准入；测试覆盖不同版本元数据不会被 decode_settings 拒绝。
- fixture 对所有请求验证随机 Basic auth；foreign port fixture 即使读取该 header 并返回 200 healthy，也验证 Provider 只联系自有 child stdout 报告的端口。
- Provider boundary test 扫描 production manifest/source，禁止 Host、Pet、Desktop、Tauri 依赖和数据链引用。
- macOS/OpenCode 1.18.25 已验证发送路径；Windows/OpenCode 1.18.29 已验证发现、启动和列表，不能将后者扩展成真实模型发送已验证。
- OpenCode V2 仍可能在未来版本变化；精确版本 fail closed 是当前安全边界，不是长期兼容承诺。

长期跨层约束继续以 `../60-rules/protocol-layer-and-channel-boundaries.md` 为准；Host lifecycle 与 manifest 事实见 `provider-host-device-and-plugin-runtime.md`。
