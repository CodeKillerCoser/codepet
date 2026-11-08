# Provider 执行失败原因传递与 Android 展示

## 现象

2026-11-08，Android 通过 OpenCode 发起对话后显示失败，无法看到原因。电脑已收到请求，OpenCode 报告免费额度耗尽。本次修复覆盖三个内置 Provider、共享 Agent 类型、Host/Gateway 回读以及 Flutter Remote；其他外部插件未逐一检查。

## 证据

- 实际会话 `ses_e48375153ffexftq1Ku4YNhTts` 在北京时间 04:52:24 创建、04:52:28 失败。Host 的 `~/Library/Application Support/code-pet/logs/events.jsonl` 收到 running、failed；原生 `~/.local/share/opencode/log/opencode.log` 在 `2026-11-07T20:52:28.622Z` 记录 `AI.Error.QuotaExceeded`。
- 同步路径已经存在：Provider SDK runtime 把 `ProtocolError` 编入 JSON-RPC error.data，Host process 优先还原对象，Gateway 重新编码，Remote 转为 GatewayProtocolException。问题并非所有接口都不返回错误。
- 修复前 OpenCode `session.next.step.failed` 解码后不使用 error；CodexTurn 不接收 error；Claude 仅在没有正文 delta 时补失败 result 文本。公共 TurnTask 没有 error，Android 会话详情不消费 displaySummary。
- conversation.get 原本只返回消息和活动 turn；原生历史没有保存消息的失败尝试，单靠刷新无法恢复原因。

## 根因与引入历史

执行接受与执行完成被混淆：发送 RPC 成功以后，失败必须通过终态和回读表达。共享结构、各适配器以及 UI 都存在缺口。引入历史未确认；本轮代码基线为 Code Pet `b8d22f0`、Remote `7486cad`，没有修改检查开始时已有的其他工作。

## 修复方案与涉及模块

- `protocol/agent/v1`：TurnTask 增加可选 Core ProtocolError，保持 code/message/retryable/details 统一，重新生成 Rust/Dart/TypeScript SDK。这是统一失败语义的独立改动；OpenCode 原生版本分支仍只在自身 Provider。
- OpenCode：失败事件写入 error 与 displaySummary；历史即使有部分正文也保留错误摘要；支持 1.x 包装在 message 中的模型 API 错误，提取嵌套错误消息而不透出整份响应。连接故障终态也携带原因。
- Codex：接收原生 turn.error；中间 error 通知缓存原因，到 failed 终态再使用，成功/中断不带失败对象。进程失联为活动 turn 发布带原因的终态；历史失败保留摘要。
- Claude：接收 result.errors、assistant.error，合并失败原因；结构化终态不再受已有正文影响，用户中断仍保持 interrupted。
- Provider-data/Host：在既有会话业务数据库新增独立 turn_failures 表；同版本较完整错误不被空副本覆盖，数据库回读按身份隔离、去重、每会话限 100 条，删除会话时清理。Gateway 仅初始历史页附带这些失败 turn，解决原生历史为空的情况。
- Remote：解码完整错误和回读 turn，校验路由；独立错误卡片展示原因、发生时间及可重试提示。保留部分正文，兼容旧 displaySummary；activity-summary 消息也能显示。同版本旧副本不清掉已有错误，同步异常 details 改为实际内层 details。

## 验证结果

- 本机 OpenCode **2.2.6**：`python3 scripts/test_opencode_observation.py --executable /opt/homebrew/bin/opencode --remote-turns --test-error` 通过。隔离配置与本地模型服务验证启动、会话创建/列举/读取、批准—拒绝—批准三轮、流式内容身份和真实工具标记；另注入 HTTP 402，终态和原生历史都保留 `CodePet test quota exhausted`。不使用用户模型凭据。
- OpenCode **1.18.25**：相同脚本改用隔离旧二进制通过两轮批准及注入错误；错误映射为 quota_exceeded，消息正确提取。旧版审批拒绝可能不发完整终态的既有局限仍见 [OpenCode 2 兼容记录](opencode2-startup-compatibility.md)，本轮没有声称其已修复。
- `npm run protocol:check`：22 项通过，包含合法错误对象与缺 message 的非法 fixture。
- `cargo test --manifest-path sdk/rust/Cargo.toml -p codepet-agent-sdk -p codepet-gateway-sdk -p codepet-provider-sdk`：80 项通过，5 项 opt-in 忽略。
- Provider 相关回归 **235 项通过、8 项 opt-in 忽略，另跳过 1 项已确认的基线失败**。覆盖 Claude 无正文/部分正文失败、Codex 中间错误后成功/失败、OpenCode 新旧失败事件、持久化重开/隔离/去重/删除。完整命令：`cargo test --manifest-path crates/Cargo.toml --no-fail-fast -p codepet-provider-data -p codepet-provider-codex -p codepet-provider-opencode -p codepet-provider-claude -- --skip provider_v1_round_trips_fixture_app_server_lifecycle_and_approval`。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test manager_gateway`：41 项通过，包括真实 Provider IPC → Host 事件 → Gateway 回读错误且原生 items 为空。
- Remote 的 gateway_client、conversation_timeline、conversation_detail_screen、turn_failure_view 四个测试文件：131 项通过；修改模块 analyze 通过。宽度 320/800、文字缩放 2 倍验证长错误换行、滚动可达、输入草稿和焦点保留。

## 既有失败与回归防线

Codex `provider_v1_round_trips_fixture_app_server_lifecycle_and_approval` 在历史图片断言 `!fetched_json.contains("data:image/png")` 失败；在未修改的 `b8d22f0` 隔离 worktree 同命令复现，故最后回归明确跳过此既有失败，未以修改断言掩盖。Claude dependency boundary 测试的名单遗漏已存在的 codepet-paths，本轮同步名单，未放宽 Host/Gateway/Pet/Tauri 隔离约束。

长期约束见 [Provider 终态错误规约](../60-rules/provider-terminal-errors.md)。新增原生错误形态应同时断言终态原因与后续会话可用，不能仅断言 status=failed。

## 剩余限制与升级

新 SDK 能读取未带 error/turns 的旧 payload；严格旧 SDK 不能保证接受新字段，Host、内置 Provider 与 Remote 应一起升级。`flutter build apk --debug --no-pub` 已成功生成 Android 调试包；测试未替换当前运行的桌面 App 或手机安装包。修复前已经丢失、且原生历史未保存的错误无法凭空恢复；终态回读保留最近 100 条。

提交前已迁移到包含队列功能的 Code Pet main `d45d601` 和 Remote main `ca3e80a`。迁移后再次通过 Gateway 45 项、Provider-data 36 项、Remote 131 项、协议 22 项及 Remote 静态检查；重新构建 Android 调试包。Remote SDK 已从合并后的 canonical protocol 直接重新生成，不携带 Android 原工作分支未合入 main 的 Live Updates 功能。
