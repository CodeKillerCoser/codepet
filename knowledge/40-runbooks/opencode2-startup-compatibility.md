# OpenCode 2.2.6 兼容修复与旧版验证

> 2026-11-08 后续勘误：本记录最初把 2.x ordinal 当作 content 数组全局下标，这一假设不成立。正文和推理各有独立计数器；原测试未覆盖混合输出。当前修复及两版原生验证见 [用户消息流与身份碰撞](opencode-input-stream-and-content-identity.md)。


## 现象

2026-11-08，macOS 用户安装 `@opencode/cli 2.2.6` 后，Code Pet 无法启动 OpenCode instance。本轮完成工作区修复与真实隔离验证，未替换已安装 App、升级用户 Runtime 或修改用户配置和会话。

## 证据

- `/opt/homebrew/bin/opencode` 指向 `@opencode/cli/bin/opencode.exe`，版本输出 `opencode v2.2.6`。修复前 native_runtime 成功发现并选择该文件，随后约 10 秒握手超时；不是 PATH 或版本准入失败。
- 隔离 Server 正常监听；旧用户名 `codepet` 和 `OPENCODE_SERVER_PASSWORD` 对 2.2.6 请求返回 401。改用 `OPENCODE_PASSWORD`、用户名 `opencode` 后，旧 `/api/health` 返回 404，新 `/api/info` 返回包含版本和 PID 的 200。
- [官方认证说明](https://dev.opencode.ai/docs/cli/web) 与本机一致；字段及事件按 [v2.2.6 OpenAPI](https://github.com/anomalyco/opencode/blob/v2.2.6/packages/protocol/openapi.json) 核对。
- 后续真实发送依次暴露：新 session 可无 title，prompt 从嵌套 `prompt.text` 改为 `text`，返回从 admission 改为 inbox 的 `payload/time.created`，审批 body 从 `reply` 改为 `decision`。
- 新 SSE 使用 `session.step.*`、`session.text/reasoning.delta`、`permission.asked/replied`；时间在 envelope `created`，步骤真实开始时间在 `data.started`。文本与推理 content 无原生 ID，SSE 使用 content 数组的 ordinal，工具仍有独立调用 ID。
- 旧 1.18.25 的初始 `/api/model/provider/agent` 可以成功返回空快照，后续查询才出现配置模型。只查一次会丢失创建控件或拒绝合法模型。

## 根因

原实现以 1.18.25 的 `/api` 契约为基线，版本号虽不限制准入，但没有适配产品 2.x 的认证、路由、请求响应和事件差异。账号/用量 CLI 在 2.x 还会默认连接或启动共享服务，必须显式连接自有 Server。

## 引入历史

具体引入提交未确认。旧 README 的“精确限制 1.18.25”已与代码不符，本次同步修正；旧文档中的 V2 `/api` 不代表 OpenCode 产品 2.x 已受支持。

## 修复方案

统一使用用户名 `opencode`，向自有 child 的新旧密码变量注入同一随机密码。先请求旧 health，仅在 404 时按剩余预算请求 info；验证成功后固定本代新 adapter。401、错误形状、网络失败不触发另一种协议的业务写入重试。

`compatibility.rs` 把新 HTTP/SSE 转为既有内部模型；保留旧 adapter、generation/epoch、消息关联、一次性审批与 active-map 完成确认。历史 content 与 delta 都按 `ordinal:<index>` 建立身份，工具原生调用 ID 单独保存；标题缺失沿用已有 session ID 展示回退。CLI 探测连接自有 Server；成功但暂时为空的目录在 3 秒初始化窗口内重试，错误不当成空结果，已有 CLI fallback 保留。

## 涉及模块

- OpenCode `client.rs`：认证、握手、新版发送/审批、历史转换、SSE 转换及 CLI 连接信息。
- `compatibility.rs`、私有 `protocol.rs`：新版原生 DTO 归一化、可选标题及 envelope 时间；这里的 protocol 是 OpenCode 内部模型，不是公共 Provider 协议。
- `provider.rs`、`background_probe.rs`：目录初始化重试与自有 Server 探测，保持 Starting→Ready 汇总边界。
- 本 crate 的单元/fixture/native/stdio 测试及 `scripts/test_opencode_observation.py`：真实版本元数据、等待 Ready、审批执行标记、流式/历史 ID、失败后 shutdown。fixture PID 从进程全局变量改为各测试独立目录配置，避免并行测试覆盖 PID；stdio generation 测试补等 Ready。
- README、架构与规约：更新验证范围和长期边界。公共 `protocol/`、生成 SDK、Host、Gateway 与 Remote 没有修改。

## 验证结果

实际命令：

```sh
cargo test --offline --manifest-path crates/Cargo.toml -p codepet-provider-opencode --all-targets
cargo test --offline --manifest-path crates/Cargo.toml -p codepet-provider-opencode --test native_runtime -- --ignored --nocapture --test-threads=1
python3 scripts/test_opencode_observation.py --executable /opt/homebrew/bin/opencode --remote-turns
python3 scripts/test_opencode_observation.py --executable /tmp/codepet-opencode-1.18.25/opencode --remote-turns
```

- all-targets：23 单元、1 边界、4 垂直、3 stdio 测试通过；4 个需要外部条件的测试默认 ignored，native_runtime/native_turns 另显式运行。
- 本机 2.2.6：发现、启动、Ready、会话创建/list/get、模型目录、批准→拒绝→再批准三轮通过。每轮各 1 个审批；两轮成功各有流式文本，拒绝轮原生报告 Failed。批准写入临时标记、拒绝不写；delta 的 content ID 在历史中存在且唯一，之后可重新取得交互权并停止。模型均为 localhost 模拟服务，无真实模型凭据或远端模型请求。
- 1.18.25：从 npm 官方 `opencode-darwin-arm64` 包下载到临时目录，并核对 registry SHA-512 integrity；未全局安装。启动、Ready、会话、连续两轮批准发送、流式与历史 ID、工具执行标记和停止通过，分别观察到 2/1 次审批。
- `cargo clippy --offline --manifest-path crates/Cargo.toml -p codepet-provider-opencode --all-targets -- -D warnings` 未能执行：当前 stable-aarch64-apple-darwin 没有安装 cargo-clippy。Rust 编译和测试已执行；未自行变更用户 toolchain。

## 回归防线与风险

- 错误降级或越过启动预算：health/info 测试覆盖旧成功、新回退、401/500、错误形状和 unhealthy，既有 startup deadline 测试继续通过。
- 串轮、重复终态或泄漏进程：保留旧 fixture 的 generation、消息匹配、取消、端口所有权、stdio EOF 和 stop 检查；新版真实流程验证连续发送。
- 流式/历史身份不同：单元测试混合文本、工具、推理、重复文本并校验同一 ordinal；native 测试核对实际历史。
- 目录初始化导致能力缺失：fixture 前两次模型列表返回空，仍需正确发布最终能力；真实旧版再次验证。
- 版本差异进入公共业务层：按 [原生 API 兼容规约](../60-rules/provider-native-api-compatibility.md) 检查 diff 和 Provider boundary test。

## 剩余限制与未知项

额外运行旧版拒绝诊断（现在可用相同脚本加 `--test-deny` 复现）：1.18.25 发出 `permission.v2.replied`、`session.next.tool.failed` 后，观测窗口内未发出 step ended/failed，turn 等待 30 秒超时。这不算旧版拒绝端到端通过；本轮未把单个工具失败伪造成 turn 终态。2.2.6 同一拒绝路径及后续再次发送已通过。

Windows/Linux、新 npm shim/仅 `opencode2` 命令发现、真实付费模型、大历史、2.x 桌宠 observation 插件、其他 2.x 版本未验证。本轮针对 Provider HTTP/SSE adapter，未扩展 observation 插件协议或重新打包安装应用。
