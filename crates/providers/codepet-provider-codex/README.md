# CodePet Codex Provider

`codepet-provider-codex` 是独立的 Provider Protocol v1 二进制。它由 `codepet-host` 启动；业务 payload 是 JSON-RPC JSON，stdin/stdout 物理通道由公共 SDK Runtime 封装为协商式 Yamux 多路复用。每个 Codex instance 只有一个共享 App Server，所有会话的 create/resume/get/turn/approval 复用它。SDK 用 Host 心跳中的客户端集合协调 start/stop：任一客户端在线时保留已 resume 会话，最后连接离开或心跳超时则停止 Server（包括 active turn），Provider 插件继续在线；没有会话续租或独立 observer。运行依赖不包含 Host、Tauri、Pet SDK 或 Desktop 私有 IPC。

两层 wire 不相同：Provider 与 Host 之间严格使用 JSON-RPC 2.0 业务语义，并由 Runtime 自动选择 raw/zstd、写入长度前缀、检查最终 encoded frame；上游 App Server 按官方 schema 使用 JSONL 的 `id/method/result/error`，不要求 `jsonrpc`，并允许 request 的 `trace`、notification 的 `emittedAtMs` 和缺失的 `params`。具体 method 的参数仍由 typed DTO 严格校验。

公共 Provider SDK 默认且仅支持 `stdio-codepet-mux-v1`：在业务 initialize 前完成传输握手，一个请求/响应使用一个 Yamux stream，事件使用独立 stream 并按 ACK 保序。普通业务最多并发 16 个；normal/small/control 传输额度分开，超出额度的发送者等待，control 方法和心跳保留通路。EOF、断管或物理协议错误触发 Provider/App Server 清理，单 stream 的取消不影响健康 stream。`main.rs` 只调用 `codepet_provider_sdk::serve_stdio`。CPRF 长度头仍用于 stream 内的消息正文，不能直接写入 STDIO。

共享 Server 上的单条解析错误和 RPC timeout 只影响对应请求。`client/framing.rs` 流式读取原生 JSONL，不限制 turn/整行大小；坏行 drain 后继续，stderr 保留前 64 KiB。历史以一次 full turns 请求透传 caller cursor/limit，默认 20。只有 `kind: tool` 在 item 生成时检测超大文本，每字段 256 KiB，UTF-8 head-tail，记录可选 `_meta.truncations`；其余 item 不截断。消息正文受 mux encoded/decoded 容量约束，单 stream 错误隔离。详见 `knowledge/60-rules/provider-item-text-and-pagination.md`。

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
