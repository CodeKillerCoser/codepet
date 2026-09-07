# 最近会话：Provider 原子能力与 Host 分页视图

## 背景

2026-09-08 用户确认：首页“最近”不能依赖 Remote 已加载的 conversation.list 页面再做局部排序。很旧的运行中或未读会话可能不在这些页面中；关注会话数量也可能很大，因此最终最近列表必须分页。

本方案已完成源码实现、独立审查和本地集成，尚未编译或运行验收。Host 基线为 v0（4e32b87），Remote 基线为 main（3f20065）；集成代码分别至 Host `1f6618a`、Remote `7248092`。任务、修复与未验证边界见 [交付计划](../40-runbooks/recent-conversation-delivery.md)。

## 目标

- Provider 只提供会话查询、活动状态和阅读进度等原子能力；Host 统一生成最近视图。
- 最近候选集为 active ∪ unread ∪ updatedAt 在最近 14 天内的会话；按完整结果排序后分页。
- Remote 只调用最近聚合接口，触底自动加载；首页最近不显示“加载更多”按钮。
- 状态变化可以使未在 Remote 缓存中的会话加入最近，支持失效刷新、断线恢复和时间到期移除。

## 非目标与保护边界

- 聊天和项目分组、详情、搜索、创建流程不改版。聊天仍是 list(projectId=null)，项目仍是 list(projectId=xxx)；wire 继续使用现有 Standalone/Project projectFilter。
- 聊天、项目原有分页入口和分页方式不变，不能跟最近共用游标、窗口或成员集合。聊天不是近期历史，旧会话不能被 14 天规则隐藏。
- 运行状态、未读含义、标记已读触发时机及现存已读记录不改变。现有按 callerScope 区分阅读进度的语义必须保留，不能改成全设备全局已读。
- 不修改 Harness 的 updatedAt 来实现置顶；不复制完整 transcript 到 Host；不涉及 Pet、Desktop companion、WebRTC、配对或模型配置。
- 本轮用户明确要求不编译、不运行构建/测试。可以编写测试、检查代码与协议、执行不触发编译的代码生成；自动化与真机验证结果必须写“未执行”。

## 现状理解与证据

- `crates/codepet-host/src/conversation_state.rs`：Host 目前维护 callerScope、activityVersion 和已读进度；mark_read 只推进到 observedActivityVersion。
- `crates/providers/codepet-provider-codex/src/protocol.rs::thread_list_params` 已固定 updated_at/desc/useStateDbOnly。
- `crates/providers/codepet-provider-claude/src/provider.rs::list_conversations` 从本地持久历史发现会话并排序分页，不调用 Agent SDK listSessions。
- `crates/providers/codepet-provider-opencode/src/client.rs` 使用 V2 /api/session 与 /api/session/active；目前只给返回页中的会话附上 active。核对 v1.18.29 上游源码，其原生 list 按创建时间分页，不能见到旧创建时间就停止查更新时间。
- Remote `DeviceSession.selectedProviderRecentConversations` 目前仅对已加载集合过滤；最近和聊天复用 all 列表来源。这是最近方案接入时必须拆开的边界，聊天应恢复独立 standalone 列表查询而非从 recent/all 页推断完整性。

## 实现路径

### 分层与命名

讨论中的 session 对应现有协议里的 conversation；沿用 conversation 命名，不引入并存的 session 协议族。

| 层 | 新增职责 | 不应知道的细节 |
| --- | --- | --- |
| Harness adapter | 转换原生查询、状态、时间与事件 | 14 天、首页分组、最近优先级 |
| Provider SDK/Provider | 查询原子能力；活动/未读权威；公共持久化 fallback | UI 展示、Remote 滚动、最近分页 |
| Host application service | 候选集合、只含摘要的索引、排序、最近快照、失效通知 | 各 Harness 原生存储格式 |
| Remote application/UI | 最近分页控制器、订阅失效、展示、滚动锚点 | 用已加载条目推测全局排序 |

Gateway facade 接收聚合请求；真正聚合逻辑放独立应用模块，不堆进 transport/巨型 dispatcher。状态转换只保留一套权威，Host 缓存是可重建视图。

### Provider 原子能力（契约冻结目标）

1. `conversation.active.list`：按运行实例枚举活动资源 ID、状态及版本，支持分页/快照。活动沿用现有状态语义，包含尚未终结且等待审批/输入的执行，不重新定义历史会话状态。
2. `conversation.unread.list`：给定不透明 readerScope，枚举未读资源 ID、readState，支持分页/快照。
3. `conversation.list`：保持已有 projectFilter/cursor/limit 请求行为；增加可选的通用查询模式：按 `updatedAfter`（包含边界，UTC）读取，或按明确 `ids` 批量读取摘要。两种选择模式互斥，不与最近产品规则耦合。日期模式保证过滤先于分页、updatedAt 降序；IDs 模式必须覆盖指定 ID 集合，允许有界批次，已删除项可省略，其他失败不得当作删除。旧请求未传新字段时行为不变。
4. `conversation.markRead`：Provider 接受 readerScope、资源和 observedActivityVersion；Gateway 保留现有客户端请求 shape，由认证上下文填充 scope 并转发。
5. active/unread 的变化通过 Provider typed events 表达；普通摘要更新、删除继续沿用已存在的事实事件，缺少删除事件时契约补齐明确失效语义。版本用于判旧、防丢更新，不能混用 Gateway eventCursor 与最近 revision。

协议仍然是 v1（用户明确要求），不创建 v2、不递增 protocolVersion。方法、事件、capability 与可选字段在既有 v1 canonical schema/manifest 内增量扩展；旧请求未提供新字段时语义不变，向旧 Provider 发新请求前必须协商能力，不能假设旧版严格decoder能接受新增字段。

字段/DTO 的最终拼写、分页 wrapper、capability 名称由契约任务按照现有 codegen 风格一次冻结并写回交付计划；语义不能自行削弱。日期、IDs 模式不能退化为“只过滤上游当前一页”。若原生没有所需过滤，Provider 内部扫描完整所需范围或使用公共摘要索引；不能把全量 transcript 传给 Host。

### 未读权威下沉与兼容

- 原生有与 callerScope/观察版本相同语义的能力才直接映射；原生全局已读不等价于当前多客户端阅读进度。
- 原生不支持时，公共 Provider SDK 模块提供持久化阅读进度与活动版本，不让三个 adapter 分别实现数据库/指纹规则。
- 当前 Host 已读数据必须无损迁移或通过单一共享存储实现兼容接管：保留 scope、baseline、已读版本、活动版本及历史指纹；接管必须原子/幂等，有备份，不能两个进程同时作为同一记录的独立写入权威。具体可实现的接管机制由 Provider/Host 任务联合给出并经 PM review 后集成。
- 旧 Provider 可以继续使用原有列表/状态路径；仅在新原子能力语义齐备时广告最近能力。unsupported 不能冒充空集合；不能静默把所有历史标为未读或已读。
- 如果无损接管存在未解决问题，报告真实阻塞，不以删存储、重置未读或改变 scope 作为捷径。

### Host 聚合、完整性与分页

按 providerId + readerScope + 当前实例 generation 建立最近视图；客户端没有权限直接选择其他 readerScope。

```text
active IDs ─┐
unread IDs ─┴─ 合并 ID、批量补齐缺失摘要 ─┐
updatedAfter 查询（14天）───────────────┴─ 去重 → 排序 → 快照 → 切页
```

运行中优先、未读次之、其余近期最后；每组 updatedAt 降序；同时间按完整 routed resource identity 排序。一个会话只出现一次，同时 active/unread 归 active。

必须在对 Remote 截页前建立足够完整的候选索引。第一版允许 Host 后台完整收集 active/unread 的枚举页和所需近期摘要页、构建可重建快照；不能把不完整首批伪装成完整第一页。Provider 日期索引后续可优化，不暴露额外产品语义。集合大时，内部请求与并发有界、可取消且不阻塞控制消息；不截断到任意 N 条冒充完整。

Gateway `conversation.recent` 的稳定目标形状：

```text
request  { providerId, cursor?, limit? }  // 默认20，上限100；14天为Host策略
response { conversations, pageInfo:{nextCursor?}, revision, snapshotCursor }
```

recent 返回最终业务排序的整张列表分页，不是“普通近期一页+全部关注会话”。100 个运行中/每页20条时，前5页均为运行中，之后才能到未读。不能先各截一页再合并。

游标是不透明 token，绑定认证 callerScope、Provider/generation、快照 revision、截止时间与翻页位置；越权/篡改/异 Provider 必须拒绝，过期明确返回 recent_cursor_expired。nextCursor 缺失才表示结束，不用返回未经确认的总数。

### 变化、事件和刷新

Host 在 active/unread/摘要/删除/Provider generation 变化、14 天边界到期时失效相关视图，发布 `conversation.recentChanged`（providerId、revision）。readerScope 由 Host 定向路由，不泄漏给其他客户端；scope 不兼容现有 replay 流时，可发不携带任何会话信息的 Provider 范围失效提示并让每个客户端重新取自己视图。

快照 fence 与订阅衔接沿用 Gateway snapshotCursor 机制。构建期间发生变化必须重试/标记失效，不能在事件之后安装更旧列表。只有会影响列表成员、顺序或展示的合并变化才推进可见 revision；高频 delta 不逐 token 重建全量。终态和 markRead 的确定结果必须触发正确失效。

第一版使用“失效后重取”而非复杂 move/insert 位置补丁。Host 合并短时间突发事件；Remote 为失效刷新使用独立请求 generation。旧 cursor 不接到新 revision，后到旧响应也不能覆盖新结果。

Remote 收到失效或 cursor_expired 后取消旧尾页、重取新首屏、原子替换最近窗口并重置分页。保留可定位的顶部会话 ID/像素偏移；若锚点落在后续页，可按新快照逐页恢复已加载深度/锚点，不能为恢复位置混入旧快照项目。锚点消失时选相邻可见项；至少不因一次刷新无条件跳到首页顶部。首屏滚动区域不足以触发滚动时，也要按需填充下一页；并发请求只允许一个当前尾页，错误可重试但不能无限自动重试。

### 保持聊天与项目列表不变

Remote 必须有独立 all/recent、standalone、project 路由状态，且 recent 不再复用 selectedProviderConversations 的局部排序作为权威。普通列表可以复用相同摘要对象和共享行组件，但成员、游标、计数、加载中/错误必须按 scope 隔离。点击项目仅影响该项目页；最近事件不重置聊天/项目分页。获取 recent 不隐式改动项目归属、运行状态或已读版本。

## 涉及模块

- `protocol/{agent,provider,gateway,core}/v1`、codegen、各语言 SDK：统一原子查询/事件/聚合契约，不手改生成结果。
- Provider SDK 与三个内置 Provider：原生适配、公共状态 fallback、摘要索引和持久化兼容。
- `crates/codepet-host/src/{gateway,conversation_state,providers}`：调用编排、认证阅读 scope、视图生命周期、事件/replay、已读权威接管。
- Remote `application/ports`、`application/sessions`、新 recent controller、gateway mapper、home、demo/fixtures：独立 recent 通路与自动分页；保护原有 standalone/project 行为。

## 风险

| 风险 | 代码检查/后续验证 |
| --- | --- |
| 未读迁移重置或跨设备互相标读 | 旧文档兼容测试、两 readerScope、重启/接管重放、同时到达新活动 |
| 排页前遗漏关注会话 | 超过100个旧active/unread，断页、交集、deleted IDs、完整性检查 |
| OpenCode 创建时间被误当更新时间 | 很旧创建且刚更新的会话必须在最近；不能按创建时间提前停止 |
| 事件/响应乱序 | 构建中变化、旧revision尾页、重连/Provider切换、snapshot fence |
| 大集合耗时/风暴 | 有界并发、取消、复用摘要索引、不逐token重扫；真实耗时待测 |
| 聊天/项目被最近逻辑侵入 | assert Standalone/Project 请求，14天前历史可见，旧分页窗口不重置 |

## 测试计划

应补齐但本机不执行：协议fixture及生成一致性；Provider 日期/IDs/active/unread/markRead；Host 全局分组分页、游标隔离、失效；Remote 触底/首屏不足/刷新锚点/错误重试/重连；聊天项目状态与分页回归。后续获准或CI资源可用时再运行 protocol:check、SDK/Host/Provider 测试、Remote flutter test/analyze 以及安装验收，不将“测试已写”写成“测试通过”。

## 知识沉淀

本文是唯一语义设计源。交付计划记录任务、冻结契约、提交、审阅与未验证项。实现偏差需 PM 修改本文再传播，禁止各任务把不同假设写进互不相干的文档。

## 未知项

- 新 Provider 公共状态模块的旧存储接管机制、外部 Harness 活动可观测范围和冷启动索引成本，须由实现与源码审查明确。
- 未运行编译、自动化、真实服务或真机验证；不能以 mock 的正确性代替 Harness 的分页/事件保证。
- Home 的完整滚动锚点恢复与任意大集合的性能待后续UI/压力验收。
