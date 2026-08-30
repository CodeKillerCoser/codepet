# CodePet Codex Provider

`codepet-provider-codex` 是独立的 Provider Protocol v1 / stdio JSON-lines 二进制。它由 `codepet-host` 启动，并在每个 Codex instance 内管理一个官方 Codex App Server 子进程。运行依赖不包含 Host、Tauri、Pet SDK 或 Desktop 私有 IPC。

两层 wire 不相同：Provider 与 Host 之间严格使用 JSON-RPC 2.0；上游 App Server 按官方 schema 使用 `id/method/result/error`，不要求 `jsonrpc`，并允许 request 的 `trace`、notification 的 `emittedAtMs` 和缺失的 `params`。具体 method 的参数仍由 typed DTO 严格校验。

资源身份始终是 `deviceId + providerPluginId + providerInstanceId + nativeResourceId`。每次 App Server session 使用独立 generation；只有普通 command/file 的 accept/decline 二元审批会发布，额外权限、结构化 decision 和未知 server request 使用原 JSON-RPC id 返回 `-32601`。

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
