# OpenCode Provider 插件运行时

## 审计结论

2026-08-30 的实现基线只接受 OpenCode 正式发行版 `1.18.25`。证据来自该 tag 的 V2 protocol 源码与本机同版本二进制：Provider 使用 `GET /api/health`、`/api/session` 的 list/get/create/active/prompt/wait/interrupt、`/api/event` SSE 和 permission reply。它不调用 `/global/*`，不使用 V1 DTO fallback，也不把 `session.idle`、`session.error` 当作 V2 turn 终态。

正式 V2 shape 对本实现有三项直接约束：

- `Session.location.directory` 是必填；缺失或非绝对路径属于 shape error，不能回退到旧 `directory` 字段。
- 成功执行以 `session.next.step.ended` 提供的正式字段为证据，再由 `POST /api/session/:id/wait` 确认 session 已 authoritative completion；一个 Provider turn 只发布一次 terminal upsert。
- `permission.v2.asked` 没有上游请求时间。Provider Protocol、Gateway Protocol 与 compat v0 的 `requestedAt` 因此改为可选；OpenCode Provider 返回 `None`，不以本机 `now` 冒充上游时间。

本机可复现基线是 `/opt/homebrew/bin/opencode --version` 输出 `1.18.25`。对该二进制的启动审计还确认：`--port 0` 仍输出并使用 `4096`，不是内核临时端口；不传 `--port` 时，正式 CLI 由 child 自己从 4096 起尝试绑定，并在 stdout 输出实际 `opencode server listening on http://127.0.0.1:<port>`。Provider 因此采用后者，不把 `--port 0` 当作不存在的能力。显式路径 smoke 只执行 health、session list 和 shutdown，没有创建 session 或发送消息；测试结束后没有残留 `opencode serve`。

## 架构边界

运行数据链只有：

```text
OpenCode Server（child 自行绑定并报告的 127.0.0.1 端口）
  ↔ codepet-provider-opencode（正式 V2 HTTP + SSE）
  ↔ Provider Protocol v1（JSON-RPC 2.0 / stdio JSON-lines）
  ↔ PluginManager / ProviderGatewayService
```

独立 manifest 只固定 `serverArgs: ["serve"]`。Tauri `AgentRuntimeService` resolver 是 executable 和 version 的唯一权威来源：Host 在 catalog 同步前删除 manifest 中的 `serverExecutable`、`serverVersion`，再注入 resolver 返回的规范化绝对路径和版本；runtime 刷新沿用既有 Manager setting replacement 与 plugin restart。Provider 要求路径为绝对路径、参数精确为 `serve`、版本精确为 `1.18.25`，不搜索 PATH、不再次探测版本，也不附着外部 Server。

Provider crate 的生产依赖只包含生成的 Provider SDK 和 Server adapter 所需库。它不依赖 Host、Gateway、Pet SDK、Tauri、Desktop IPC、companion 或 activity store，也不发布任何 OpenCode 专用 Tauri/Pet 事件。

## NO-GO 根因与修复

### 正式终态

旧实现把 prompt HTTP、SSE step 与 waiter 分散在多个 map 中，prompt 返回还会用 `entry/or_insert` 复活已经完成的 turn；只按 session 关联 Step.Ended 也会让上一轮延迟事件完成下一轮。现在每个 conversation 只有一个锁内 `ActiveTurnState`，其中原子保存 turn epoch、Provider turn、prompt message ID、当前 assistant message ID 和 wait 状态。prompt HTTP 只能 `get_mut` 仍匹配 epoch/resource/message 且仍 active 的项，找不到就返回 `turn_not_active`，绝不重建状态。

`session.next.step.started` 是 `startedAt` 的唯一来源，并原子绑定当前 assistant message ID；delta、Step.Ended、Step.Failed 必须匹配该 ID。`finish: "tool-calls"` 只结束当前 step，允许下一 assistant step；非 continuation Step.Ended 才启动唯一 `/api/session/:id/wait`。wait 与事件都再次匹配 generation、conversation、epoch 和 turn resource；完成/失败通过一次 remove 发布唯一 terminal upsert。上一轮延迟 Step.Ended、旧 waiter 或迟到的 prompt response 都不能命中下一轮。

### 生命周期与进程树

Server startup 使用总计 10 秒 deadline；每次 health probe 的 timeout 是剩余预算与 250ms 的较小值，成功后只做至多 100ms 的 child 存活确认。startup 失败会在同一有界路径回收 child。

普通 V2 请求仍有 30 秒上限；`/api/session/:id/wait` 使用独立 client，只有 3 秒 connect timeout，没有会误杀长 turn 的 30 秒总 timeout。stop/restart/shutdown 先撤销 generation 与 active state，再直接终止自有 child，连接关闭会立即取消阻塞 wait。shutdown 不调用不存在的 V2 dispose。Provider 直接 kill/wait 自己持有的 child，shutdown、event subscriber 和 stdout reader 共用 3 秒 deadline。stdio EOF、正常 shutdown、坏帧和写失败都经过同一个 `provider_shutdown` cleanup。Unix Host 把 Provider 放进独立进程组，外层 timeout/force-kill 一次终止 Provider 及其 Server 后代；Windows 沿用 `taskkill /T /F` 的最小进程树终止。

### 路由资源与重复操作

每个 Provider boot 生成随机 UUID；每次 instance start 再递增本 instance counter，二者共同组成 Server generation。turn native ID 绑定该 generation、per-turn epoch、conversation 和 `clientMessageId`；OpenCode prompt message ID 绑定相同输入。approval native ID 绑定 generation、conversation 和上游 permission request ID。即使 route/session/clientMessageId 全部相同，两个独立 Provider 进程也不会生成相同 turn handle。相同 client ID 在不同 session、stop/restart 后的 stale turn/approval，以及已成功 resolve 的 approval 都会被拒绝；旧 handle 遇到同 conversation 的新 active turn 明确返回 `stale_turn`。approval resolve 在 HTTP 调用前从 pending map 原子占用，避免两个并发 resolve 同时发往上游；请求失败且 generation 未变化时才回填以允许重试。

### 身份与资源上限

Basic auth 只认证 client，不能证明 server 身份。每次启动仍生成随机 `OPENCODE_SERVER_PASSWORD` 并显式注入 child，但端口所有权来自更直接的事实：Provider 不预选、不释放端口，也不传 `--port`；它只从自己刚启动 child 的有界 stdout pipe 接受正式 loopback listening URL，再对该 URL 做带认证的 `/api/health`。foreign process 即使先占候选端口、读取 Authorization 并返回 `200 {"healthy":true}`，也不会收到 Provider 请求；自有 child 会跳到下一端口并报告它实际持有的地址。

资源上限保持固定且简单：

- Server JSON success body：4 MiB；error/no-content body：16 KiB。
- SSE 单行：1 MiB；单 event 累计 data：4 MiB。
- child startup stdout 单行：16 KiB；只接受正式 loopback listening 行。
- Server→Provider event queue：64 项，使用非阻塞固定容量 channel。
- HTTP connect timeout：3 秒；普通 request timeout：30 秒；wait 无总 timeout并由 lifecycle 取消。超限、慢消费者、断连或 shape error 均使当前 instance fail closed 并清理 active/pending state。

这些限制同时覆盖 Content-Length 与 chunked body；SSE fixture 使用真正的 chunked transfer 并跨 chunk 拆分 frame。

### 时间与 interrupt 语义

prompt admission 的 `timeCreated` 只说明 prompt 被 Server 接纳，不能证明模型 step 已开始，因此只可推进 `updatedAt`，不能写入 `startedAt`。未知开始时间保持 `None`，直到正式 `session.next.step.started.timestamp` 到达。

interrupt 在 V2 HTTP 成功前不修改本地 turn。它先读取正式 `/api/session/active`：Server 已 idle 时，后续 204 视为 no-op，返回仍匹配的本地 turn，不伪造 Interrupted；Server 确认 active 时，也只在 HTTP 返回后、同一 epoch/resource 仍 active 的条件下原子移除并标记 Interrupted。若完成/失败事件已先移除该 turn，interrupt 返回 `turn_not_active`，不能用旧快照覆盖 terminal 状态。

## 能力与明确不支持项

Provider 诚实声明 session list/get/create、turn start/steer/interrupt 和 approval resolve。list/get/create 需要正式 V2 `location`；turn start 只用 `delivery: "queue"`，steer 只用 `delivery: "steer"`。无法关联本地 active turn 的 permission 会立即回复 `reject`，不会虚构 turn。

当前不支持 create title、model/reasoning 选择、question 回答、历史 delta replay、断线恢复、自动重启、持久 `always` approval、V1 compatibility 或未来 OpenCode 版本。未知事件可忽略；已支持事件形状错误、SSE 断开、wait 失败或资源超限会让 instance 进入 Error。升级到 `1.18.26+` 前必须重新验证正式 tag schema 与 fixture，不能按 semver 猜测兼容。

## 受影响模块

- `crates/providers/codepet-provider-opencode/`：V2 client、正式 shape、generation state、stdio cleanup、fixture 与边界测试。
- `protocol/{provider,gateway}/v1/` 与生成 SDK：把 approval `requestedAt` 改为可选，并通过既有 generator 更新 Rust/TypeScript。
- `crates/codepet-host/src/process.rs`：Provider 进程组/进程树的有界强杀。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs`：只注入 resolver 的 executable/version，不承载 OpenCode 数据。
- Gateway/compat/现有 Codex 调用方：适配可选 approval timestamp；已有真实 timestamp 的 Provider 继续返回 `Some`。

## 可复现验证

以下命令在 2026-08-30 当前 worktree 实际执行：

```sh
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-opencode --all-targets
cargo test --manifest-path crates/Cargo.toml -p codepet-host process::tests::force_kill_terminates_the_provider_process_group -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib runtime_gateway::tauri_bridge::tests::catalog_runtime_executables_are_overridden_by_resolver_values -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --test runtime_gateway_core_tests
cargo check --manifest-path crates/Cargo.toml --workspace --all-targets
cargo check --manifest-path src-tauri/Cargo.toml --lib
cargo clippy --manifest-path crates/Cargo.toml -p codepet-provider-opencode --all-targets -- -D warnings
npm run protocol:check
CODEPET_OPENCODE_EXECUTABLE=/opt/homebrew/bin/opencode cargo test --manifest-path crates/Cargo.toml -p codepet-provider-opencode --test provider_vertical provider_real_opencode_server_smoke -- --ignored --nocapture
```

fixture 覆盖正式 chunked SSE/framing、prompt response 在 SSE 前后两种重排、延迟上一 turn Step.Ended、多 assistant step、Step.Ended→wait 无 interrupt 成功、单次 terminal、下一 turn、idle interrupt no-op、stop 时取消阻塞 wait、跨 restart stale handle、重复 resolve、startup deadline、端口抢占服务读取 Basic 后返回健康响应仍不被接入、chunked JSON 超限、bounded body/line/event/queue、坏帧 cleanup 和 Pet/Desktop/Tauri 生产依赖隔离。测试用 20ms 普通 request timeout 与 100ms wait 响应等比例证明 wait 不继承普通总 timeout，不真实等待 30 秒；垂直测试同时证明阻塞 wait 下 stop 小于 1 秒且 child 已退出。两个独立 Provider 二进制进程使用完全相同 route/session/clientMessageId 时，turn resource 仍不同。真实 smoke 只验证同一 `opencode serve` 的 health/list/shutdown。

`cargo check --manifest-path src-tauri/Cargo.toml --all-targets` 在当时仍被仓库既有缺失文件 `src/macos_window.rs` 阻断；该轮受影响的 Tauri library 与 resolver 定向测试已通过。当前统一打包/发现由 `provider-host-device-and-plugin-runtime.md` 的 staging/resources 流程负责，发行包内置的是 OpenCode Provider adapter，不是 OpenCode runtime。

## 回归防线与未知项

- 版本 validator 同时拒绝 `1.18.24`、`1.18.26`、`v1.18.25` 和 development 字符串。
- fixture 对所有请求验证随机 Basic auth；foreign port fixture 即使读取该 header 并返回 200 healthy，也验证 Provider 只联系自有 child stdout 报告的端口。
- Provider boundary test 扫描 production manifest/source，禁止 Host、Pet、Desktop、Tauri 依赖和数据链引用。
- 当前真实 smoke 只在 macOS/OpenCode 1.18.25 完成；跨平台发行只保证 adapter binary/manifest 进入 bundle，OpenCode runtime 仍由用户本机 resolver 提供并按精确版本 fail closed。
- OpenCode V2 仍可能在未来版本变化；精确版本 fail closed 是当前安全边界，不是长期兼容承诺。

长期跨层约束继续以 `../60-rules/protocol-layer-and-channel-boundaries.md` 为准；Host lifecycle 与 manifest 事实见 `provider-host-device-and-plugin-runtime.md`。
