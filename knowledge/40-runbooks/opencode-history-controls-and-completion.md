# OpenCode 历史重复标识、创建控件缺失与发送后掉线

## 现象

2026-09-06，用户在 macOS 上反馈 OpenCode 发送后出现 `Provider conversation history contains a duplicate content identity`，随后无法获取会话交互权，提示 `Provider instance is not ready: opencode`；新对话中的模型、访问模式选择也消失。

## 证据

- Mapper 新增用例构造同消息两个工具及跨消息复用 tool ID，修复前发现工具结果使用重复的 `output`、`content:0`、`structured`。Host 的 `validate_conversation_items` 会检查嵌套工具结果中的 content ID，拒绝整页历史。
- 能力发现只填充 `turn_send`，`conversation_create.selection` 仍只有 `opencode-default`。Remote 的创建页面优先使用 create selection，因此不会取到已有的发送控件。Provider 的创建方法也主动拒绝 model/reasoning 和 build/plan。
- 通过隔离 XDG 配置、本地模型与 `/opt/homebrew/bin/opencode` 正式版 `1.18.25` 实际发送，修复前 `native_remote_turns_keep_the_instance_ready` 失败。原生 `/api/session/:id/wait` 返回 HTTP 503：`{"_tag":"ServiceUnavailableError","message":"Session wait is not available yet","service":"session.wait"}`。错误随后进入 `fail_from_event_forwarder`，使整个 instance 变为 Error。
- 修复后同一真实二进制连续两轮发送通过，覆盖工具审批、完成、历史唯一性及每轮重新取得交互权，共 5 次本地模型请求。

## 根因

三个独立适配缺口叠加：工具 outcome 没有继承父 item 的 ID 作用域；创建能力声明与实际支持均未接入选项；完成等待依赖了正式版 schema 中存在但未实现的路由。重复历史校验本身不会停止 instance，不能将两条错误误判为单一因果链。

## 引入历史

本次三个缺口的首次引入提交未确认。既有内容标识规约已覆盖 text/reasoning，但工具 outcome 遗漏。旧运行时文档与 fixture 假定 `/wait` 成功；当时真实 smoke 只有 health/list/shutdown，未验证完整发送流程，现已更正文档。

## 修复方案

工具结果 ID 统一派生为 `<messageId>:<partId>:output|content:<index>|structured`，Host 保留原校验。完成等待改为在非工具续接的 Step.Ended 后每 100ms 查询 `/api/session/active`，仅在 session 退出 active 集合时完成；继续校验 generation、epoch 和 turn resource，并由 stop/restart/终态取消循环。

创建能力补充 build/plan、模型、reasoning 选项，校验后传给原生 session.create。既有 create model 字段是字符串，使用带 provider 前缀的 flat catalog 避免不同 provider 的同名模型歧义；发送保留 grouped catalog。Provider/Gateway 协议仍为 v1，只更新 capability revision 指纹。

## 涉及模块

- `codepet-provider-opencode/src/mapper.rs`：修正嵌套内容标识，提供真实创建控件。
- 同模块 `provider.rs`、`client.rs`：接收创建选项，用可用的 active 查询替代 `/wait`，保持实例与 turn 的生命周期边界。
- Provider fixture、纵向与 stdio 测试：fixture `/wait` 返回与正式版相同的 503，验证创建选项传递、连续 turn、取消和跨代状态。
- `codepet-host/tests/builtin_provider_integration.rs`：实际经 Gateway 读取多工具历史与创建能力，避免只验证 mapper 而漏掉 Host 校验。
- `scripts/test_opencode_observation.py`、`tests/native_turns.rs`：复用隔离本地模型，新增 `--remote-turns` 正式二进制验证。

## 验证结果

2026-09-06 已通过：

```sh
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-opencode -p codepet-host -- --test-threads=1
python3 scripts/test_opencode_observation.py --executable /opt/homebrew/bin/opencode --remote-turns
```

第一项 98 个测试通过、2 个需要真实 runtime 的测试默认忽略；第二项显式运行新增真实发送测试并通过。全部使用隔离目录与模拟模型，不读取用户会话、不消费真实模型额度。

另有插件单测 3 项通过；普通原生 smoke 模式验证两个独立进程的成功、权限等待、失败事件通过。首次普通 smoke 在 `/agent` 初始化处 20 秒超时，补充失败日志输出后重跑通过，首次超时原因未确认。已构建 macOS arm64 `0.1.4` 本地修复包，App ad-hoc 签名校验、DMG 校验、主程序与三个内置 Provider 架构检查通过；内置 OpenCode capability 指纹与插件源码均核对为本轮版本。未生成 updater 产物，未做 Apple 公证。

## 回归防线

Mapper 覆盖同消息多工具、跨消息复用原生工具 ID、成功及失败结果；Host 集成读取两份工具结果。创建测试覆盖带 provider 的模型选择、build/plan、reasoning 及非法选项拒绝。原生测试必须连续发送并再次获取交互权，不能仅以 prompt HTTP 成功为验收条件。

## 规约候选

已补充 [内容标识规约](../60-rules/provider-conversation-content-identity.md) 与 [正式版本验证规约](../60-rules/harness-release-api-validation.md)：嵌套 outcome 同样受会话级唯一性约束；schema 存在不代表发布二进制完成了实现。

## 未知项

- 未读取用户原本报错的私有会话；安装后重新打开该会话可确认真实历史重载行为。
- Remote UI 源码控件路径与 Host 能力返回已检查，但未连接用户设备点击新对话；安装后检查模型、访问模式显示及选择创建。
- 本次真实 runtime 验证限 macOS 与正式版 1.18.25；Windows 和其他 OpenCode 版本需独立执行同一流程。
- active 查询仍采用既有单次 30 秒 HTTP 上限，整体循环无任务总时限；取消与旧 turn 不得污染新 turn，由生命周期纵向测试覆盖。


## 2026-09-07 Windows：启动超时与旧历史空页

### 证据

使用 SQLite backup 从本机数据库建立隔离副本，配置、缓存、状态均指向临时目录；不发送模型请求、不修改原始数据库。实际已安装 Provider 经 Host mux 在默认 10 秒预算下返回 `provider_request_timeout`，method=`instance.start`、stage=`stream`。仅将诊断测试预算改为 60 秒后，同一 Provider 在 19.99 秒完成启动，三条会话分别成功返回 0、3、0 个 item。源码 Provider 直调也成功，原生 HTTP 消息查询约 0.45～3.24 秒。首次隔离启动还曾触发 Server 自身的 10 秒监听地址超时；后续原生启动为 4.47 秒，首次额外耗时来源未确认。

只读数据库统计另显示：三条 session 的旧 `message` 表各有两条记录，而 V2 `session_message` 只有一条 session 有三条记录。原生 `/api/session/:id/message` 对另两条返回 HTTP 成功的空页，与 Provider 输出一致。原生日志另有模型 `FreeUsageLimitError` HTTP 429；目前没有证据将该发送错误当作历史查询错误。

### 结论与范围

已确认 Host 的请求预算短于本机完整 instance.start，启动失败可使后续历史读取不可用。OpenCode 的 instance.start 仍串行执行 Server 启动、模型/agent/provider 目录发现、`auth list` 与 `stats --days 30`，完成后才返回 Ready；可执行文件后台扫描改造没有涵盖这些初始化步骤。尚未分别计时所有步骤，不能把约 20 秒全部归到某一个 CLI 命令。

另两条空历史发生在原生 V2 API 与其存储层，不是消息传输丢失。不能直接把旧表数据插入新表，也不能据此自动新增 V1 兼容映射。本次尚未收到手机完整错误文本，因此上述两个断点与用户具体一次报错的对应关系仍待核对。

### 模块及后续验证

- Host `providers/process.rs`：默认所有 RPC 10 秒；设计修复时区分启动与普通查询预算，保留有界取消，不能全局无限等待。
- OpenCode `provider.rs`、`client.rs`：把非必需账号/用量探测从 Ready 路径移出，按统一接口有界执行并通知更新；验证慢 CLI、初始化期间断开、stop/restart 与旧代结果隔离。
- OpenCode 原生存储/API：核对指定会话旧表和 V2 结果；若要兼容旧历史，另行确定原生迁移/查询规范并验证顺序、去重、分页，不能跳过 API 私自写库。
- 手机端：保留完整错误 code/method，区分 provider 未就绪与 HTTP 成功空历史。

### 复现命令与结果

新增两个默认 ignored 的 opt-in 诊断测试，需要显式提供 `CODEPET_OPENCODE_EXECUTABLE`、`CODEPET_OPENCODE_HISTORY_SNAPSHOT`（已隔离的配置/数据根目录）；Host 测试另需 `CODEPET_OPENCODE_PROVIDER`。只输出记录数、耗时与错误，不输出消息正文。大数据只检查首批 20 条会话及各自首批 20 条消息，不能据此声称完整历史验证。

- `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-opencode --test native_history -- --ignored --nocapture`：本机隔离副本通过。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test native_history -- --ignored --nocapture`：默认 10 秒在 instance.start 超时；设置 `CODEPET_HISTORY_DIAGNOSTIC_TIMEOUT_SECONDS=60` 后通过。

60 秒只用于诊断对照，没有改生产默认超时，也未重打包、安装、升级 OpenCode 或迁移用户数据。排查暴露出的后续设计约束是启动可用性不能等待非必需的账号/用量统计；其修复仍需单独实现和回归。
