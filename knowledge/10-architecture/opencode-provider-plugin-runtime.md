# OpenCode Provider 插件运行时

## 审计结论

2026-08-30 的实现基线只接受 OpenCode 正式发行版 `1.18.25`。证据来自该 tag 的 V2 protocol 源码与本机同版本二进制：Provider 使用 `GET /api/health`、`/api/session` 的 list/get/create/active/prompt/wait/interrupt、`/api/event` SSE 和 permission reply。它不调用 `/global/*`，不使用 V1 DTO fallback，也不把 `session.idle`、`session.error` 当作 V2 turn 终态。

正式 V2 shape 对本实现有三项直接约束：

- `Session.location.directory` 是必填；缺失或非绝对路径属于 shape error，不能回退到旧 `directory` 字段。
- 成功执行以 `session.next.step.ended` 提供的正式字段为证据，再由 `POST /api/session/:id/wait` 确认 session 已 authoritative completion；一个 Provider turn 只发布一次 terminal upsert。
- `permission.v2.asked` 没有上游请求时间。Provider Protocol、Gateway Protocol 与 compat v0 的 `requestedAt` 因此改为可选；OpenCode Provider 返回 `None`，不以本机 `now` 冒充上游时间。

本机可复现基线是 `/opt/homebrew/bin/opencode --version` 输出 `1.18.25`。显式路径 smoke 只执行 health、session list 和 shutdown，没有创建 session 或发送消息；测试结束后没有残留 `opencode serve`。

## 架构边界

运行数据链只有：

```text
OpenCode Server（127.0.0.1 随机端口）
  ↔ codepet-provider-opencode（正式 V2 HTTP + SSE）
  ↔ Provider Protocol v1（JSON-RPC 2.0 / stdio JSON-lines）
  ↔ PluginManager / ProviderGatewayService
```

独立 manifest 只固定 `serverArgs: ["serve"]`。Tauri `AgentRuntimeService` resolver 是 executable 和 version 的唯一权威来源：Host 在 catalog 同步前删除 manifest 中的 `serverExecutable`、`serverVersion`，再注入 resolver 返回的规范化绝对路径和版本；runtime 刷新沿用既有 Manager setting replacement 与 plugin restart。Provider 要求路径为绝对路径、参数精确为 `serve`、版本精确为 `1.18.25`，不搜索 PATH、不再次探测版本，也不附着外部 Server。

Provider crate 的生产依赖只包含生成的 Provider SDK 和 Server adapter 所需库。它不依赖 Host、Gateway、Pet SDK、Tauri、Desktop IPC、companion 或 activity store，也不发布任何 OpenCode 专用 Tauri/Pet 事件。

## NO-GO 根因与修复

### 正式终态

旧实现依赖不属于正式 V2 的 `session.idle` / `session.error`，导致正常 turn 无法可靠完成。现在 `session.next.step.ended` 更新正式时间并为当前 turn 建立一个 pending completion；唯一 waiter 调用 `/api/session/:id/wait`。wait 成功后才移除 active turn/message/pending approvals 并发布一次 Completed。重复 Step.Ended 只更新同一 pending completion 的时间，不创建第二个 waiter；旧 waiter 还要同时匹配 instance generation、conversation 和 turn resource，不能完成下一轮执行。

### 生命周期与进程树

Server startup 使用总计 10 秒 deadline；每次 health probe 的 timeout 是剩余预算与 250ms 的较小值，成功后只做至多 100ms 的 child 存活确认。startup 失败会在同一有界路径回收 child。

shutdown 不调用不存在的 V2 dispose，也不先等待 30 秒 HTTP 请求。Provider 直接 kill/wait 自己持有的 child，shutdown 与 event subscriber 共用 3 秒 deadline。stdio EOF、正常 shutdown、坏帧和写失败都经过同一个 `provider_shutdown` cleanup。Unix Host 把 Provider 放进独立进程组，外层 timeout/force-kill 一次终止 Provider 及其 Server 后代；Windows 沿用 `taskkill /T /F` 的最小进程树终止。

### 路由资源与重复操作

turn native ID 绑定当前 Server generation、conversation 和 `clientMessageId`；OpenCode prompt message ID 也绑定相同输入。approval native ID 绑定 generation、conversation 和上游 permission request ID。相同 client ID 在不同 session、stop/restart 后的 stale turn/approval，以及已成功 resolve 的 approval 都会被拒绝。approval resolve 在 HTTP 调用前从 pending map 原子占用，避免两个并发 resolve 同时发往上游；请求失败且 generation 未变化时才回填以允许重试。

### 身份与资源上限

每次启动都生成新的随机 `OPENCODE_SERVER_PASSWORD`，显式注入 child，并由内部 client 使用 Basic authentication；不会继承环境中的权威凭据。端口被 foreign process 抢占时，认证必须失败且刚启动的 child 会被回收，Provider 不能误接入错误实例。

资源上限保持固定且简单：

- Server JSON success body：4 MiB；error/no-content body：16 KiB。
- SSE 单行：1 MiB；单 event 累计 data：4 MiB。
- Server→Provider event queue：64 项，使用非阻塞固定容量 channel。
- HTTP connect timeout：3 秒；普通 request/wait timeout：30 秒；超限、慢消费者、断连或 shape error 均使当前 instance fail closed 并清理 active/pending state。

这些限制同时覆盖 Content-Length 与 chunked body；SSE fixture 使用真正的 chunked transfer 并跨 chunk 拆分 frame。

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

fixture 覆盖正式 chunked SSE/framing、Step.Ended→wait 无 interrupt 成功、单次 terminal、下一 turn、generation/stale handle、重复 resolve、startup deadline、认证端口抢占、bounded body/line/event/queue、stop/restart、坏帧 cleanup 和 Pet/Desktop/Tauri 生产依赖隔离。真实 smoke 只验证同一 `opencode serve` 的 health/list/shutdown。

`cargo check --manifest-path src-tauri/Cargo.toml --all-targets` 仍会被仓库既有缺失文件 `src/macos_window.rs` 阻断；本次受影响的 Tauri library 与 resolver 定向测试已通过。统一打包/发现由最终三 Provider 集成阶段处理，本分支不新增发行逻辑。

## 回归防线与未知项

- 版本 validator 同时拒绝 `1.18.24`、`1.18.26`、`v1.18.25` 和 development 字符串。
- fixture 对所有请求验证随机 Basic auth；foreign port fixture 验证 401 不能被当作健康实例。
- Provider boundary test 扫描 production manifest/source，禁止 Host、Pet、Desktop、Tauri 依赖和数据链引用。
- 当前真实 smoke 只在 macOS/OpenCode 1.18.25 完成；Windows/Linux 的进程树行为仍需对应平台 CI/实机验证。
- OpenCode V2 仍可能在未来版本变化；精确版本 fail closed 是当前安全边界，不是长期兼容承诺。

长期跨层约束继续以 `../60-rules/protocol-layer-and-channel-boundaries.md` 为准；Host lifecycle 与 manifest 事实见 `provider-host-device-and-plugin-runtime.md`。
