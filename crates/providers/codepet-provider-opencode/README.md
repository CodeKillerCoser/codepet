# CodePet OpenCode Provider

`codepet-provider-opencode` 是独立的 Provider Protocol v1 二进制。它由 `codepet-host` 启动；业务 payload 是 JSON-RPC JSON，stdin/stdout 物理通道由公共 SDK Runtime 封装为协商式 Yamux 多路复用。Provider 为每个 instance 管理一个仅监听 loopback 的官方 OpenCode Server 子进程。运行依赖不包含 Host、Gateway、Tauri、Pet SDK、Desktop IPC 或 activity store。

默认且仅支持 `stdio-codepet-mux-v1`，省略 transport 环境变量也使用 mux。CPRF 消息正文封装在 Yamux stream 内。Provider 实现生成的 `Provider` trait，入口只构造 `OpenCodeProvider` 并调用 `codepet-provider-sdk::serve_stdio`。公共 SDK 统一拥有 业务 initialize 前的 mux 握手、独立请求/响应 stream、JSON/raw/zstd、encoded/decoded 预算、normal/small/control 通路、背压、保序 typed event、terminal cleanup 和 shutdown drain；Provider 内部的 HTTP/SSE DTO 只描述 OpenCode v1.18.25 正式发行版实际提供的 V2 `/api` Server 形状。OpenCode 可执行文件和版本必须由 Host resolver 作为绝对 `serverExecutable` 与 `serverVersion` 注入，Provider 不搜索 PATH、应用目录或其他候选位置，也不再次探测版本。

## 构建与开发安装

```sh
cargo build --manifest-path crates/Cargo.toml -p codepet-provider-opencode
```

在 Code Pet 应用数据目录下创建 `provider-plugins/opencode/`，把 `crates/target/debug/codepet-provider-opencode`（Windows 为 `.exe`）与本目录的 `codepet-provider.json` 复制到同一目录。也可以把包含 manifest 的目录加入 `settings.providerPlugins.directories`。不要把个人 OpenCode 路径写进 manifest；Tauri Host 会删除该字段并只注入 resolver 验证后的绝对路径。

## 能力边界

当前实现只接受精确 OpenCode 1.18.25，并使用该发行版提供的 V2 `/api/health`、`/api/session`、`/api/event` 与 permission reply 路由。更早和未来版本都 fail closed：

- 支持 session list/get/create、prompt queue、活跃 turn steer/interrupt、一次性 approve/deny 和实时 SSE 事件；
- 上述 prompt/control 实现目前不通过 capability 对 Remote 宣告 `turn.start` / Gateway `turn.send`；在缺少稳定、完整的 model/reasoning discovery contract 时不会伪造可选择目录；
- `opencode-default` 表示沿用 OpenCode 自己的权限规则，Provider 不构造第二套沙箱或权限策略；approve 只映射为 `once`，从不写入 `always`；
- OpenCode 没有 Provider Protocol 的原生 Turn 对象；Provider 只把一个 session 当前活动执行投影成共享 `TurnTask`，用 `session.next.step.ended` 加 `/api/session/:id/wait` 确认一次成功终态；
- 不支持 create title、model/reasoning 选择、Provider extension、question 回答、历史 delta replay、自动重启或断线补偿；这些请求返回标准错误，不伪造成功；
- V2 `Session.location` 缺失、SSE 断开、资源超限或已支持事件出现非法官方形状时，instance 进入 Error。未知 OpenCode 事件被忽略，不通过 Hook、transcript 或字段猜测补齐。

Provider 每次启动都生成随机 `OPENCODE_SERVER_PASSWORD`，显式注入唯一 child，并由内部 HTTP/SSE client 使用同一 Basic authentication。启动总预算为 10 秒；JSON body、SSE line/event 与事件队列都有固定上限。shutdown 不调用 `/global/dispose`，而是有界 kill/wait 自己持有的 Server。

## 验证

```sh
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-opencode --all-targets
cargo test --manifest-path crates/Cargo.toml -p codepet-host --test builtin_provider_integration
```

必跑测试使用真实子进程 fixture 覆盖官方 v1.18.25 JSON/SSE 形状、turn/approval 垂直映射、stdio framing 和 Pet 隔离。若本机有 OpenCode，可显式提供 Host 等价的绝对路径运行只读 smoke；测试自身不探测路径：

```sh
CODEPET_OPENCODE_EXECUTABLE=/absolute/path/to/opencode cargo test --manifest-path crates/Cargo.toml -p codepet-provider-opencode --test provider_vertical provider_real_opencode_server_smoke -- --ignored --nocapture
```

完整数据流、风险与限制见 `knowledge/10-architecture/opencode-provider-plugin-runtime.md`。

Item mappers apply the SDK's optional `truncate_tool_item_text` helper only to `kind: tool`: text fields retain UTF-8-safe head/tail within 256 KiB and record paths and byte counts in optional item `_meta.truncations`. Other item variants stay complete. Runtime does not apply this policy or change pagination. See `knowledge/60-rules/provider-item-text-and-pagination.md`.
