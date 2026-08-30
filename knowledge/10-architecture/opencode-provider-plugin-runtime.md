# OpenCode Provider 插件运行时

## 背景

2026-08-30 审计基线是 OpenCode 正式发行版 v1.18.25。官方 Server 文档声明 `opencode serve`、`/global/health` 与 SSE；同版本源码的 `packages/protocol/src/groups/{session,event,permission}.ts` 进一步给出 `/api/session` 的 list/get/create/active/prompt/interrupt、`/api/event` 和 permission reply 的实际 schema。V1 session API 没有 steer；正式发行包内标记为 experimental 的 V2 API 有 `delivery: "queue" | "steer"`，因此 Provider 以 v1.18.25 为最低版本，完整使用同一套 V2 session 语义，不混搭两个版本。

仓库已有生成的 `codepet-provider-sdk`、`ProtocolServer`/dispatcher、四段 `RoutedResourceId`、manifest catalog、Plugin Manager lifecycle 与 Provider Gateway。OpenCode 只需要成为普通独立 Provider，不需要新的插件框架。审计机器最终解析到 `/opt/homebrew/bin/opencode` 1.18.25；除正式 tag 源码、官方文档与真实子进程 fixture 外，显式路径的真实 health/list/shutdown smoke 也已通过。测试仍保持 ignored，避免普通 CI 自行探测或依赖本机安装。

## 目标

- 提供独立 `codepet-provider-opencode`、显式 manifest 和 Provider Protocol v1 stdio JSON-RPC 接入。
- 只使用 OpenCode Server 正式提供的 HTTP/SSE 能力，覆盖 lifecycle、session、turn、approval、事件与 shutdown。
- 让 OpenCode executable 只有一个权威来源：Tauri `AgentRuntimeService` resolver 注入的 instance setting。
- 保持 Provider 数据只进入 Host/Gateway remote 通道，永不进入 Pet、activity、Desktop companion 或私有 IPC。

## 非目标

- 不实现市场、下载、签名、沙箱、复杂依赖注入、自动重启/backoff 或兼容层。
- 不使用 Hook、transcript、audit 文件或字段推断补充 Server 数据。
- 不把 OpenCode HTTP/SSE DTO 写入 Provider IDL，也不复制 Provider SDK DTO。
- 不把 OpenCode question 事件伪装成二元 approval，不保存 `always` 权限。

## 现状理解

运行数据链只有：

```text
OpenCode Server（127.0.0.1 随机端口）
  ↔ codepet-provider-opencode（HTTP + SSE）
  ↔ Provider Protocol v1（JSON-RPC 2.0 / stdio JSON-lines）
  ↔ PluginManager / ProviderGatewayService
```

`codepet-provider.json` 只声明 Provider executable 和 `serverArgs: ["serve"]`。Host 在 catalog 同步前删除任何 `serverExecutable`，然后只注入 resolver 返回的规范化绝对路径；runtime 刷新通过既有 Manager setting replacement 与 plugin restart 生命周期生效。Provider 对 settings 使用 `deny_unknown_fields`，要求绝对 executable 和精确 `serve` 参数，不做第二次发现。

OpenCode 没有 Provider Protocol 原生 Turn。queue prompt 的官方 message ID 成为当前 `ProviderTurn` 的 native ID；step/delta/idle/error 事件更新该投影。steer 是同一活动 turn 的附加 prompt，不创建第二个 Provider turn。list/get 通过 `/api/session/active` 标记外部活动 session，但如果 SSE 没有提供可路由的 message ID，就诚实返回 running conversation 且不虚构 active turn。

permission request 只有在能关联当前 turn 或官方 tool source message ID 时才发布。无法路由的 permission 会立即以 `reject` fail closed，避免 OpenCode 永久等待。Provider approve 只发送 `once`，deny 发送 `reject`；permission resource 加入 Server generation，旧 instance session 的 approval 不能误命中新进程。

## 实现路径

1. `client.rs` 使用 Host 路径启动 `opencode serve --hostname 127.0.0.1 --port <ephemeral>`，轮询 health 并拒绝低于 v1.18.25 的版本；stdout 不继承，防止污染 Provider wire。
2. `protocol.rs` 只定义经官方 tag 验证的 OpenCode HTTP/SSE 形状；已支持事件形状异常会终止 event forwarder，未知事件安全忽略。
3. `mapper.rs` 把 Server session、当前执行、text/reasoning delta 与 permission 映射成生成的 Provider DTO；capability 只声明实际实现的方法、`opencode-default` 和空 model/reasoning/extension 集合。
4. `provider.rs` 复用生成的 `ProtocolServer`，管理 instance、Server generation、active turn 与 pending approval。Server/SSE 故障进入 Error 且不自动重启。
5. `main.rs` 只使用生成 dispatcher 与 `JsonLineCodec` 驱动 stdio；EOF 调用统一 shutdown，Provider shutdown 终止自己持有的 Server 子进程。

## 涉及模块

- `crates/providers/codepet-provider-opencode/`：独立 binary、manifest、Server adapter、映射和必要测试。
- `crates/Cargo.toml` / `crates/Cargo.lock`：把新 binary 纳入现有 Rust workspace。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs`：只负责把 resolver 结果注入 catalog/Manager instance setting，不承载 OpenCode 数据。
- `src-tauri/src/lib.rs`：runtime 设置刷新时沿用 Plugin Manager restart lifecycle。

## 风险

- 官方 V2 API 仍标记 experimental：最低版本固定到已审计 tag；真实形状 fixture 覆盖本 Provider 使用的每个关键端点和事件。
- SSE 只有 live stream、没有全量 delta replay：断线时 instance 进入 Error，不伪造连续输出；恢复需要显式 restart。
- 端口选择存在 bind 后交给子进程的短竞争窗口：启动 health 超时会终止子进程并返回可见错误，不改为复杂 socket handoff。
- Provider/Pet 越界：crate 依赖与源码边界测试禁止 Host、Tauri、Pet、Desktop IPC、companion 和 activity 引用；现有 remote channel 隔离测试继续覆盖 Gateway 下游。
- OpenCode 进程退出或 shutdown 卡住：Provider 拥有 child handle，shutdown 最终 kill/wait；Host 仍保留外层有界 shutdown 与 force-kill。

## 测试计划

- `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-opencode --all-targets`：stdio framing、版本/settings fail closed、真实子进程 HTTP/SSE 垂直映射、approval/steer/interrupt 与隔离边界。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib catalog_runtime_executables_are_overridden_by_resolver_values`：Codex/OpenCode resolver setting 覆盖与清除。
- `cargo test --manifest-path src-tauri/Cargo.toml --test runtime_gateway_core_tests`：Provider remote event 不进入 companion/Pet/activity。
- `cargo check --manifest-path src-tauri/Cargo.toml --lib`、`npm run protocol:check` 与 `git diff --check`：跨模块编译、生成协议 freshness 与机械检查。
- 本机存在 OpenCode 时，以 `CODEPET_OPENCODE_EXECUTABLE` 显式运行 ignored smoke；变量只属于测试，不进入 Provider 路径发现。

## 知识沉淀

长期跨层约束继续以 `../60-rules/protocol-layer-and-channel-boundaries.md` 为准；Host lifecycle 与 manifest 事实见 `provider-host-device-and-plugin-runtime.md`。本页是 OpenCode Server 版本、能力矩阵和故障语义的事实入口。

## 未知项

- v1.18.25 之后 experimental V2 schema 的兼容窗口尚无官方稳定性承诺；升级前必须重新比对正式 tag 的 protocol schema 与 fixture。
- 当前只在 macOS 上完成真实 OpenCode 1.18.25 smoke；Windows/Linux 仍由跨平台 Rust 编译与 fixture 覆盖，尚无对应平台实机结果。
- OpenCode question API 不是 Provider v1 二元 approval；在 Provider 协议扩展前保持不支持。
