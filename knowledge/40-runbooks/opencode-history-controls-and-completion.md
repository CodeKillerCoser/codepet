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

60 秒只用于诊断对照，没有改生产默认超时，也未重打包、安装、升级 OpenCode 或迁移用户数据。排查暴露出的后续设计约束是启动可用性不能等待非必需的账号/用量统计；其修复与回归见下方的启动握手拆分记录。


### 启动分段计时补证（2026-09-07）

在同一隔离副本上临时为实际 `instance_start` 各阶段添加单调时钟计时，运行原生 history 测试后撤回计时代码。单次观测如下，不能当作固定性能保证：

| 阶段 | 耗时 |
| --- | ---: |
| 子进程启动、监听地址报告、health 成功及存活确认 | 4.630 秒 |
| model API（有结果，未走 `opencode models` 回退） | 2.720 秒 |
| agent API | 0.158 秒 |
| provider API | 0.035 秒 |
| `opencode auth list` | 4.243 秒 |
| `opencode stats --days 30` | 2.750 秒 |
| 启动至账号/用量探测结束 | 14.540 秒 |

因此这次 Server 本身在 10 秒内就绪；Host 的 10 秒整个 RPC 预算在账号探测阶段耗尽。账号和用量两个非必需 CLI 串行增加约 7 秒。此前约 20 秒的实测与本次差异说明启动耗时会变化，不能把阈值简单贴着单次结果调整。

代码还暴露两项边界缺口：`probe_opencode_account_metadata` 在 async 启动任务里直接阻塞执行 `Command.output`，两个命令没有执行 deadline；`discover_cli_models` 的可选 CLI 回退也没有执行 deadline，且输出上限在收集结束后检查。本次 model API 非空，未执行该回退，不能把回退计入本次耗时。

超时层次需分开：Host 全 RPC 默认 10 秒；Server 启动阶段另有 10 秒；每个普通 HTTP 请求允许 30 秒；CLI output 无 deadline。Server 阶段的 10 秒不是整个 instance.start 的总预算。这份分段计时是修改前证据；现已将详细探测整体移出 Ready 路径，处理方式见下节。


### 启动握手拆分（2026-09-07）

三个 Provider 统一采用握手成功即 Ready、后台详细探测通知的边界。OpenCode 的模型目录、账号、用量和版本在 Ready 后并行执行；Claude 的版本与账号并行；Codex 的模型、项目、账号、额度和用量 RPC 并行。具体进程和跨代约束见 [后台信息规约](../60-rules/provider-background-details.md)。Host 同时防止初始启动响应覆盖已经收到的详细信息通知。

本机原生 Windows 验证不发送模型请求、使用隔离数据目录：Claude 2.1.209 握手约 0.23 毫秒（无持久 Harness 进程），Codex 0.153.4 约 0.50 秒，OpenCode 1.18.29 约 2.80 秒。历史副本通过真实 Host mux、默认 10 秒预算时握手约 4.87 秒，随后消息数仍为 0/3/0。不同数据目录的耗时不能直接视作同等基准；没有升级 Harness 或修改原数据库。

验证命令：`cargo test --offline --manifest-path crates/Cargo.toml -p codepet-provider-claude -p codepet-provider-codex -p codepet-provider-opencode --test native_runtime installed_ -- --ignored --nocapture --test-threads=1`，以及上述 Host native_history 命令。macOS 原生行为和更新后的 release 安装包尚未验证；本次没有重新打包安装。旧历史空页是原生存储/API 的独立问题，本次没有实现历史兼容迁移。


#### 自动化回归与剩余边界

握手拆分后的检查：Claude 单元及纵向测试通过（4+10），OpenCode 单元及纵向测试通过（17+3，1 个原生发送测试默认忽略），Codex 单元测试 56 项通过，纵向测试 44 项通过、1 个原生发送测试默认忽略；SDK 单元测试 30 项通过，4 个子进程 fixture 默认忽略；Host 单元测试 37 项通过。

Codex 的 `provider_binary_keeps_eof_visible_after_at_least_forty_nine_saturated_frames` 在本机仍超过测试的 2 秒退出预算。将 `CODEPET_TEST_PROVIDER_EXE` 指向已安装的旧版 `D:/Software/Code Pet/provider-plugins/codex/codepet-provider-codex.exe` 后，同一测试也失败；因此不能将此现象认定为握手拆分引入。本次未放宽这项测试，也没有确认其最终根因。检查中另外发现版本扫描只 abort 外层任务而未取消阻塞进程，已用 RuntimeProbeControl 修复并通过早/晚登记的取消测试；它不是上述 EOF 问题已经解决的证据。

20 MiB 历史测试在 Windows debug 实测约 7.76 秒，超过旧测试的 5 秒等待。仅这些多 MiB 数据测试改用 30 秒等待；生产 Host 的默认 10 秒预算保持原值。后台失败通知测试按 CLI 的 30 秒执行上限等待，与握手预算分开。

Host/Gateway 端到端检查通过：`cargo test --offline --manifest-path crates/Cargo.toml -p codepet-host --test builtin_provider_integration --test manager_gateway -- --test-threads=1`（1+24 项）。最终原生 CLI 复测三项通过，Codex 约 0.53 秒、OpenCode 约 2.83 秒。本机这轮原始日志保存在 `C:/Users/17633/AppData/Local/Temp/codepet-provider-probes-20260907-4ea0d01a`，包含旧版 EOF 对照；临时日志不提交仓库。
