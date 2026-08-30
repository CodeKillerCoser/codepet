# CodePet Codex Provider

`codepet-provider-codex` 是独立的 Provider Protocol v1 / stdio JSON-lines 二进制。它由 `codepet-host` 启动，并在每个 Codex instance 内管理一个官方 Codex App Server 子进程。运行依赖不包含 Host、Tauri、Pet SDK 或 Desktop 私有 IPC。

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

完整协议矩阵、配置权威、双链路与限制见 `knowledge/10-architecture/codex-provider-plugin-runtime.md`。
