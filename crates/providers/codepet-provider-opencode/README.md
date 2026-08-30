# CodePet OpenCode Provider

`codepet-provider-opencode` 是独立的 Provider Protocol v1 / stdio JSON-lines 二进制。它由 `codepet-host` 启动，并为每个 instance 管理一个仅监听 loopback 的官方 OpenCode Server 子进程。运行依赖不包含 Host、Gateway、Tauri、Pet SDK、Desktop IPC 或 activity store。

Provider 与 Host 之间只使用生成的 `codepet-provider-sdk` DTO、dispatcher、四段 Route 和有界 `JsonLineCodec`；Provider 内部的 HTTP/SSE DTO 只描述 OpenCode v1.18.25 正式发行版实际提供的 `/api` Server 形状。OpenCode 可执行文件必须由 Host resolver 作为绝对 `serverExecutable` 注入，Provider 不搜索 PATH、应用目录或其他候选位置。

## 构建与开发安装

```sh
cargo build --manifest-path crates/Cargo.toml -p codepet-provider-opencode
```

在 Code Pet 应用数据目录下创建 `provider-plugins/opencode/`，把 `crates/target/debug/codepet-provider-opencode`（Windows 为 `.exe`）与本目录的 `codepet-provider.json` 复制到同一目录。也可以把包含 manifest 的目录加入 `settings.providerPlugins.directories`。不要把个人 OpenCode 路径写进 manifest；Tauri Host 会删除该字段并只注入 resolver 验证后的绝对路径。

## 能力边界

当前实现需要 OpenCode 1.18.25 或更新版本，并使用该发行版提供的 V2 `/api/session`、`/api/event` 与 permission reply 路由：

- 支持 session list/get/create、prompt queue、活跃 turn steer/interrupt、一次性 approve/deny 和实时 SSE 事件；
- `opencode-default` 表示沿用 OpenCode 自己的权限规则，Provider 不构造第二套沙箱或权限策略；approve 只映射为 `once`，从不写入 `always`；
- OpenCode 没有 Provider Protocol 的原生 Turn 对象；Provider 只把一个 session 当前活动执行投影成一个 `ProviderTurn`；
- 不支持 create title、model/reasoning 选择、Provider extension、question 回答、历史 delta replay、自动重启或断线补偿；这些请求返回标准错误，不伪造成功；
- SSE 断开或已支持事件出现非法官方形状时，instance 进入 Error。未知 OpenCode 事件被忽略，不通过 Hook、transcript 或字段猜测补齐。

Provider 启动的 Server 继承官方 `OPENCODE_SERVER_USERNAME` / `OPENCODE_SERVER_PASSWORD` 环境；若配置了密码，内部 HTTP/SSE client 使用同一凭据，且不会输出密码。

## 验证

```sh
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-opencode --all-targets
```

必跑测试使用真实子进程 fixture 覆盖官方 v1.18.25 JSON/SSE 形状、turn/approval 垂直映射、stdio framing 和 Pet 隔离。若本机有 OpenCode，可显式提供 Host 等价的绝对路径运行只读 smoke；测试自身不探测路径：

```sh
CODEPET_OPENCODE_EXECUTABLE=/absolute/path/to/opencode cargo test --manifest-path crates/Cargo.toml -p codepet-provider-opencode --test provider_vertical provider_real_opencode_server_smoke -- --ignored --exact
```

完整数据流、风险与限制见 `knowledge/10-architecture/opencode-provider-plugin-runtime.md`。
