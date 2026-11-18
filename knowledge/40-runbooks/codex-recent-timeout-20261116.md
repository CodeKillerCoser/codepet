# Codex 最近对话超时：2026-11-16 日志排查

## 当前结论（2026-11-17 实时栈与原生对照实验）

已定位的主链路是：Host 为首屏 recent 枚举全部活动/未读 ID → Codex Provider 对候选串行调用 `thread/read(includeTurns=false)` → 对历史索引中已无法读取的 ID，Codex 原生查找历史文件目录 → 大量重复目录查找的累计耗时超过手机 30 秒期限。Host 的快照重试和逐条数据库接纳进一步放大开销。不是某个连接完全不可用，也不能将问题仅归为 Host 的 1295ms 数据库处理。具体证据见文末实时栈与复现章节；后续已完成第一阶段运行时修改，范围与验证见 [有界补读修复](codex-recent-bounded-metadata-repair.md)。下文保留各排查阶段的证据与当时未知项。

用户随后要求核对读取必要性：[首屏查询审计](recent-first-page-query-audit.md) 确认 recent 的 IDs 分支跳过了原生 list。历史约束要求 DB 筛候选、正式对象来自 App Server；直接把 DB 列拼成正式摘要并非默认修复。第一阶段保留全局排序，先约束串行成本、重叠/重试读取和 Host 事务放大；真正按排序键只补当前页对象仍需后续设计。

## 现象与证据

用户报告约 18 点手机端 Codex 最近对话加载失败，错误为 `conversation.recent timeout`。桌面包 `codepet-logs-2026-11-16-to-184432-CST.zip` 的 manifest 标记版本 `0.3.11-beta+b45f6fc`，截止 18:44:32；手机包 `codepet-logs-2026-11-16T15-01-42-240560Z.zip` 实际仅保留北京时间 22:01:23—23:01:42。两包不覆盖同一次超时，不能直接拼成完整调用链。

桌面 17:43:17 三个 Provider 初始化成功，Codex 17:43:22 ready，声明 active/unread/list IDs 与日期查询能力。因此启动成功或支持能力不等于最近查询实际完成。包内没有这次 recent 的请求响应或分段耗时。Codex 的模型 Responses WebSocket 报错也不能据此归因于目录读取。

手机包保留了同一台桌面设备 `device-8819250a-4a46-476a-bf05-2f13187ed893` 的另一次超时：

| 北京时间 | 证据 |
| --- | --- |
| 22:43:32.629 | `codepet.2.log:1280`，IPv6 直连，recent 请求哈希 `355268ef85c8f2b995736c8a` 入队 |
| 22:43:36.096—36.152 | 同文件 1283—1284，同一连接 ping 约 56ms 返回；recent 仍在途 |
| 22:44:02.632 | 同文件 1311，recent 到达 30 秒超时；1312 的 client span 为 30,010,846µs |
| 22:47:55.718 | `codepet.1.log:152`，同一请求收到 230 字节错误响应，耗时约 263.089 秒；153 记录响应被丢弃 |

这次超时不是连接完全断开：同连接心跳继续返回，recent 的错误响应也最终抵达。但没有请求发送完成/Host 接收时间，不能完全排除数据通道单条消息排队，也没有错误码来确定服务端最终失败阶段。

另一台设备的 relay recent 多次成功（约 6.37—18.38 秒），23:00:44 的超时则与 ICE failed、channel.abort 同时出现，且计时器延迟至约 65.60 秒触发。必须与前述 IPv6 请求分开分析，不能用它证明同一业务根因。

## 源码定位与待证实项

- `codepet-host/src/gateway/recent.rs`：同一个 view 的构建锁从聚合开始持有至结束；先完整枚举 active/unread，再每 100 个 ID 补读摘要，前后核对事实，变化时最多做三轮。首屏 20 条不代表上游只读取 20 条。构建锁等待、全局容量等待和整次聚合缺少统一总预算。
- `codepet-provider-codex/src/provider/directory_list.rs::read_db_page`：DB 候选的原生摘要补读逐条 await `thread_read_metadata`。大量候选或个别慢读会放大首屏成本。这是确定的串行路径，尚未证明是本次 18 点超时的实际慢点。
- Remote `webrtc_gateway_transport.dart`：普通请求 30 秒截止；超时移除本地 pending，晚到响应直接丢弃。Host `remote/channels/session.rs` 在连接停止时取消请求，未因为客户端单条本地超时而取消 recent。因此超时后旧构建仍可能占锁，影响相同 view 的后续查询。
- 现有包缺少 recent 各阶段耗时、活动/未读数量、IDs 批次数和具体下游错误。只能确认另一条 recent 长时间没有完成，不能把等待锁、Provider RPC、原生 read 或共享阅读状态锁中的任何一个定为最终根因。

## 下一步验证

先在故障设备同一时段关联 recent 的 requestId、Host 接收/完成时间及错误码，再记录构建锁/容量等待、active、unread、IDs 补读、日期页、事实重验和快照提交耗时。Provider 记录每批原生 read 数量与耗时，不记录消息正文、令牌或密钥。

以大量活动/未读、慢原生 metadata、持续事件失效和超时后立即重试分别复现；断言首屏预算、旧构建回收、其他 Provider 与 ping 可独立响应，以及分页完整性。不能以延长手机 timeout、截断活动/未读或返回伪成功解决。

## 本轮验证范围

只解包并分析用户提供的两份日志、核对当前源码和既有排查记录；原生 UI 确认本机三个通知来源在线后已恢复原页面。未修改运行时代码、重启 Provider、安装 App 或推送提交，未执行编译测试。本机运行状态不能替代用户故障设备的同时段请求证据。后续补齐证据后更新本记录的根因与修复结论。

## 继续排查：分层实验与边界

用户确认故障电脑名为“汪鑫的MacMini”。当前可操作电脑的 SQLite `host_identity` 为 `device-a8b289e3…`、显示名“wangxin的Mac Mini”；故障请求为 `device-8819250a…`，不是同一个运行实例，不能将当前机器的正常结果当成故障机器的排除证据。旧 JSON identity 文件不再是当前身份权威。

使用独立进程与 SQLite backup 快照，临时加入 Host 阶段计时，并用原生 Codex stdio 包装器仅记录方法、ID、耗时、响应大小和错误码。所有 hooks/config/auth 副本位于权限 0700 的临时目录；未对真实任务发送 prompt/resume/stop。独立诊断 Provider 的 shutdown 不作用于正在运行的 App。结束后已还原临时源码计时改动，未替换 App。

### 实测

| 实验 | 结果与可支持的结论 |
| --- | --- |
| 当前机器数据快照，直接调用已安装 Provider | 普通列表 11ms、日期列表 8ms、20 个 IDs 摘要 16ms；原生成功 `thread/read` 约 0.6–1.1ms。只证明这份快照读取正常 |
| 相同快照经过实际 Host、已安装 Provider 与原生 Codex | 首屏 137ms，缓存读取 0ms；分发阶段均正常 |
| 完整诊断包装器自身验收，复制当前配对读者身份 | 首屏 217ms（两轮收集），449 次原生 metadata 请求最大约 1.36ms，原生 `thread/list` 最大约 1.55ms；同样没有复现原始超时 |
| 在隔离 Hook endpoint 每 15ms 发布活动事件 | 两个 recent 请求均经历 attempt 0/1/2，约 252ms、242ms 后返回 `recent_snapshot_changed`；Provider/原生请求均正常完成。证明持续失效可使 Host 三轮构建全部失败，不证明用户当时有相同频率的活动 |
| 同一共享存储、200 条相同摘要、约 5000 条背景记录，无修改的重复读取 | 模拟 Host 当前逐条调用耗时 1765ms，直接使用底层批量 API 耗时 9ms，约 196 倍差异。确认 Host facade 丢失批量边界，不能把这个合成规模的耗时冒充故障设备耗时 |

`ConversationStateStore::observe_and_decorate_summaries` 与 `observe_page_summaries` 把一批 rows 拆成 `std::slice::from_mut(row)`。底层 `SharedConversationStateStore::lock` 每次打开 SQLite、`BEGIN IMMEDIATE`、读取整个阅读状态并复制快照。逐条调用因此重复完整读取和锁事务；这是源码与对照实验共同确认的 Host 性能缺陷，原生线程读取无需参与就能复现。

手机同一长请求的失效事件为北京时间 22:43:32.705、22:45:00.021、22:46:27.954，间隔 87.316s、87.933s；错误响应在 22:47:55.718，再相隔 87.764s。Host 的 `recent_snapshot_changed` 两个静态错误变体，按实际 requestId 序列化后均为 230 字节，与晚到错误响应大小一致。这与三轮重收集高度吻合，但响应长度不能替代实际错误码，更不能据此证明每轮 87 秒全部花在数据库里。

### 可直接运行的故障机诊断

已产出 `~/Documents/CodePet-builds/recent-diagnostic-20261116.zip`，在故障 Mac（Apple Silicon）解压后执行 `python3 run.py`。驱动自动复制该机器的 App/Codex 数据、采用最近活跃的已配对读者 scope，并调用实际安装 Provider；可用 `--reader` 指定出错手机。输出到下载目录的报告只含 `host.log` 和 `native.jsonl`，不打包配置、认证、数据库或消息正文。包装器已在当前电脑完整运行通过，检查了原生导出键集合，诊断进程退出，无相关残留进程。

工具按阶段区分构建锁、active/unread、摘要补读、Host 数据库接纳、日期页以及原生逐条 RPC。它不重放实时 Hook，因此失败设备上静态快照读取正常仍不能排除活动期间的一致性失效。故障机分段结果到手之前，结论应为“Host 的批量退化和快照饥饿机制已复现；原请求每轮约 87 秒的具体慢点尚未取证”，不可宣布已排除故障机上的 Provider/原生慢读。

## 故障机诊断结果：23:43:29 报告

用户提供 `codepet-recent-diagnostic-20261116-234329.zip`，包含 55 行 Host 阶段记录与 306463 字节原生计时。新启动的诊断进程成功完成首屏，未复现旧请求超时。

| 阶段 | 实测 |
| --- | --- |
| recent 总耗时 | 首屏 2519ms，缓存读取 0ms；返回 20 条、仍有下一页 |
| 构建锁 | 0ms |
| active 枚举 / 重验 | 4ms / 4ms |
| unread 枚举 / 重验 | 72ms / 75ms |
| IDs 摘要补读 | 16 批，合计 1046ms，得到 228 条摘要 |
| Host 摘要数据库接纳 | 1295ms，约占整次请求 51.4% |
| 日期尾页 | 0ms，本页已有足够的优先条目 |
| 原生 `thread/read` | 1506 次全部完成；p50 0.209ms、p95 1.1ms、p99 1.815ms，最大 6.838ms |

原生补读 228 次成功，1278 次返回 -32600；Provider 成功返回意味着这些错误被当前 `is_thread_not_loaded` 分支识别并跳过。报告没有错误正文，不能把所有落空 ID 都认定为用户删除，也可能是旧索引、换 home 或其他不可读取资源。落空调用约占 84.9%，证明 recent 在遍历大量无法得到摘要的历史注意力索引。需要验证存在性/失效回收策略，不能直接截断 unread 或把暂时错误当作删除。

原生 metadata 累计执行约 544.456ms，整段原生摘要读取时间窗口约 1.022s，没有悬挂 RPC。启动时 `model/list` 约 4.877s，发生于 ready 前，不能计入已记录的 recent 2.519s。

这份报告确认当前静态快照最大的单项开销是 Host 数据库接纳，而非单条 Codex 原生 metadata 读取。它没有运行原 App 的历史进程、实时 Hook 或竞争负载，不能据此将原请求约 263 秒全部归因到 1295ms 的操作，也不能完全排除原进程状态下的慢读。

### 下一步：采运行中实例

新增 `scripts/diagnose_recent_live.py`，在原 App 保持运行时，用系统 `/usr/bin/sample` 并行采 Host、其直接 Codex Provider 子进程及原生后代的函数栈；默认 45 秒，用户在窗口内从手机刷新 recent。只读取进程树和线程栈，不读取数据库、认证文件或消息内容，不发 RPC、不重启进程。该方式能够观察静态快照诊断没有包含的锁竞争和实际原生进程状态；若未在采样窗口触发慢请求，仍不能推断根因。

已验证进程树筛选排除独立 Codex App、其他 Provider 和不属于该 Host 的同名进程；本机 2 秒验收得到 Host/Provider/CodexNative 共 6 份采样，0 失败，Host 栈可识别 `codepet_host` 符号。`git diff --check` 通过。未修改业务运行时或替换 App，尚未取得故障电脑原进程发生超时期间的栈。

### 2026-11-17 采样工具进程识别修正

用户执行初版脚本后报 `The running /Applications/Code Pet.app Host was not found`。初版按完整默认安装路径识别 Host，本机验收也只覆盖默认位置；这是已确认的工具缺陷。该报错本身不能区分目标电脑安装在其他位置与 App 没有运行，也与 recent 接口是否畅通无关。

新版使用 `ps -ww` 获取不截断的可执行文件名，以名称识别 Host，不依赖 app 包名或安装目录；Provider 按实际祖先进程归属查找，允许中间 wrapper。选定 Host 后固定 PID，不在采样中自动切到另一实例；多 Host 要求 `--host-pid` 显式选择，`--list` 仅输出候选进程供诊断。自定义 Host 二进制名称也可通过 PID 指定，仍须找到属于该进程树的 Codex Provider 才开始采样。

运行 `python3 -m unittest discover -s scripts -p diagnose_recent_live_test.py -v`：7 项通过，覆盖 Downloads 安装、App Translocation、重命名 app、仅文件名、wrapper、多个 Host、孤儿 Provider、独立 Codex App、进程环和只读候选列表。新版在本机运行 2 秒再次得到三个角色的 6 份采样、0 失败。新交付包为 `~/Documents/CodePet-builds/recent-live-diagnostic-v2-20261117.zip`。未在故障电脑运行新版，原始超时根因状态不因此改变。

## 故障机实时栈：00:15:43 报告与根因复现

用户提供 `codepet-recent-live-20261117-001543.zip`，采样窗口为北京时间 00:14:58—00:15:43，共 45.691 秒。Host PID 74911、Codex Provider PID 74922、原生 Codex PID 75359，各 32 份，共 96 份采样，0 失败。Host 栈标记版本为 `0.3.11-beta+b45f6fc`。

第 000—010 轮没有 metadata 补读等待；从第 011 轮开始，连续至第 031 轮：

- 21 份 Provider 栈均出现 `directory_list::read_db_page → CodexAppServerSession::thread_read_metadata → request_value_with_timeout_outcome → recv_timeout`，且可见多条补读工作线程同时等待。
- 对应 21 份原生栈均出现目录读取，包括 `tokio::fs::read_dir::ReadDir::next_chunk → readdir_r → __getdirentries64`，并有 `read_dir/open` 系统调用。不是仅在 Binary Images 中找到 SQLite 库名；统计仅使用 Call graph 部分。
- Host 仅第 011 轮短暂采到 recent 的阅读状态初始化/SQLite 等待，未出现同样持续 21 轮的 recent 数据库执行栈。异步 await 不一定留在 OS 线程栈中，不能据此宣布 Host 没有在途请求；但 Provider 的明确 RPC 等待与原生的目录 I/O 同时成立。

Provider 的 `read_db_page` 对请求的每个 ID 都准备候选，包括 Codex DB 中未找到的 ID；随后逐条调用原生 `thread/read`。只有返回严格匹配的 `-32600 / thread not loaded: <id>` 时才跳过。上一份故障机快照诊断已记录 1506 次原生读取，其中 1278 次走此落空分支；因此大量旧索引成员仍会重复进入原生查找。

### 原生最小对照

对本机安装的 Codex 原生 App Server 直接发送相同的不存在 UUID 的 `thread/read(includeTurns=false)`，不经 Host 或 Provider；分别使用空临时 home 和含 5040 个合成 rollout 文件名、336 个日目录的临时 home。仅创建空文件，不复制任何真实对话；两组都连续调用 8 次，均返回同一 `thread not loaded` 错误。

| 历史目录 | 8 次调用的中位耗时 | 说明 |
| --- | --- | --- |
| 无历史目录 | 0.1795ms | 立即结束查找 |
| 5040 个历史文件名 | 36.85ms | 每次约 35—38ms，重复请求没有消除扫描成本 |

仅改变历史目录树，落空读取成本增加约 205 倍。1278 次落空读取按这个合成规模估算就约 47 秒，已超过手机期限；这只是放大量级验证，不是对故障机器每次 RPC 的实测。故障机与本机 Codex 二进制 UUID 不同，不能把本机耗时当作故障机精确耗时；实际原生目录读取由故障机自身采样直接证明。

### 对前一探针的修正

原快照探针复制 SQLite 与配置，没有复制 `sessions` / `archived_sessions` 目录。这使不存在 ID 的文件查找很快结束，1506 次 metadata 在约 1 秒内完成。因此那份 2.519 秒的静态结果低估了真实历史文件查找，不能用来排除原进程的目录 I/O。之前关于“尚未排除原生慢读”的限制应保留，后续诊断必须明确是否保留原生文件布局。

### 修复边界与验证建议

1. **Provider 候选补读**：对 DB 未收录的 IDs 按批次或查询共享一次有界的历史路径查找，避免每个 ID 各扫一遍。必须保留“刚创建、尚未落库”和仅存在于 rollout 的合法会话，不能简单把 DB 缺席当作删除；I/O 失败要显式报错。
2. **Host 查询与存储**：恢复按 Provider 批量接纳摘要，避免逐条重读完整阅读状态；为整次聚合与重试设置总预算，避免客户端超时后继续做多轮昂贵工作。不得通过截断全部 unread/active 或伪造成功的空列表规避完整性要求。
3. **索引治理**：核对已无法解析的活动/未读成员如何回收或缓存不存在结论，区分临时不可用、切 home、归档与真实删除，不能无证据清空用户阅读状态。

验证应覆盖上千个落空 ID、真实布局的历史目录、尚未落库的新会话、仅文件存在、权限失败、并发 recent 与持续 Hook 更新；同时检查原生查找次数、首屏整体期限、分页完整性和其他 Provider/心跳响应。业务代码尚未修复，不宣称已消除超时。

实验源与结果保存在 `~/Documents/CodePet-builds/recent-root-cause-20261117/`，包括 `native-missing-probe.py`、原生对照日志和逐轮栈统计。所有合成目录由 TemporaryDirectory 清理；未向用户会话发送提示词或执行控制指令，未替换 App 或推送提交。

### 2026-11-19 的 Codex 单 Provider 超时

用户在手机上看到 `conversation.recent timeout`。e90cc8e 桌面日志记录了局域网连接建立，未记录这次 recent 的收发边界。模型目录的请求超时属于另一项操作。原故障请求耗时、候选规模和阻塞阶段均未知。

复查发现三处重复工作。未读续页曾为内存中的同一快照重新打开 SQLite、读取所有会话与读者。现在首轮按 Provider 与读者直接查询未读，续页直接读取快照。读者首次登记仍原子捕获基线；Host 在阻塞线程上登记，读库期间释放路由表的互斥锁。活动与未读首轮枚举可以并行，发布快照前继续校验成员。Codex 两个目录来源判断本页时间边界时只需看到本页最后一条；需要原生回退核实排序时，同一轮已确认落空的 ID 也可复用，下一次查询重新求证。

`recent.rpc.received` 表示请求到达桌面并准备入队，`started` 给出队列等待时间，`completed` 记录条数、业务错误码、总耗时以及回复是否进入发送队列。按相同连接与请求 ID 匹配手机的发送、超时或收到事件。进入发送队列之后，仍需手机端的收包记录才能确认送达。

Host 合成 1000 个活动候选的集成测试首屏 20 条用时 157 ms。Codex 子进程用 128 个缺失和 3 个可解析候选触发排序回退，131 次原生读取后返回正确第一条。另用持有 SQLite 写事务的第二个连接确认已注册读者能立即读取已提交状态。运行 Host、Provider、共享状态库的单元及纵向测试；这些数据来自独立测试环境，手机与故障机的真实查询仍需同次请求核对。

#### 真实 App Server 压力回放

用隔离的 Host、内置 Codex Provider 和本机 Codex App Server 构造 24 个有 rollout 的会话、1278 个缺失的未读候选，以及分布在 336 个历史子目录中的 5040 个占位文件。整个环境单独初始化，没有复制用户会话或认证数据。此前的合成 Provider 首屏耗时无法覆盖这种原生目录查找。

相同规模下，原 Host 连续两次在 10072 ms 和 10060 ms 返回 `provider_request_timeout`，错误阶段是 `conversation.list / stream`。Host 与 Provider 的传输层使用普通 RPC 的 10 秒期限，早于 Codex 列表自身的 20 秒预算。带读者 scope 和查询条件的列表现在使用 22 秒的传输预算，整次 recent 仍受 Host 的 25 秒和手机的 30 秒约束。再次回放，冷首屏在 10892 ms 返回 20 条并提供续页游标，同一读者再次请求命中快照，耗时低于 1 ms。

`process_rpc::scoped_summary_has_its_own_deadline_on_the_provider_stream` 用同一个慢 Provider 同时发普通列表和 scoped 列表，验证前者按普通期限退出，后者可等到有效结果。此实验复现了一个真实可达的超时原因。原手机请求的服务端阶段仍须从新版的同次收发日志确认。
