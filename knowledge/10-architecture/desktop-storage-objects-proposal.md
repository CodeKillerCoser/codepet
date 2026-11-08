# 桌面业务状态的 SQLite 存储

## 背景

2026-11-08 用户确认并授权实施：设置保留 JSON，会话和 Provider 业务记录使用 SQLite；数据库中的整块 JSON 拆成字段和关联表。不迁移、不兼容旧数据，后续重新连接。此前已确认 conversations 的 14 列及有界哈希规则。

## 目标

- 设置只有 JSON 一个权威来源，业务状态只有 SQLite 一个权威来源。
- 会话活动、各客户端已读进度、Provider 身份、运行时选择、配对和用量可以独立查询、事务更新。
- 消除 JSON 整行落库，保留已有协议和用量的未知值语义。

## 非目标

手机暂不引入 SQLite；任务抽取、任务谱系及其存储不改。完整聊天历史仍由 Harness 提供。存储改造自身不包含队列；后续已实现 Gateway Steer、队列关联表、派发器及两端 UI，详见[消息排队与 Steer](../20-product/conversation-input-queue-and-steer.md)。设置仍沿用现有读写服务，本次未增加外部编辑热加载、冲突合并或新配置格式。

## 现状理解

实现证据：`codepet-provider-data/src/app_state.sql` 定义共享状态，`usage.sql` 定义用量；`conversation_state.rs`、`usage_snapshot.rs` 映射业务对象与列；Host 的 `device.rs`、`providers/instance_registry.rs`、`remote/access.rs`、`remote/channels/webrtc/cloud/registration.rs` 使用同一个共享库。Tauri 的 `runtime_gateway/tauri_bridge.rs` 注入生产路径。

原会话 JSON 文档是活动/已读状态，并非完整聊天记录。原用量数组、事件 payload 和分页 result 已展开；内存与协议层仍可使用 JSON，不把协议编码方式当作落盘方式。

## 实现路径

### 物理布局与初始化

| 路径 | 权威内容 |
| --- | --- |
| 默认系统数据根 `config/settings.json` | 有类型的用户配置；包含自定义业务数据目录引导 |
| 实际数据根 `config/pet-sources.json` | 已有来源偏好，保持原行为 |
| 实际数据根 `app-state.sqlite` | Provider 主表/实例、Host 身份/连接、会话活动和阅读进度 |
| 实际数据根 `providers/<pluginId>/usage.sqlite` | 该插件的用量、采集游标、待处理计量和冻结查询快照 |

两个业务领域不等于只有两张物理表：主表是 providers 和 conversations，一对多关系与重复记录使用关联表。身份、会话外键同库，插件用量保持既有观察来源隔离，避免把多个插件的 observation 记录混合。

Host 使用 `CODEPET_APP_STATE_DATABASE` 注入共享库；持久会话参与方还收到同路径的 `CODEPET_CONVERSATION_STATE_DATABASE`。独立 ProviderData 没有前者时，使用分配的数据目录下的 app-state.sqlite；没有共享会话路径时不宣称持久未读能力。ProviderDirectories.databasePath 指向 usage.sqlite，接口形状不变。

SQLite 使用 WAL、5 秒 busy timeout、外键和 FULL 同步；Unix 新建文件权限 0600，拒绝数据库路径为符号链接或非普通文件。共享库以 user_version=1 初始化，未知版本拒绝打开。损坏的新库报错，不自动删除或重置整库。旧 JSON、provider.sqlite、conversation-state.sqlite 不导入、不回退读取、不自动删除；新库首次生成身份，需要重新配对。

### 设置为什么继续用 JSON

设置规模小、按用户操作保存，JSON 可直接检查和备份，且 Rust/Node 路径引导已有同一份解析契约。保留现有 AppSettings；不建设置表或第二份数据库副本。TOML/YAML 暂无足以抵消格式、默认值和回写改造成本的需求。手工修改后的生效时机仍遵循现有设置行为，数据目录变更需要重启。细节见 [设置持久化](settings-persistence.md)。

### 会话主表 conversations

每个 Provider 实例下的原生会话一条；标题是缓存，活动版本用于未读判断。三个 queue 字段已建表保留，当前业务不读写它们，后续队列功能接入时再定义消息与操作表。

| 字段 | 类型、可空与初始值 | 保存什么 | 为什么需要 |
| --- | --- | --- | --- |
| id | INTEGER PRIMARY KEY | 本地分配的会话主键 | 已读记录、输入队列和操作记录可引用稳定、短小的关联键 |
| provider_instance_id | TEXT NOT NULL，FK → provider_instances.id | 该会话所属的 Provider 实例 ID | 将请求路由到正确实例，区分不同实例中相同的原生会话 ID |
| native_conversation_id | TEXT NOT NULL，非空字符串 | Harness 原生会话 ID，原样保存 | 调用 Provider 时识别原生会话；与实例 ID 一起构成业务唯一身份 |
| title | TEXT NULL，初始 NULL | 最近一次从 Provider 观察到的标题缓存 | 断线或尚未加载 Provider 时仍可为本地队列展示会话名称；可重建，不成为原生标题权威 |
| activity_scope_id | TEXT NOT NULL，FK → activity_scopes.id | 该会话使用的活动版本计数域 | 现有各数据库的计数独立；收敛后仍需让活动版本、客户端基线和已读版本在同一计数域内比较 |
| activity_version | INTEGER NOT NULL，≥ 0 | 该会话最近一次有效活动取得的版本号 | 与客户端已读版本比较来判断未读；它是活动序号，不是消息数、时间戳或任意数据库修改版本 |
| summary_fingerprint | TEXT NULL，初始 NULL | 有界短摘要与状态的指纹，见下方计算约束 | 重复刷新相同摘要时不反复制造新活动；NULL 表示尚未建立该观察通道的基线 |
| detail_fingerprint | TEXT NULL，初始 NULL | 最后一条助手消息或待审批项的 item ID 与 turn ID 的 SHA-256，64 位小写十六进制 | 加载相同详情时识别相同的相关项，不扫描或哈希消息正文；NULL 表示暂无此类项的基线 |
| last_activity_event_key | TEXT NULL，初始 NULL | 最近观察到的相关事件键，例如 turn-terminal:<turnId>:Completed | 避免相同事件键连续重复推进活动版本；直接保存字符串，不计算哈希，也不承诺跨任意交错历史事件的全量去重 |
| queue_paused | INTEGER NOT NULL DEFAULT 0，限定 0/1 | 是否持久暂停本会话的自动续发 | 停止、失败或发送结果未知时能保持暂停意图；0 仅表示没有持久暂停，派发仍须核对实时运行状态和交互权 |
| queue_revision | INTEGER NOT NULL DEFAULT 0，≥ 0 | 队列可见状态的单调版本号 | 入队、撤回、派发状态改变及暂停/恢复后，多端能识别旧快照/乱序事件并按版本做条件更新 |
| next_queue_sequence | INTEGER NOT NULL DEFAULT 1，≥ 1 | 下一条入队消息将获得的顺序号 | 在事务内为并发入队分配 FIFO 顺序；撤回后不回收序号，队列清空后不重置，也不依赖设备时间排序 |
| created_at | INTEGER NOT NULL，无隐式默认值 | 本地首次登记时间，Unix 毫秒 | 明确这条本地记录的生命周期，不代表原生会话创建时间 |
| updated_at | INTEGER NOT NULL，无隐式默认值 | 本行持久字段最近实际改变的时间，Unix 毫秒 | 用于本地记录维护与排查；相同快照重复读取不更新，不用于最近会话业务排序或判断未读 |

主键为 id，业务唯一键为 `(provider_instance_id,native_conversation_id)`。实例与活动计数域的外键限制直接删除被引用对象。当前统一使用 host 活动计数域；reader_baselines 按 `(scope_id,reader_id)` 保存首次接入版本，conversation_reads 按 `(conversation_id,reader_id)` 保存已读版本和更新时间。

一次活动变更在 BEGIN IMMEDIATE 事务内重读状态、推进 activity_scopes.last_version、更新相关会话，再提交；不同进程使用同一事务入口。只写实际变化的会话和已读记录，活动更新不会覆盖 queue 字段。首次摘要/详情仅建基线，首次活动事件推进版本；已读不会倒退，读者互不影响。created_at 登记后不变，updated_at 只随本行实际变化更新，不用于最近会话排序。

哈希约束：summary_fingerprint 使用 preview 最前面至多 512 个 UTF-8 字节的完整字符前缀，截取后 trim，再与状态计算 SHA-256，结果带 `s1:`。不先遍历或复制完整正文。detail_fingerprint 只处理 item ID 和 turn ID，任一 ID 超过 512 字节则不生成详情指纹。last_activity_event_key 直接保存事件键，不计算哈希。不哈希聊天正文、完整历史、工具输出或完整排队请求。有界指纹不能检测前缀之外、同一 ID 内的所有正文变化，仍依赖 Provider 活动事件。

### Provider 主表 providers

| 字段 | 内容与作用 |
| --- | --- |
| id TEXT PK | 稳定 pluginId，供实例关联 |
| display_name TEXT | 从 catalog 缓存的插件名称 |
| selected_executable_path TEXT NULL | 每插件上次选择的 Harness 路径，NULL 表示无持久选择 |
| revision INTEGER DEFAULT 0 | 选择实际变化时递增 |
| created_at / updated_at INTEGER | 登记和最近修改的 Unix 毫秒 |

provider_instances 保存 `id、provider_id、instance_kind、display_name`，不保存进程 PID、ready/error、实时 capability。enabled/settings 来自当前 manifest。插件暂时移除时保留身份关系，运行时列表仍只展示当前 catalog；恢复相同配置可复用实例 ID，避免破坏历史会话外键。registry_identity 标记该注册库所属设备，设备不匹配时报错。

### Host 连接关联对象

| 表 | 持久对象 |
| --- | --- |
| host_identity | 单个 device_id、display_name、created_at |
| host_tls_identity | 单个证书 DER、私钥 DER 二进制及各自 SHA-256、created_at |
| remote_credentials | credential/client ID、device_name、operating_system、system_version、bearer SHA-256、TLS 指纹、创建/最后访问/撤销时间 |
| signaling_identity | Host 的签名种子 BLOB |
| signaling_services | 按规范化 service_url 保存 host_token 和 auto_register |
| signaling_peers | 按服务 URL + credential_id 保存 public_key BLOB 和 token |

Host 身份与连接不重复写入每个 Provider 行。TLS 载入继续校验证书、私钥匹配和校验值；损坏时报错。明确更换 TLS 后，旧指纹凭据失效并要求重新配对，只清理 credential 记录。配对 session 仍是内存临时对象。bearer 明文不入库；上述小型凭据/证书校验不涉及会话长内容哈希。SQLite 本身未提供加密。

### Provider 用量

五项 token 指标各自使用 INTEGER NULL；聚合表每项另有 known_count，保证未知不变成 0。usage_sources 将 prefix_fingerprint 与 skip_partial_line 分列；usage_inbox 只保存已解析的计量字段，不存原始事件。快照拆为头、查询绑定、顺序选项、结果项和账户摘要表，事务保存和读取，重启后仍可按原查询翻页。查询绑定按字段精确比较，不对完整请求计算哈希。

具体列定义见 [Provider 用量查询与业务存储](provider-usage-query-and-storage.md) 及 `usage.sql`。时间单位保留查询语义：来源桶 at/duration 和快照 expires 用秒；记录 updated、接收 received 用毫秒；冻结结果中的时间字符串保持协议格式。

## 涉及模块

- `codepet-provider-data`：共享 DDL、事务、会话活动和用量 DTO 映射，集中持久化规则。
- `codepet-host`：设备/实例/配对/信令由 JSON 改为列，Provider 启动时分配同一共享库；移除无调用者的 JSON persistence helper。
- `src-tauri`：生产入口将各组件接到实际数据根的同一 app-state.sqlite。
- 附近测试和架构文档：使用新 SQL fixture，去掉旧文件布局与迁移承诺。

## 风险

| 风险 | 验证路径 |
| --- | --- |
| 跨进程活动丢更新、客户端已读相互影响 | 两独立 store 并发写、两个 reader、重开库及 Host Gateway 回归 |
| 活动更新覆盖 queue 控制字段 | storage_schema 测试先写 queue 值，再观察活动并核对 |
| 未登记实例产生悬空会话或写入一半 | 外键拒绝、失败事务后计数不变测试 |
| 用量 NULL/partial 或分页快照漂移 | 修正计量、冻结查询与账户摘要、重启翻页及错误 binding 测试 |
| 配对状态与证书不一致 | 证书破坏、轮换、bearer hash、descriptor 原子更新、LAN 授权测试 |
| 平台文件和打包差异 | 本轮 macOS 编译与自动化；发布前补 Windows、真实手机重新配对和安装包验收 |

## 测试计划与结果

- `cargo test -p codepet-provider-data --manifest-path crates/Cargo.toml --quiet`：35 项通过，包括结构化 schema、并发活动、用量冻结快照。
- `cargo test -p codepet-host --manifest-path crates/Cargo.toml --quiet -- --test-threads=1`：148 项通过，4 项依赖已安装 Harness、Live VPS 或 Android 的测试按原标记忽略。串行避免全进程 FD 计数测试受其他测试同时创建连接影响。
- 桌面检查：`TAURI_CONFIG='{"app":{"macOSPrivateApi":false},"bundle":{"resources":[]}}' cargo check --manifest-path src-tauri/Cargo.toml --quiet` 通过。覆盖用于绕过已有 Tauri feature 配置检查与工作区缺失的打包资源，不等价于完整发行构建；未修改产品配置。
- 存储提交本身未改协议或 UI；后续消息队列、Steer 的验证结果见对应功能文档。

## 知识沉淀

已同步 Path Manager、设置、Provider Host、用量和最近会话文档，新增 [桌面状态存储规约](../60-rules/desktop-state-storage.md)。[队列与 Steer](../20-product/conversation-input-queue-and-steer.md) 另行记录后续实现和验证。

## 未知项

尚未构建安装新包或连接真实手机；Live VPS、Android instrumentation 和依赖已安装 Harness 的忽略测试仍需专用环境。大量会话的全量内存状态读取成本未做压力验收；本次仅将实际变化的行写回，没有宣称全量读取已优化。
