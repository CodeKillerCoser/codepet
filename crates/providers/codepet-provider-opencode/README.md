# CodePet OpenCode Provider

`codepet-provider-opencode` 是独立的 Provider Protocol v1 二进制，由 Host 启动。它通过 SDK 的 `stdio-codepet-mux-v1` 服务统一 JSON-RPC 接口，每个 instance 持有一个只监听 loopback 的 OpenCode Server 子进程。HTTP/SSE、原生版本兼容和进程生命周期均封装在本 crate；不依赖 Host、Gateway、Tauri、Pet SDK 或 Desktop IPC。

## 运行与兼容

Provider 通过 `runtime.getInstalled/select` 发现、验证 Runtime；Host 转发并保存选择。`serverExecutable` 必须为绝对路径，`serverArgs` 为 `["serve"]`，可另指定绝对 `dataDirectory` 隔离存储。版本号只展示，实际握手决定原生接口路径。

- 旧版 `/api/health` 成功时沿用旧请求、响应和事件。仅该路由返回 404 才探测 `/api/info`，验证成功后启用 OpenCode 2 适配。401、错误 JSON 和网络故障不当成版本信号。
- 使用固定用户名 `opencode` 和每次启动生成的随机密码，同时注入旧 `OPENCODE_SERVER_PASSWORD` 与新 `OPENCODE_PASSWORD`；旧用户名变量也显式设置。
- `src/compatibility.rs` 仅转换 OpenCode 2 原生 DTO：prompt admission、步骤/文本/审批事件、时间与历史内容标识。`src/protocol.rs` 是私有原生 DTO，不是对外 Provider schema。Provider/Gateway 协议及生成 SDK 不因本次适配变化。
- 新版 CLI 账号、用量和模型探测使用 `--server` 连接自有 Server，避免隐式启动共享服务。初始成功但为空的目录快照在短暂初始化窗口内重试。

## 能力边界

支持 session list/get/create、模型与 reasoning 选择、build/plan、prompt queue、steer/interrupt、一次性 approve/deny 和实时 SSE。`opencode-default` 沿用原生权限规则，批准仅映射 `once`，不写 `always`。

原生步骤结束后通过 `/api/session/active` 确认退出 active 集合，保持 turn generation/epoch 与消息身份检查；不调用旧版实际返回 503 的 `/wait`。OpenCode 2 文本/推理的 SSE ordinal 与历史 content 数组位置使用同一标识，工具原生调用 ID 独立保留。

不支持 create title、question 回答、扩展、历史 delta replay、自动重启或断线恢复。未知事件可忽略；已支持形状错误、SSE 断开和资源超限仍失败关闭。Server 启动预算 10 秒，握手后可返回 Starting，详细探测汇总后一次发布 Ready；停止只回收自有进程。

## 构建与验证

```sh
cargo build --manifest-path crates/Cargo.toml -p codepet-provider-opencode
cargo test --offline --manifest-path crates/Cargo.toml -p codepet-provider-opencode --all-targets
cargo test --offline --manifest-path crates/Cargo.toml -p codepet-provider-opencode --test native_runtime -- --ignored --nocapture
python3 scripts/test_opencode_observation.py --executable /absolute/path/to/opencode --remote-turns
```

最后一项使用临时 XDG 配置、隔离工作区和 localhost 模拟模型；不会发送真实模型请求。OpenCode 2 自动覆盖批准、拒绝、再次发送，旧版默认覆盖连续两轮批准；`--test-deny` 可额外诊断旧版拒绝路径。测试断言工具执行标记、审批、流式内容与历史 ID、交互权和停止。

2026-11-08 已验证 macOS 本机 2.2.6 和临时目录中的 1.18.25。旧版额外拒绝诊断仍出现原生工具失败后无步骤终态、等待超时；不将其声明为通过。桌宠 observation 插件属于独立路径，本轮没有扩展其 OpenCode 2 支持。详见 [兼容修复记录](../../../knowledge/40-runbooks/opencode2-startup-compatibility.md) 与 [架构说明](../../../knowledge/10-architecture/opencode-provider-plugin-runtime.md)。

开发安装可把构建出的二进制及 `codepet-provider.json` 放入应用数据目录的 `provider-plugins/opencode/`，或将插件目录加入 `settings.providerPlugins.directories`。本轮仅修改工作区，没有替换已安装 App。

工具文本继续使用 SDK 的 `truncate_tool_item_text`，不修改分页、其他 item 或 Provider 协议；见 [分页与文本规约](../../../knowledge/60-rules/provider-item-text-and-pagination.md)。
