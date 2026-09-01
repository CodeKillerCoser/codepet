# CodePet Codex Provider

`codepet-provider-codex` 是独立的 Provider Protocol v1 / stdio JSON-lines 二进制。它由 `codepet-host` 启动；每个 Codex instance 长期持有一个纯读 observer App Server，`conversation.create` 使用完成即关闭的一次性 App Server，历史 conversation 的写操作则使用按 conversation 隔离、随 active turn 终态关闭的 execution App Server。运行依赖不包含 Host、Tauri、Pet SDK 或 Desktop 私有 IPC。

两层 wire 不相同：Provider 与 Host 之间严格使用 JSON-RPC 2.0；上游 App Server 按官方 schema 使用 `id/method/result/error`，不要求 `jsonrpc`，并允许 request 的 `trace`、notification 的 `emittedAtMs` 和缺失的 `params`。具体 method 的参数仍由 typed DTO 严格校验。

Provider 的 Host stdio reader 不逐条等待 RPC，也不在普通队列满时阻塞读 stdin：普通请求最多并发 16 个、排队 32 个；`instance.stop`、`instance.destroy`、`provider.shutdown` 另有 2 个并发与 4 个排队的保留通路。普通或控制队列过载时，请求以原 JSON-RPC id 收到 retryable `provider_overloaded`，不会进入 Provider 方法。response 允许按完成顺序乱序返回，并与 event 共用串行 stdout writer；因此 stdin EOF/fatal 即使在普通请求饱和时仍可见，并会触发 Provider/App Server 清理和有界 dispatch drain/abort。

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

该 smoke 只从环境变量读取 executable，覆盖 initialize、instance create/start、conversation list 和 instance stop；Provider 不搜索或硬编码用户路径。

完整协议矩阵、配置权威、双链路与限制见 `knowledge/10-architecture/codex-provider-plugin-runtime.md`。
