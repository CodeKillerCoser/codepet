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

生产 Host 使用 `<AppData>/providers/<pluginId>/data/provider.sqlite`，日志放同级 `logs/provider.log`。这是 Provider 目录，不修改 Harness 配置目录。stderr 仍进入内存诊断环，同时落 JSON 行日志；5 MiB 轮换，保留一份旧文件。

每个插件持有一个 SQLite 文件，Host 只分配路径。`codepet-provider-data` 是共享业务库，负责 WAL、5 秒 busy timeout、建表和查询；它位于 Providers 目录，不属于 SDK transport runtime。用量保存来源记录、预聚合桶、已知文件游标、待处理事件及分页快照。记录修正与桶增减在同一事务中完成。未读状态保存在 `conversation_state` 表的结构化文档行中；Host facade 按实例路由至插件 DB，沿用业务库执行读写。未配置 Provider 根目录的独立测试/兼容 Host 可使用独立 SQLite 或内存。旧文件不导入。

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
- 三个 Provider：接入各自原生来源及 usage 方法，移除独立 runtime usage 摘要。
- `crates/codepet-host`：注入目录、日志落盘、Gateway 路由和未读存储路由。
- `src-tauri`：配置生产 Provider 根目录及 SQLite fallback 文件名。

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

## 当前 SQLite 列定义

主表 token 值是 JSON 数组，顺序固定 `[total,input,output,cacheRead,cacheWrite]`，不是五个独立 INTEGER 列。记录表允许 null；聚合表保存已知部分之和，用 known_json 记录已知数量。input 包含缓存，total=input+output。

### usage_records：来源记录与幂等账本

| 列 | SQL 类型 | 定义 |
| --- | --- | --- |
| instance | TEXT NOT NULL | 来源作用域；Codex 为实例 ID，Hook 使用 observation，表示插件共享来源 |
| dataset | TEXT NOT NULL | observed-model-tokens 或 codex-account-daily |
| record_id | TEXT NOT NULL | 稳定来源身份：消息 ID 或原生每日记录 ID，不是随机投递 ID |
| at | INTEGER NOT NULL | 所属桶起点，Unix 秒，已向下对齐 |
| duration | INTEGER NOT NULL | 桶宽秒数：Hook 1800，Codex 86400 |
| model | TEXT NULL | 模型 ID，NULL 表示未知/不提供 |
| values_json | TEXT NOT NULL | 五项可空 token 数组 |
| updated | INTEGER NOT NULL | 最后更新时间，Unix 毫秒 |

主键 `(instance,dataset,record_id)`；时间索引 `(instance,dataset,at)`。重复记录不累计，修正先减去旧桶贡献再加入新桶。

### usage_buckets：查询预聚合

| 列 | SQL 类型 | 定义 |
| --- | --- | --- |
| instance、dataset | TEXT NOT NULL | 同来源记录表 |
| at | INTEGER NOT NULL | 桶起点，Unix 秒 |
| duration | INTEGER NOT NULL | 桶宽，秒 |
| model | TEXT NOT NULL | 模型 ID，未知使用空串参与主键 |
| values_json | TEXT NOT NULL | 五项指标已知值之和，JSON 数组 |
| known_json | TEXT NOT NULL | 每项指标有数值的来源记录数，五项整数数组 |
| records | INTEGER NOT NULL | 桶内来源记录总数，不是 session 数 |
| updated | INTEGER NOT NULL | 最后更新时间，Unix 毫秒 |

主键 `(instance,dataset,at,model)`。某指标 known=0 返回 null；0<known<records 返回已知部分之和并标记 partial。最后一条记录移出后删除空桶。

### 辅助表

| 表 | 全部列及定义 |
| --- | --- |
| usage_sources | `path TEXT PRIMARY KEY`：注册的 transcript 绝对路径；`position INTEGER NOT NULL`：字节偏移；`fingerprint TEXT NOT NULL`：前缀指纹/尾行跳过状态；`checked INTEGER NOT NULL DEFAULT 0`：最近检查 Unix 毫秒，0 表示优先检查 |
| usage_inbox | `id TEXT PRIMARY KEY`：投递事件 ID；`provider TEXT NOT NULL`：来源适配器；`payload TEXT NOT NULL`：待处理事件 JSON；`received INTEGER NOT NULL`：接收 Unix 毫秒，成功消费后删除该行 |
| usage_snapshots | `id TEXT PRIMARY KEY`：快照 UUID；`binding TEXT NOT NULL`：序列化查询和实例绑定；`result TEXT NOT NULL`：完整查询结果 JSON；`expires INTEGER NOT NULL`：过期 Unix 秒 |
| usage_native_summary | `instance TEXT PRIMARY KEY`：Codex 实例；`payload TEXT NOT NULL`：原生账户累计、每日峰值等摘要 JSON，不受查询过滤影响 |
| conversation_state | `id INTEGER PRIMARY KEY CHECK(id=1)`：唯一文档行；`document TEXT NOT NULL`：客户端已读版本、活动指纹等结构化 JSON，非 token 表 |

本次清理未改列、未迁移旧统计文件。前端查询/API/样式 49 项测试、Host manager_gateway 29 项、采集和查询 13 项、协议生成器 20 项通过，前端生产构建通过。预览使用模拟 Gateway 响应，验证 1280px 与 390px 布局、键盘焦点、空结果和 Codex 能力限制；尚未代替真实账户验收。全量 tsc 暴露原有 Node 类型缺失及 activity、sound 等测试类型错误，新查询模块无报错。

清理后补充验证：OpenCode account_probes_are_parallel_notify_later_and_cancel_on_stop 通过，确认移除 stats 后初始化与 stop 取消行为正常；git diff --check 通过。

## 合并主工作区验证（2026-09-09）

合并 v0 的 Hook 活动投影、会话联合发现和可选 item 更新协议。Provider 业务层承接 conversation_atoms 的显式活动事件模式；Host 未读数据库路由从事件 conversation 或 item 所属会话解析 provider，兼容无 item 的内容失效通知。`content_invalidations_advance_unread_once_per_source_update` 验证同一来源更新只推进一次未读版本。

合并后执行 `cargo check --manifest-path crates/Cargo.toml --workspace --tests --quiet`、Tauri lib/runtime_gateway_core_tests 编译检查、Provider 数据层 27 项测试、Host manager_gateway 29 项测试、前端 usageQuery/codepetGateway 4 项测试、`npm run protocol:check`（20 项）和 `npm run build`，均通过。扩展 Host lib 测试初次 57/58 通过，日志轮转测试在 Windows 遇到一次文件访问拒绝；单独复测通过，该模块本次没有改动。项目未定义 npm test，前端测试使用 `npx vitest run`。

Codex Provider lib 67 项测试通过，覆盖合并后的 Provider 行为。
