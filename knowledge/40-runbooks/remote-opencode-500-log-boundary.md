# 手机与目标电脑日志定位 OpenCode 500 的证据边界

2026-11-10 用户要求从手机导出包 `codepet-logs-2026-11-10T14-46-44-147255Z` 排查另一台
Mac mini 拉取 OpenCode 对话的 500，随后补充目标电脑的 `Downloads/logs/`。
本次仅分析日志与记录路径，没有修改业务代码，也没有在目标电脑复现请求。

2026-11-11 用户反馈升级 OpenCode 后问题已解决。这是目标环境恢复的用户验收；升级前后
具体版本和上游修复提交尚未核对，不据此宣称所有相对路径异常都能通过任意升级修复。

## 已有证据

设备 `device-a8b289e3-94ca-48cd-85e1-09937153c0cb` 的 `conversation.recent` 有三次错误：

| 北京时间 | 请求 ID | 耗时 | 文件/行 |
| --- | --- | --- | --- |
| 15:43:25 | remote-926 | 35 ms | codepet.3.log:152 |
| 16:18:44 | remote-1047 | 110 ms | codepet.2.log:667 |
| 16:38:54 | remote-1112 | 62 ms | codepet.2.log:2602 |

对应 RTC response 均 `isError=true`，大小 342/343 字节。说明手机收到了服务端错误回复，
不是这三次请求的回包丢失或客户端等待超时。该包没有 OpenCode 字样或 HTTP 500 错误正文，
也未记录请求的 providerId，不能据此确认三次错误均属于 OpenCode，更不能确定原生服务根因。

## 为什么界面可能有 500、导出却没有

Remote `webrtc_gateway_transport.dart` 只记录回包大小及 isError；`app_trace_recorder.dart`
仅记录 span 的 status=error；`recent_conversation_controller.dart` 捕获异常后赋给界面 error，
没有记录异常正文。`gateway_client.dart::_call` 保留 Protocol 错误供界面展示，但不额外落日志。
所以日志缺失错误正文不等于用户没遇到 500。

## 目标电脑补充证据

主日志记录 Code Pet 版本 `0.3.10-beta+c394192`，运行路径 `/Applications/Code Pet.app`。
`providers/dev.codepet.opencode/provider.log` 共 17,349 行，其中 HTTP 500 记录 15,445 条；
按北京时间统计，11 月 10 日有 8,910 条。后台约每五秒报告一次
`OpenCode atomic discovery unavailable: HTTP 500`，响应为 `UnknownError`，正文要求查服务端日志。

| 北京时间 | Provider 日志行 | 原生服务错误 ref |
| --- | --- | --- |
| 15:43:25.171 | 11802 | err_66a6713f |
| 16:18:42.267 | 12226 | err_976d1ee2 |
| 16:38:51.136 | 12468 | err_d906e539 |
| 16:38:56.233 | 12469 | err_a1f4c112 |

这些是手机失败时间附近的后台轮询错误，不是已关联的手机 RPC 错误编号。
它们确认目标机 OpenCode 原生 HTTP 接口持续失败，但仍不足以逐一归因手机的三次请求。
日志目录只有 Code Pet 主日志、Provider 日志和事件归档，没有原生 OpenCode 内部异常栈。

## 2026-11-11 原生日志补充证据

用户补充微信文件目录下的 `2026-11/opencode.log`（43,264,504 字节，18,364 行）。
上述四个 ref 全部精确命中原生日志，分别位于 12291、12708、12946、12947 行；
同属 `run=dc01f7ee`，时间与 Provider 回包一致（毫秒级）。异常链为：

```text
V2Session.list
EffectDrizzleQueryError: Failed query: select ... from "session" ... limit ?
Cause([Fail(Error: Path is not absolute: .)])
```

查询按 `time_created desc, id desc` 分页，limit=100，携带非空分页游标。
这定位到了会话列表查询执行/结果解码期间的路径异常，不是泛指所有接口不可用。
全文件 15,896 条 ERROR 中，15,877 条包含相同的 `Path is not absolute: .`。
其余错误中另有旧测试目录不存在的 ENOENT，不能替代上述四个 ref 的直接原因。

注意：SQL 参数中的 `ses_...` 是分页边界，不是已经确认的坏记录 ID。
日志没有输出异常对应的行或字段，尚不能断言是 `directory` 还是 `path` 列，也不能判断
相对路径是在写入、迁移还是读取转换中产生。历史会话数据与当前路径校验不兼容是后续检查方向。

## 调用链与影响范围

- OpenCode Provider `provider.rs::collect_atomic_summaries` 依次读取会话分页、活跃会话，
  必要时补读单个会话；`poll_atomic_changes` 也可能补读消失的会话。失败影响会话发现和同步。
- `client.rs` 对应 GET `/api/session`、`/api/session/active`、`/api/session/{id}`。
  原先统一错误没有 method/route；新日志的 `V2Session.list` 和 SQL 已将这四次错误
  定位到会话分页读取，对应本客户端的 GET `/api/session` 调用。
- 手机 Remote 的错误记录缺少正文和 Provider 信息，影响跨端关联，而不是原生 HTTP 500 的证明原因。

## 当前判断与已排除方向

已确认：即使没有手机请求，目标机后台读取 OpenCode 也会收到 HTTP 500。
手机的三次快速错误回复不是等待超时，不能用增加超时解决已观察到的服务端 500。
已确认的直接原因：原生服务读取会话分页时拒绝相对路径 `.`，抛出异常并返回 500。
尚未确认：具体记录/字段、路径产生来源、引入版本、哪个版本已修复。不能承诺升级或降级即可修好。
也不能将本问题与另一项 Codex/ChatGPT 强制接管异常直接关联。

## 下一步排查与验证

已收到 Host、Provider 和原生日志，不再要求重复导出这三份日志。
下一步核对目标实例可执行文件版本与实际数据库路径，在数据库备份副本上只读检查
`session.directory`、`session.path` 等路径字段及失败分页范围，再对照该版本原生解码代码。
不能把所有 `.` 一律替换成当前目录：需要从所属项目或工作区还原真实路径。
使用现有只读脚本 `scripts/diagnose_opencode_macos.command` 验证修复前后同一会话分页；
不要把本机另一套 OpenCode 运行实例的日志或数据当作目标实例。
隔离实例会通过 `local_runtime.rs::opencode_environment` 重设 XDG 数据目录，必须核对实际实例目录。

本次使用 Python 按 `timestampMs` 转北京时间计数、匹配原生 ref 并统计路径异常；阅读 Provider 轮询和 HTTP 调用链。
没有运行生产请求或修改运行配置。最终修复应回归失败路由及手机会话列表，并确认后台轮询恢复。

后续改进 Remote 错误日志时应记录受控错误码、方法、设备/Provider 与 trace ID；错误正文和
details 需要脱敏及长度限制，不能为排查把凭据或会话内容整包输出。最终回归应验证错误界面
与导出日志可关联，并用真实 500 fixture 验证；本次定位到直接异常，尚未实施修复。
