# Provider 用量查询与业务存储

## 背景

原有用量统计不能作为新查询的数据源。需要分模型、分桶、输入输出与缓存 token，以及累计和每日峰值；同时明确 Provider 持久化位置。30 分钟是桶宽，不是采集周期。

## 目标

- Provider 自行采集、去重、聚合，通过 `usage.query` 提供结果；Gateway 暴露 `codepet.usage.query`。
- Host 注入 Provider 的 data、logs、databasePath；SQL 留在 Provider 业务实现。
- 未读状态改存 SQLite，保留已有客户端作用域和已读版本语义。

## 非目标

不迁移旧 JSON 或旧 token 统计，不另建统计系统，不在启动时遍历所有历史 transcript，不将额度窗口当作 token 用量。

## 现状理解

依据官方 [Claude Hooks](https://code.claude.com/docs/en/hooks#sessionend-input)，Stop、SessionEnd 不包含 usage，只提供 transcript 路径等元数据。Stop 是回复结束，SessionEnd 是会话结束；Hook 时最后一条写入可能尚未完成。

官方 [Codex app-server](https://learn.chatgpt.com/docs/app-server) 的 `account/usage/read` 提供账户摘要和可选每日总量，不含模型、半小时桶或输入输出拆分。账户 lifetime 不等于返回的每日桶之和。

OpenCode 的 [Session.getUsage](https://github.com/anomalyco/opencode/blob/dev/packages/opencode/src/session/session.ts) 将缓存从 input、reasoning 从 output 分离。统一口径需加回：input 包含缓存读写，output 包含 reasoning，total=input+output；缓存是 input 子集，不能再加到 total。未知字段返回 null，不伪造零。

## 实现路径

### 存储和目录

生产 Host 使用 `<AppData>/providers/<pluginId>/usage.sqlite`，日志位于 `<AppData>/logs/providers/<pluginId>/provider.log`。这是应用分配的目录，不修改 Harness 配置目录。stderr 同时进入内存诊断环与 JSON 行日志；5 MiB 轮换，保留一份旧文件。

`codepet-provider-data` 是共享业务库，负责 WAL、5 秒 busy timeout、外键、建表和查询，不属于 SDK transport runtime。每插件保留独立用量库；运行时选择、Host 身份、会话活动和已读状态使用实际数据根的 `app-state.sqlite`。Host 与持久会话 Provider 使用同库事务，不再在 Provider 用量库里复制 conversation_state JSON 行。未配置持久会话路径的独立测试可用内存，不能广告持久未读能力。

2026-11-08 按用户要求全新初始化，不迁移或兼容旧 JSON、provider.sqlite、conversation-state.sqlite，也不自动删除旧文件。对象、路径和生命周期见 [桌面业务状态的 SQLite 存储](desktop-storage-objects-proposal.md)。

### 采集和时间桶

Claude 的 Stop、SessionEnd、SubagentStop、StopFailure 注册对应 transcript，立即唤醒采集；其余活动仍照常转发。增量读取消息时间、模型与 usage，以 session/message ID 幂等写入。OpenCode 只转发完成的 assistant message.updated 的计量字段，不携带正文；按 session/message ID 去重。

来源时间向下归入整点半小时桶：10:29:59 归 10:00，10:30:00 归 10:30，区间左闭右开。延迟收到的事件仍归原始时间。每次最多处理 1000 个队列事件、32 个已知文件，每文件最多 4 MiB；保存 offset 和前缀指纹，处理截断、替换与未完成尾行。超长单行跳过并记日志。事件后延迟 1 秒及再 4 秒重试；每 5 分钟复查已注册来源，启动也续读这些来源。不会因启动扫描整个历史目录；停机期间从未注册的新文件不会自动发现，结果覆盖状态始终明确为 partial。

Codex 在无 cursor 的查询时读取原生账户接口，保存每日总量和原生摘要。数据集只广告 UTC 日期桶、总 token；不支持模型过滤、模型分组或半小时。原生日期标签按 UTC 边界查询，不宣称掌握上游时区换日规则。

### 查询契约

前端由 `UsagePanel.svelte` 通过 `codepetGateway.ts` 和通用 Tauri `codepet_gateway_request` bridge 直达规范 Gateway。先 list/describe 读取 usageDatasets，再调用 codepet.usage.query；按所选 Provider/数据集展示，不跨实例相加。UTC 是当前界面展示时区。筛选/刷新重新查询，加载更多沿用 cursor，revision 丢弃过时响应。未知值显示 —，覆盖不足明确提示，Codex 原生账户摘要单列。

已删除 Tauri token_usage_summary、token-usage-updated 监听、JSON 统计模块、旧图表聚合与 DTO。Provider runtime 的 usage/opaque details 同时移除，Codex 不再为 runtime 摘要读取用量或额度，OpenCode 不再调用 stats，Claude 不再维护 last-turn 统计。保留上游原生 DTO 必需的计量字段，不构成第二套统计。

Agent schema 是共享定义。请求指定 datasetId、filter.time（all 或 from/to）、modelIds/includeUnknownModel、aggregation.timeBucket（none/halfHour/hour/day/month）、IANA timeZone、groupBy、metrics、summaries、orderBy、page。

先对全部匹配数据汇总再分页。totals 是所查数据集与范围的累计；peakDaily 先按查询时区汇总一天所有匹配模型再取最大值，另可请求按模型累计/每日峰值。`nativeAccountSummary` 是未过滤的 Codex 原生账户指标，不能混作查询摘要。范围必须对齐来源精度；不能准确重分的时区边界返回 unsupported_usage_query，不按比例猜测。

返回覆盖范围、更新时间、修订号、桶是否尚未结束、每个指标完整性和 null。没有数据的桶不填零。分页快照持久化 5 分钟、最多 32 份，每页 1–1000 行，绑定实例和完整查询；过期/改过滤器返回 invalid_cursor。查询最多生成 100000 行。Hook 数据按插件观察来源共享，实例 ID 用于路由及 cursor 绑定；重复配置实例不能相加为账户总量。

## 涉及模块

- `protocol/agent|provider|gateway` 及生成 SDK：定义契约、能力和严格 DTO。
- `crates/providers/codepet-provider-data`：计量、SQLite、未读业务，不依赖 Host。
- 三个 Provider：接入各自原生来源及 usage 查询方法，同时保留独立的 `ProviderInstance.usage` 摘要，由 Gateway 转发为 `runtime.usage`。摘要是 Provider 来源范围内的账户/用量概览，不是详情查询结果，不受查询时间筛选、分页影响。Codex 读取额度与账户摘要，OpenCode 保留近 30 天 CLI 统计，Claude 保留最近一轮结果用量；不可把后两者宣称为账户全历史总量。
- `crates/codepet-host`：注入目录、日志落盘、Gateway 路由和未读存储路由。
- `src-tauri`：配置生产 Provider 用量目录和共享状态库路径。

## 风险

- 重放或修改来源导致重复计量：消息键 upsert，测试同时覆盖计数修正和跨模型/日期移动。
- 结束 Hook 早于文件落盘：尾行游标与延迟重试测试；真实 CLI 仍需验收。
- 巨量 transcript 阻塞：只读已注册文件且有每轮预算；测试增量和重启，结果显式 partial。
- 分页期间新增数据、未知值被误当零：快照绑定、null 线缆序列化和跨页摘要测试。
- 未读行为退化：保留业务状态测试并运行 Host Gateway 回归。

## 测试计划

执行 `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-data`、Host `manager_gateway` 集成测试、workspace 编译检查，以及 `node --test tools/protocol-codegen/test.mjs`、OpenCode observation-plugin 测试和 `bun test tools/cp-sdk-gen/cp-sdk-gen.test.mjs`。生成文件用 `node tools/protocol-codegen/generate.mjs --check` 检查。

本次验证：Provider 数据层 25 项、Host manager_gateway 29 项、协议生成器 20 项、OpenCode Hook 4 项、Bun SDK 导出 3 项通过；workspace cargo check 和生成文件 freshness 通过。Host lib 56 项通过，另有原有 event_journal 的 date_rotation_partial_tail_and_live_page_boundaries 在 Windows 删除文件时报 PermissionDenied，未修改该日志模块。SDK 导出清单补齐已引用模块，比较生成文件时统一 CRLF；旧 recent 测试的数字 RPC ID 改为协议要求的字符串。

桌面首次检查曾遇到 Windows 应用控制策略及缺失 provider-sdk 打包资源。后续使用仓库 stageProviderSdkResources 补齐资源，并修正规范 JSON-RPC 响应类型和旧兼容层 capability 匹配后，`cargo check --manifest-path src-tauri/Cargo.toml --lib --test runtime_gateway_core_tests -j 2 --quiet` 已通过。

## 知识沉淀

Claude 机器接口规约补充 usage 专用的只读 transcript 例外。SDK 分层继续保持 transport 与 Provider 业务持久化分离，以本文说明新的存储位置和统计语义。

## 未知项

Codex 原生每日数据保留范围、日期的上游时区及不同客户端版本支持度没有保证；Hook 漏报不能证明完整历史。真实账户端到端数据核对和完整安装包运行仍待验收。

## 当前 SQLite 列定义（2026-11-08）

完整 DDL 是 `crates/providers/codepet-provider-data/src/usage.sql`。五项指标为 `total_tokens、input_tokens、output_tokens、cache_read_tokens、cache_write_tokens`，各自使用 INTEGER 列；未提供值为 NULL。input 包含缓存，total=input+output。没有 values_json、known_json、payload、binding/result JSON 列。

| 表 | 列与对象语义 |
| --- | --- |
| usage_records | 主键 `(instance,dataset,record_id)`；at 桶起点秒、duration 秒、model 可空、五项可空 token、updated 毫秒；原始来源身份用于幂等和修正 |
| usage_buckets | 主键 `(instance,dataset,at,model)`；duration、五项 token 之和、五项 `<metric>_known_count`、records、updated；未知模型使用空串参与主键 |
| usage_native_summary | instance 主键；lifetime_tokens、peak_daily_tokens、longest_running_turn_seconds、current_streak_days、longest_streak_days，均可空 |
| usage_sources | path 主键；position 字节偏移、prefix_fingerprint、skip_partial_line 布尔、checked 毫秒；尾行状态不再编码进指纹 |
| usage_inbox | id 投递 UUID 主键；source_record_id、at、model、五项可空 token、received 毫秒；OpenCode 输入在入队前解析，不保存原始事件正文 |
| usage_snapshots | id 快照 UUID 主键；dataset、generated_at、updated_at、scope、available_from、complete_through、coverage_status、summary_flags、expires 秒 |
| usage_snapshot_queries | snapshot_id 主外键；instance、dataset、time_kind、range_from/to、time_zone、time_bucket、grouped、include_unknown、page_limit；has_models/summaries/orders 区分缺省与显式空列表 |
| usage_snapshot_query_options | `(snapshot_id,kind,ordinal)` 主键；kind 为 model/metric/summary/order，value 和可空 direction；保存查询选项与顺序 |
| usage_snapshot_entries | `(snapshot_id,kind,ordinal)` 主键；kind 为 row/total/group_total/peak/group_peak；model_present/model_id、bucket_from/to、provisional、completeness、date、peak_total、五项 token 和五项 completeness |
| usage_snapshot_accounts | snapshot_id 主外键；冻结查询时的五项原生账户摘要，避免后续页读到新摘要 |

来源作用域保持现有语义：Codex 使用实例 ID，Hook 使用插件库内的 observation。重复来源记录不累计；修正先减去旧桶贡献，再加入新桶，两者同一事务。known_count=0 返回 null；0<known_count<records 返回已知部分并标记 partial；空桶删除。

冻结快照按关联表保存，子表随头部级联删除。保存和读取均用事务；重开库后的 cursor 按完整查询字段、选项顺序与实例核对，不计算完整请求哈希。`summary_flags` 保存结果对象的可选字段存在性，避免重建时混淆缺省、空数组与 null。协议时间字符串原样保存；桶秒/更新时间毫秒单位不混用。

本轮验证：Provider 数据层 35 项通过，涵盖计量修正、来源尾行、NULL/partial、快照冻结、查询绑定、重启和会话并发。Host/桌面验证见 [存储实施记录](desktop-storage-objects-proposal.md)。以下日期章节保留此前功能交付记录，不代表本轮重新执行过前端、协议、真实账户或 Windows 验收。

## 合并主工作区验证（2026-09-09）

合并 v0 的 Hook 活动投影、会话联合发现和可选 item 更新协议。Provider 业务层承接 conversation_atoms 的显式活动事件模式；Host 未读数据库路由从事件 conversation 或 item 所属会话解析 provider，兼容无 item 的内容失效通知。`content_invalidations_advance_unread_once_per_source_update` 验证同一来源更新只推进一次未读版本。

合并后执行 `cargo check --manifest-path crates/Cargo.toml --workspace --tests --quiet`、Tauri lib/runtime_gateway_core_tests 编译检查、Provider 数据层 27 项测试、Host manager_gateway 29 项测试、前端 usageQuery/codepetGateway 4 项测试、`npm run protocol:check`（20 项）和 `npm run build`，均通过。扩展 Host lib 测试初次 57/58 通过，日志轮转测试在 Windows 遇到一次文件访问拒绝；单独复测通过，该模块本次没有改动。项目未定义 npm test，前端测试使用 `npx vitest run`。

Codex Provider lib 67 项测试通过，覆盖合并后的 Provider 行为。

## 恢复独立 Provider 用量概览（2026-09-09）

提交 `06ffe4f` 的 schema 和 Provider diff 证明：新增详情查询时同时删除了 `ProviderUsage`、Provider 实例 usage 字段、Gateway runtime.usage 及原生采集。这不是详情查询所要求的变更。概览与详情是独立业务；本次恢复被误删链路，保留数据集、SQLite、筛选与分页查询。

Codex 恢复 account/rateLimits/read 和 account/usage/read 的后台概览采集；OpenCode 恢复可取消的 stats --days 30；Claude 将 result usage/cost 写回快照并发布事件。Host 恢复大小限制与递归脱敏，手机保留 usageDatasets 解码兼容并恢复概览展示映射。各来源范围仍按原生含义展示，不把最近一轮或近 30 天数据当作全账户累计。

验证：Host lib 58、manager_gateway 29、Claude lib 7、Codex lib 67、OpenCode lib 17 项通过；Codex 账户/项目子进程测试、Claude result 概览与查询能力共存测试、OpenCode 并行探测与停止取消测试通过。手机客户端/映射/分层测试 58 项、Flutter analyze、协议检查 20 项通过。两个主工作区的 SDK 输出以 cp-sdk-gen --check 核对。初次 OpenCode 定向测试因测试服务的 stats 分支也曾被删除而超时，恢复该分支后通过。未构建安装新包，真实账户与手机验收仍需重新部署后执行。
