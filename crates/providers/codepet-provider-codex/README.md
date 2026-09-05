# CodePet Codex Provider

`codepet-provider-codex` 是独立的 Provider Protocol v1 二进制。它由 `codepet-host` 启动；业务 payload 是 JSON-RPC JSON，stdin/stdout 物理通道由公共 SDK Runtime 封装为 Provider Frame V1。每个 Codex instance 只有一个共享 App Server，所有会话的 create/resume/get/turn/approval 复用它。SDK 用 Host 心跳中的客户端集合协调 start/stop：任一客户端在线时保留已 resume 会话，最后连接离开或心跳超时则停止 Server（包括 active turn），Provider 插件继续在线；没有会话续租或独立 observer。运行依赖不包含 Host、Tauri、Pet SDK 或 Desktop 私有 IPC。

两层 wire 不相同：Provider 与 Host 之间严格使用 JSON-RPC 2.0 业务语义，并由 Runtime 自动选择 raw/zstd、写入长度前缀、检查最终 encoded frame；上游 App Server 按官方 schema 使用 JSONL 的 `id/method/result/error`，不要求 `jsonrpc`，并允许 request 的 `trace`、notification 的 `emittedAtMs` 和缺失的 `params`。具体 method 的参数仍由 typed DTO 严格校验。

公共 Provider SDK 的 stdio reader 不逐条等待 RPC，也不在普通队列满时阻塞读 stdin：普通请求最多并发 16 个、排队 32 个；manifest 以 `dispatchLane: control` 标记的 `instance.stop`、`instance.destroy`、`provider.shutdown` 使用 2 个并发与 4 个排队的保留通路。普通或控制队列过载时，请求以原 JSON-RPC id 收到 retryable `provider_overloaded`，不会进入 Provider 方法。response 允许按完成顺序乱序返回，并与 typed event sink 共用串行 stdout writer；因此 stdin EOF/fatal 即使在普通请求饱和时仍可见，并会触发 Provider/App Server 清理和有界 dispatch drain/abort。Codex `main.rs` 只调用 `codepet_provider_sdk::serve_stdio`，不拥有另一套 transport 实现。

共享 Server 上的单条解析错误和 RPC timeout 只影响对应请求。`client/framing.rs` 流式读取原生 JSONL，不限制 turn/整行大小；坏行 drain 后继续，stderr 保留前 64 KiB。历史以一次 full turns 请求透传 caller cursor/limit，默认 20。只有 `kind: tool` 在 item 生成时检测超大文本，每字段 256 KiB，UTF-8 head-tail，记录可选 `_meta.truncations`；其余 item 不截断。最终 Provider Frame V1 上限与错误隔离不变。详见 `knowledge/60-rules/provider-item-text-and-pagination.md`。

资源身份始终是 `deviceId + providerPluginId + providerInstanceId + nativeResourceId`。每次 App Server session 使用独立 generation；只有普通 command/file 的 accept/decline 二元审批会发布，额外权限、结构化 decision 和未知 server request 使用原 JSON-RPC id 返回 `-32601`。

instance 启动时会通过官方 `model/list` 获取当前可见模型和默认模型；全局 reasoning control 只发布所有可见模型共同支持的 effort 交集，交集为空则省略。App Server initialize 返回的版本写入 `harness` 描述。该实时目录与 session generation 共同形成 capability revision；Remote 对已有空闲 conversation 发起 `turn.send` 时，Provider 会校验 revision 和选择项，再把 access mode、model、reasoning effort 与 `clientRequestId` 映射到官方 `turn/start`。成功响应立即返回 App Server 的权威 turn 和实际生效的 selection；官方 start ack 的 `items` 可以为空，此时返回 `userItem: null`，真实 item 由通知和后续 snapshot 收敛。

## 构建

```sh
cargo build --manifest-path crates/Cargo.toml -p codepet-provider-codex
```

## 开发安装

1. 在 Code Pet 当前应用数据目录下创建 `provider-plugins/codex/`。
2. 将 `crates/target/debug/codepet-provider-codex`（Windows 为 `.exe`）复制到该目录。
3. 将本目录的 `codepet-provider.json` 复制到同一目录。manifest 使用相对 Provider executable，并显式提供 App Server 参数。
4. 不要把个人 Codex 路径写入 manifest。Tauri Host 会把 Agent Runtime resolver 验证后的绝对路径作为 `appServerExecutable` 注入 instance settings。

也可以把包含 manifest 的目录加入 `settings.providerPlugins.directories`。相对目录按应用数据目录解析；重复 `pluginId` 会 fail closed。

## 验证本机 App Server

必跑测试使用官方 wire 形状的可重复 fixture。若本机有 resolver 已验证的 Codex executable，可额外运行真实纵向 smoke：

```sh
CODEPET_CODEX_EXECUTABLE=/absolute/path/to/codex cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --test provider_vertical provider_real_codex_app_server_smoke -- --ignored --exact
```

不依赖 Rust 测试内部 API 的进程级 smoke：

```sh
python3 scripts/test_codex_provider_stdio.py \
  --provider crates/target/debug/codepet-provider-codex \
  --app-server crates/target/debug/codex-app-server-fixture
```

该 smoke 通过命令行接收 Provider 与 App Server fixture 的 executable，覆盖 initialize、instance create/start、conversation list/create/get、instance stop 和 provider shutdown；Provider 不搜索或硬编码用户路径。

验证 CodePet 自身的 manifest discovery、Plugin Manager、Gateway 与关闭链路：

```sh
cargo test --manifest-path crates/Cargo.toml -p codepet-host --test builtin_provider_integration
```

完整协议矩阵、配置权威、双链路与限制见 `knowledge/10-architecture/codex-provider-plugin-runtime.md`。
