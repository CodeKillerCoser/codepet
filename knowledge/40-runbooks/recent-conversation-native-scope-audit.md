# 最近会话原生活动范围与摘要感知审查

## 结论与范围

2026-09-08，R1 按 PM 明确的范围审查：active 完整性相对于 `ProviderInstanceRoute + generation` 对应的实际 backend 集合，不要求把全机所有独立 Harness 进程合并成一个实例。摘要可来自共享历史 namespace；这不使本实例自动拥有其他进程的运行状态权威。运行/等待/终结含义保持既有实现，不以沉默、旧持久记录或失联推测终态。

**Codex、Claude、OpenCode 在各自受控范围内都有可实现路径，不应仅因缺少全机监控而关闭 active。** 这是源码可行性结论，不代表实现已接通或测试通过。只有完整枚举、持续失效、错误处理和已读权威实际齐备，Host 才能广告 recent。

| Provider | 当前实例的 active 权威范围 | 完整发现与变化路径 | 冷启动/失联边界 |
| --- | --- | --- | --- |
| Codex | main read AppServer 加本 route 每个 dedicated execution AppServer | 每个 server 的 `thread/loaded/list` 全部页 + `thread/read(includeTurns=false)`，合并 native active/activeFlags；每条 server 的 status/name/lifecycle 通知驱动失效 | 新 generation 重新枚举全部 owned server；Creating/Spawning 不能当空，查询期间变化须重试或明确不完整 |
| Claude | 本 route 当前 generation 启动、管理的 CLI turn | managed conversations 的 `active_turn` 与既有 status；start、审批、审批回复、真实进程退出/失败/stop 都能维护事实并发 typed invalidation | managed 集合仅证明本 generation 所有受控执行；不能用 hooks 重建此前未知外部 CLI 的运行状态 |
| OpenCode | 本实例 native server 的 foreground drains，包括同 server 其他客户端触发的执行 | `/api/session/active` 完整 map；同 server `/api/event`，或 Provider 内单飞、有界 active/摘要核对，转换为 typed invalidation | server restart 绑定新 generation 后重建；不能把独立 CLI/serve 进程的 active 也算入本 server 的承诺 |

## 证据与版本

本次只读取现有适配器、之前保留的本机 schema、安装包 metadata 和官方源码；没有启动任何 Harness 服务。早期 404 是旧目录路径，不是目标版本不存在；随后使用正确 tag 路径重验成功，没有拿 main 代替已安装版本。

- Codex：PM 保留的本机 0.153.4 schema 为 `C:/Users/17633/AppData/Local/Temp/codepet-codex-filter-schema/v2/`。对应官方 tag `rust-v0.153.4` 的 tree commit 为 `3d2ee51ca2d5db578f328aa75e20aa22c0197c9a`。本机入口为 `C:/Users/17633/AppData/Local/OpenAI/Codex/bin/8e5b6932251c2c1c/codex.exe`；本轮未执行版本命令。
- OpenCode：`D:/Dev/global/npm/node_modules/opencode-ai/package.json` 为 `1.18.29`；官方 `v1.18.29` tree commit 为 `16747470f976aca3d362ad730bcd3fe82ecc2c9a`。
- Claude：`D:/Software/ClaudeCode/node_modules/@anthropic-ai/claude-code/package.json` 为 `2.1.263`，已不同于旧 runbook 的 `2.1.209`。安装包提供原生 exe，没有可逐行核查的 CLI 实现源码；CLI 内部全局 active 枚举不能凭猜测断言存在。下面的 managed 权威证据来自本仓库适配器，而不是声称反编译了 Claude。

官方源码缓存仅放本 worktree 忽略目录 `sdk/rust/target/r1-native-audit/`，没有修改上游或 R2/R3 业务文件。在线最新说明只作为辅助，版本绑定的判断使用上述 tag / 本机 schema。

## Codex：绕开持久列表缺口，逐 owned server 查原生活动

[`thread_processor.rs:2743`](https://github.com/openai/codex/blob/rust-v0.153.4/codex-rs/app-server/src/request_processors/thread_processor.rs#L2743) 从 ThreadManager 的全部 loaded IDs 构造列表并排序；它没有 ordinary thread/list 的持久化、sourceKind、project/section 过滤，因此可发现未落盘和不在已加载普通页中的会话。准确 wire：

```text
thread/loaded/list params {cursor?:string|null,limit?:u32|null}
result {data:string[],nextCursor:string|null}
thread/read params {threadId:string,includeTurns:false}
result {thread:Thread}
```

loaded/list 默认 limit 为全部当前 loaded 数量，native 没有固定上限；0 被 native clamp 到 1。Provider 应显式用有界页长取尽。native cursor 是最后一个 ThreadId，按字典序继续；每次调用重新读取 live 集合，并非不可变快照，因此必须以 native event epoch、server registry 和 generation fence 保护构建，再安装 Provider 自己的 immutable snapshot。不能把 sourceKind 默认过滤过的 stateDB/list 或本地 loaded_threads 缓存当完整来源。

metadata-only read 在持久摘要缺失但 thread 仍 loaded 时从 live snapshot 构建，且读取 runtime status。只有没有 persisted/live 视图的具体缺失分支生成 `thread not loaded: <id>`；本仓库 `protocol.rs::is_thread_not_loaded` 使用精确 `-32600` 和同 ID 字符串匹配。其他 read error 不可当删除；loaded→read 的竞态应重枚举/重试。此错误表示查询视图缺失，不自动证明历史已被删除。

[`thread_status.rs:223`](https://github.com/openai/codex/blob/rust-v0.153.4/codex-rs/app-server/src/thread_status.rs#L223) 按 runtime facts 产生状态变化；其 outgoing 是全局 `OutgoingMessageSender`，并非单 thread 的目标连接 sender。[`outgoing_message.rs:585`](https://github.com/openai/codex/blob/rust-v0.153.4/codex-rs/app-server/src/outgoing_message.rs#L585) 通过空目标列表走 Broadcast。因此每个 owned server 的 `thread/status/changed` 都可用作发现与失效来源，无须先订阅每个会话的正文流；turn/item 正文订阅的范围不能替代这一通路。

本仓库 `CodexInstanceRuntime::mutable.sessions/executions` 管理多个 AppServer，旧架构文档的“单共享 Server”不能替代现状。需要同时覆盖 main 与每个 dedicated slot，包含注册/启动/取消竞态；某 ID 在 dedicated server active 时，main read server 的 NotLoaded 不能覆盖它。等待标志沿用 native `activeFlags`，不能只检查本地 active_turn_id 而漏掉其他 loaded 子线程的真实状态。

## Claude：受管执行可完整枚举，hooks 不补造全局权威

`crates/providers/codepet-provider-claude/src/provider.rs` 的 `start_turn` 在锁内登记 ManagedTurn 并设置 Running；审批处理维护 WaitingApproval；`finish_turn_after_exit` 在真实 CLI 退出与输出排空后终结，stop 回收受控进程。这些路径能够维护本实例完整 active 集合并发 typed invalidation。`refresh_discovered_conversations` 在已有 active_turn 时保留 managed 状态；`summarize_claude_history` 对未受管历史映射 Idle，是既有运行状态范围，不能仅因 JSONL 最近增长改成 Running。

目前全局 Hook 安装已有 UserPromptSubmit、工具、审批、Stop/StopFailure、SessionEnd、Elicitation 等事件。官方 [Hooks reference](https://code.claude.com/docs/en/hooks) 定义它们是生命周期回调；不能由此推导出可查询的全量 active 注册表或历史 replay。实际桥接 `codepet-observation/src/forward.mjs` 有超时且投递失败直接成功退出，接收端没有持久补发；订阅前发生的状态、丢失终态和硬杀进程不能靠未来 hooks 保证重建。

因此 hooks 可触发对应已知 namespace 的摘要刷新，或在既有 Pet 观察通路提供覆盖期内证据。它不是扩大 Remote managed scope 的必要条件；不能因全局 hooks 不完整而关闭已受管 active，也不能据 hooks 沉默给外部会话制造终态。若产品以后要求全机 active，需要另行定义可恢复的权威注册和冷启动契约，当前不引入。

## OpenCode：同 server 的全局事件与 active 可组合

[`groups/session.ts:146`](https://github.com/anomalyco/opencode/blob/v1.18.29/packages/protocol/src/groups/session.ts#L146) 的 `/api/session/active` 响应为 `{data:Record<SessionId,{type:"running"}>}`，明确限定当前进程拥有的 foreground drains。不是分页 API；需取完整 map，再在 Provider 侧快照分页。[`execution/local.ts`](https://github.com/anomalyco/opencode/blob/v1.18.29/packages/core/src/session/execution/local.ts) 使用当前进程的 run coordinator，[`run-coordinator.ts`](https://github.com/anomalyco/opencode/blob/v1.18.29/packages/core/src/session/run-coordinator.ts) 在 drain 生命周期维护 map；等待 drain 完成前仍在 active 集合。审批/输入展示状态仍按既有 adapter/native 事实细分，不能仅由 running 这一单值抹平等待状态。

`/api/event` 是该 server 的 EventV2 事件流；[`event.ts`](https://github.com/anomalyco/opencode/blob/v1.18.29/packages/core/src/event.ts) 的实时 all-stream 使用进程内 PubSub。每 session durable stream 可先读取已持久事件，再等进程内唤醒；不能据名称推导所有其他进程的实时事件会自动广播过来。旧 `/global/event` 使用 Node `GlobalBus` EventEmitter，同样不是全机总线，也不应与 V2 DTO 混用。

可行实现是订阅同 server 生命周期/摘要事件，再由 Provider 内单飞核对 active map 与摘要，补同 server 未映射事件或共享存储外部写入。R2 报告的 5 秒内部核对符合这一方向，但本审查没有运行它。全局 observation plugin 能跨启用插件的进程投递将来的事件；其 128 队列、gap 与 endpoint 生命周期仍不能提供冷启动全量 active。它只作辅助失效来源。

## 外部摘要 namespace 与 active 分开处理

最近还依赖历史摘要 updatedAt、未读和删除；即使某独立进程不属于本实例 active 权威，其对共享历史 namespace 的修改仍需被 Provider 感知。不能只在 Host 主动 list/get 时更新索引，否则未缓存会话不会触发 recent 失效。

- Codex：Provider 内核对完整的 summary namespace，可用 `thread/list` 的 state DB / updated_at 顺序及 metadata-only reads，或文件/DB watcher 触发同一路径。stateDB-only 不修复旧 rollout metadata，兼容较旧写入来源时需明确选择；不靠轮询正文。周期核对须感知摘要删除/改名以及新加入历史。
- OpenCode：完整 `/api/session` 页构建摘要索引；官方 [`core/session.ts:254`](https://github.com/anomalyco/opencode/blob/v1.18.29/packages/core/src/session.ts#L254) 按创建时间排序分页，不能遇到旧 createdAt 就停止查近期 updatedAt。完整扫描后再按查询条件筛选、排序、分页，active 查询独立。
- Claude：当前摘要通过本地 JSONL 提取 title/preview；“只传摘要”不等于 Provider 完全不读本地历史文件。可采用 stat 指纹缓存、仅解析变更文件、目录 watcher 加周期核对，保持 title/preview 原语义，不能每个 tick 重读所有正文。当前 discovery 会跳过 read_dir/open 错误，refresh 只 upsert 不 prune；新核对必须区分成功完整发现、真实 NotFound 和读失败，才能发布删除。仍 managed active 或尚未落盘的记录不可仅因文件缺失删除。

上述核对统一要求：Provider 内执行、单飞、有界并发、generation 可取消；成功完整一轮才原子替换索引；失败不伪造空集/删除；只有影响摘要或 active/unread 的变化发 typed invalidation，不逐 token 重建。Host 不轮询 Harness，保持原有 snapshot fence/epoch 安装防线。

## 交付与未验证

R1 已把完整 native scope、Codex 准确字段/分页与广播路径、Claude managed 结论及 discovery 错误边界、OpenCode 同 server 边界直接通知 PM/R2/R3。R2/R3 负责实现与集成；本文不修改其业务文件，不以当前阶段性关闭 capability 当作原生无能力的证据。

后续验证路径：Codex 多 owned server 与 Creating 竞态、loaded 中旧/ephemeral 会话、审批/输入标志、广播与失败重建；Claude start/等待/exit/stop 与外部 JSONL 增改删/读失败；OpenCode 同 server 第二客户端活动、全 active map、共享 DB 更新、失联/重启；三者均验证未缓存旧会话加入 recent、错误不冒充空集合，以及聊天/项目分页不受影响。

本轮未执行编译、构建、自动化测试、Harness 服务、真实模型请求或真机验收。没有声称 CLI 内部无法观测的事实已被证明。只读代码和协议审查、官方源码抓取、安装 metadata 读取为本报告的证据边界。
