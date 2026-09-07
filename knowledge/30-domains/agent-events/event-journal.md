# 事件页：原始 Hook 与 Provider 日志

## 背景

事件页原来从 `recent_events` / `pet-event` 读取 `PetEvent`，只显示最近五条标题与消息。它既不能保留 Hook 原始对象，也不能检查独立 Provider 进程发出的协议事件。用户希望在同一个独立日志中检查这两种数据，支持历史分页、轮转、实时刷新和关键词过滤。

## 目标

- Hook 条目的 `payload` 保留接收到的原始 JSON 对象；不经过 `PetEvent` 归一化或前端裁剪。
- Provider 条目记录 Host 已解码的 `ProtocolEvent`，包含协议方法、参数和原有路由字段。
- 文件是历史数据源，UI 通过 Tauri 接口访问；实时查询读取内存增量。
- 关键词覆盖完整 JSON 和外层来源信息，支持 Hook / Provider 来源筛选。

## 非目标

不记录网络密文、二进制帧或 RPC 请求/响应，不改变桌宠活动归并与 Remote 业务路由，也不把事件页诊断日志作为业务 replay。日志是本地诊断能力，不是保证每条写入的审计系统。

## 现状理解与证据

- `src-tauri/src/activity/collector.rs` 负责旧 `/hook` 和 spool 回放，先拿到 `IncomingHook.payload`，再归一化。
- `crates/providers/codepet-observation/src/lib.rs` 是 Provider 内的独立接收入口，接收 Hook / OpenCode observation 的 JSON，再通过 `EventNotification` 上报。原实现会向对象插入 `codepet_gap`。
- `crates/codepet-host/src/providers/process.rs` 为业务处理解码 Provider wire event；`providers/manager.rs` 检查当前进程代际后调用 `accept_provider_event`，之后才进入订阅者和 Gateway 分发。
- `src-tauri/src/app/log.rs` 的 `code-pet.log` 是应用运行日志；本功能不把原始事件混入其中。

## 实现路径

### 采集与数据边界

旧 collector 在归一化与来源过滤之前提交原始 payload；spool 回放也记录，但仍按旧规则决定是否进入桌宠。

Provider observation 使用 `payload.codepet_observation = { raw, gap }` 外层封装，内部缺口标记与原始对象隔离。Host 从 `raw` 生成 Hook 日志，再生成完整 Provider 日志。两个条目分别表示原始输入与协议输出。桌宠 projection 读取该封装，同时兼容旧的扁平 notification。原生对象中的任意 `codepet_gap` 等键均可保留。

Host 在 `accept_provider_event` 入口记录收到的协议事件，发生于订阅者分发和 Gateway 映射之前，因此不依赖 Remote 连接数量，也不记录 replay 重发。记录代表“Host 收到”，并不代表后续路由校验或投递成功。没有为日志再解码二进制；仍需要把类型对象转换为 JSON，该工作交给写入线程。

### 持久化与轮转

`EventJournal` 使用独立写入线程、容量 256 的队列。业务侧只提交对象，队列满时计数并返回；不等待磁盘锁。写入文件句柄复用，轮转前关闭，避免 Windows 下打开句柄影响重命名。

路径为应用数据目录下 `logs/events.jsonl`，每行结构为 `{ sequence, receivedAt, source, provider, payload }`。`receivedAt` 是 Host 入队时间（毫秒），原始源时间仍在 payload 内。文件达到 5 MiB 或 UTC 日期变化时轮转为 `events.<UUID>.jsonl`，保留最近五份归档。UUID 是稳定分片 ID，不随着轮转次数改变；旧 `.1` 至 `.5` 文件在启动时迁移为稳定文件名。单条超大记录完整写入，可超过分片大小；分片阈值不是记录截断阈值。

每份日志配套一个 `.idx` 字节偏移索引。索引包含 32 字节版本/分片 ID 头，每条有效事件对应 32 字节记录：序号、起始位置、结束位置及校验值。校验值复用已有 ring 的 SHA-256 前 8 字节，用于发现意外损坏。写入日志成功后更新内存索引，磁盘索引缓冲每 64 条 flush，轮转前也 flush。

启动时加载索引恢复序号；索引落后时只扫描未索引的尾部，索引损坏或缺失时从该 JSONL 重建。索引是可丢弃的加速数据，JSONL 才是事实来源；索引写入失败不把已写成功的事件计为丢失。残缺 JSON 行跳过，非换行结尾先隔离再追加。写入成功后才更新内存序号和实时缓冲。缓冲最多 1000 条，并以约 8 MiB 的序列化内容量限制保留大小；Rust 对象内存开销不计入该估算。未使用逐条 `fsync`，进程异常退出或掉电仍可能丢失队列和操作系统尚未落盘的数据。

### 接口与实时刷新

`query_event_journal({ query })` 接口支持：

- `keyword`：不区分大小写的子串匹配，不执行正则。
- `source`：`hook` / `provider`，空值表示全部。
- `limit`：默认 100，上限 200。
- `historyCursor`：`{ segmentId, beforeOffset }`，从稳定分片和字节边界继续历史分页。
- `before`：兼容按序号定位，使用内存偏移索引二分查找，不扫描较新的记录。历史按新到旧返回。
- `after`：严格大于该序号的实时增量，按旧到新返回。

历史查询在短暂持锁期间定位分片、固定索引快照并打开文件，然后释放写入锁。归档索引以 Arc 共享；活动索引在查询快照存在时通过 copy-on-write 保持一致。查询仅从游标所在分片及更早分片取数据，按偏移倒序读取，每次缓存一个 64 KiB 块；超大单条记录单独完整读取，不截断 UTF-8。每页最多返回 `limit` 条，再寻找一个匹配记录判断是否还有下一页。

返回的 `nextHistoryCursor` 指向该条预读匹配记录的结束字节；下一页只会重新解析这一条预读记录，不会重新解析之前已扫描的不匹配记录。相邻页的 64 KiB 预读块可能存在少量物理读取重叠，但不会从文件头重新扫描。`nextBefore` 保留为兼容字段。

查询句柄以 lease 固定分片，即使轮转超过保留数量，也在当前查询关闭文件后再清理其数据和索引。后续请求引用已经清理的分片时，返回 `resetRequired=true` 与 `resetReason=history_expired`，UI 提示后加载当前保留日志。客户端只能引用后端已知的分片 ID，不能传文件路径；非记录边界游标会被拒绝。

实时仍只读内存环形缓冲，不扫文件。返回 `cursor`，即使关键词没有命中也推进实时游标；批次超过上限时推进到已交付位置。游标超出缓冲或因重启失效时返回 `resetReason=live_expired`，UI 显示提示并重新获取最新历史。实时追加保留已有历史游标；列表超过 1000 条并裁掉旧记录后，使用最旧保留序号经偏移索引重新定位。

### 文件读取与平台取舍

按用户要求不引入 mmap，也不增加 mmap 依赖。Windows 支持文件映射，但映射大小、共享和生命周期约束与 Unix 系统并非完全一致，见 [Windows 文件映射文档](https://learn.microsoft.com/en-us/windows/win32/memory/creating-a-file-mapping-object) 和 [memmap2 安全说明](https://docs.rs/memmap2/latest/memmap2/struct.MmapOptions.html#safety)。当前所有平台统一使用标准库 `File + Seek + Read`，收益来自定位索引、减少解析量以及缩短锁占用，无需平台专用读取实现。

写入失败通过 `error` 提示；队列满、序列化或写入失败累计在 `dropped` 中。计数与错误是本次进程状态，不持久化。文件完全不可读、初始化失败则接口直接返回错误。

### UI 状态

事件页打开时加载历史，每秒在页面可见且未暂停时请求内存增量。离开事件页卸载组件并清理定时器。搜索防抖 300 ms，用请求代际防止旧搜索结果覆盖新结果；关键词和来源变化重新加载。支持原始 JSON 展开、来源与方法、接收时间展示；仅在展开时格式化和渲染 JSON。历史翻页自动暂停刷新，防止正在阅读的列表移动；恢复实时后列表最多保留最新 1000 条。搜索框节点保持不变以保留输入焦点。

## 涉及模块

- `codepet-host/event_journal`：统一写入、搜索及内存实时游标。
- `codepet-host/event_journal/history.rs`：稳定分片、偏移索引、查询快照、分块读取与延迟清理。
- Host manager：在解码后的共享分发入口采集，避免 Remote 扇出重复。
- Provider observation 与 Pet projection：保留原始对象，同时维持活动状态解析。
- Tauri collector / app / bridge：连接旧 Hook、共享日志实例与前端命令。
- `frontend/lib/EventJournal.svelte` / `App.svelte`：用独立日志视图替换旧事件展示。

## 风险

- 流量突增或磁盘失败可能使队列满：非阻塞提交测试覆盖，UI 显示损失数。历史扫描已释放写入锁，但仍可能竞争磁盘带宽，需观察损失计数。
- Hook 封装影响活动识别：原始对象保真测试和 projection 回归验证；Provider 与 Host 应配套升级。
- 轮转、重启、损坏索引及分页可能漏记录：稳定 ID、过期游标、退休分片持有、索引重建、损坏尾行和大 UTF-8 记录测试覆盖。
- 索引/数据轮转中断：中断的重命名不能使新活动文件继承归档 ID，有故障恢复测试。查询期间外部修改 JSONL 不受支持。
- UI 搜索竞态、焦点和窄窗口：浏览器模拟接口检查旧响应、键盘展开、搜索焦点、980/820 像素布局和离页停止轮询。

## 测试计划与结果

2026-09-07 至 2026-09-08：

- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --lib`：最终 46 项通过，含新封装 projection 回归和 4 项日志测试，覆盖实际异步写入、原始数据、无 Remote、队列满、分页、日期轮转、残缺尾行与重启。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-observation --lib -- --test-threads=1`：6 项通过。首轮并行运行时已有订阅锁释放测试失败，串行复跑通过，未改其锁行为或放宽断言。
- `npm run build`：通过。
- `npx vitest run frontend/lib/api.test.ts frontend/lib/eventFeed.test.ts frontend/lib/activity.test.ts`：61 项通过。
- `TAURI_CONFIG='{"app":{"macOSPrivateApi":false},"bundle":{"resources":[]}}' cargo check --manifest-path src-tauri/Cargo.toml --lib`：通过。直接检查受到现有 macOS allowlist 与未 staging 的 provider-sdk 资源阻碍；环境覆盖仅用于编译验证，不等同于完整打包验证。
- Playwright 使用模拟 Tauri 接口验证历史分页、来源/关键词、JSON 展开、输入焦点、过时响应、实时增量、键盘操作、980/820 像素布局和卸载清理：通过。

2026-09-08 偏移读取升级：

- Host lib 最终 53 项测试通过，其中日志模块 11 项，覆盖物理游标跨追加/轮转/重启、过期接口返回、查询持有期间清理、索引落后/损坏/缺失、旧文件迁移以及大 UTF-8 行。
- 5000 条记录夹具中，首次、继续和跳转中间序号取 10 条均只解析 11 条、读取 64 KiB。稀疏关键词两次分页分别扫描 1001 条，验证前页过滤掉的 999 条不会重新扫描；这是读取量与解析量验证，不是墙钟性能倍数。
- Tauri lib 编译检查通过，仍使用上文所述仅用于检查的环境覆盖。
- 前端生产构建通过；浏览器模拟接口验证物理游标续页、实时更新后保留历史位置、历史读取失败重试及过期提示：通过。

## 知识沉淀

本文件保存事件页诊断语义与接口；`event-pipeline.md` 中的桌宠事件管线继续描述业务状态。诊断文件不得反向驱动桌宠或替代 Provider/Gateway replay。

## 未知项

尚未使用真实 Agent、真实 Tauri 窗口跑完整端到端链路；Windows 运行时轮转与高频 Provider 输出压测待补。单例路径在首次打开时确定，修改数据目录后需重启应用以切换事件文件。
