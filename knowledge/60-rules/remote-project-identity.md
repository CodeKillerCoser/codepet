# Remote 项目身份规约

## 规则

复用 Codex 原生项目管理、让 Code Pet 创建的会话在原生 App 中归属到用户选择的同一项目，是产品成立的核心约束。兼容性改造必须保留这项能力，不能以自建项目体系或仅在 Code Pet 界面中分组代替。上游变化的维护成本由 Codex Provider 适配层承担。

Remote 项目实体来自 App Server `Project`。`Conversation.project` 表达显式归属，`workspaceRoot` 表达原生 cwd。项目页的查询成员还可使用桌面显示规则。目录判断只用于查询成员，正式项目 ID 和执行目录由原生数据及可核实的桌面显式关联提供。

Codex Desktop 尚未把桌面归属同步到 App Server `Thread.projectId` 时，Codex Provider 可读取当前 local host 的 `thread-project-assignments` 与桌面→App Server 项目 ID 映射。优先读取 `.codex-global-state.sqlite` 的 global settings，仅在数据库不存在时读取旧版 `.codex-global-state.json`。原生非空 `Thread.projectId` 始终优先。存在桌面归属但无法映射为原生项目 ID 的会话应排除在 standalone 之外。

项目资源必须保留完整 `deviceId + providerPluginId + providerInstanceId + nativeResourceId`。项目筛选和项目归属创建所携带的项目资源必须与目标 Provider route 完全一致；不一致时 Host 和 Provider 都要 fail closed。

## 适用场景

- 修改 Provider/Gateway v1 的 `project.list/get/create/update/delete`。
- 修改 `conversation.list` 的 all、standalone、project 筛选。
- 修改 `conversation.create.project` 或 Conversation DTO 的项目归属。
- 映射 Codex App Server 的 `Project`、`Thread.projectId`、`Thread.cwd` 或 `project/changed`。

## 反例

- 把不同 worktree 的 cwd 按 Git remote 或 basename 合成一个项目：会绕过上游项目身份和排序。
- 把仅按 cwd、根目录提示或 worktree 匹配到的对话写入 `Conversation.project`：会把列表分组误作正式归属。
- 用项目第一个 root 覆盖 `workspaceRoot`：会丢失 App Server 返回的真实执行 cwd。
- 只比较 `nativeResourceId`：不同设备或 Provider instance 的同名项目会串路由。
- App Server 未启用实验 Project API 时仍广告部分项目方法：Remote 会看到不可兑现的能力。
- 把 App Server 全量会话按 cwd 分组，或直接把 legacy project id 暴露为 `Conversation.project`：会漏掉 worktree、破坏 routed identity，并把 Desktop 私有 ID 泄漏到公开协议。
- 项目筛选先过滤整页再透传原生 cursor：稀疏项目会产生短页、漏页或无法稳定续读。
- 为减少协议适配而改成 Code Pet 自管项目，或在归属无法确认时将用户选择的项目会话静默创建成 standalone。

## 推荐做法

- Agent schema 中 Project 使用两段 routed `resource`、`name`、`roots[{path}]`、`metadata: Map<String,String>`、只读 `position`、`createdAt`、`updatedAt`；Provider/Gateway 共同引用该定义。
- Codex 项目页将额度分成两个互斥查询。原生分支按会话索引的 `project_id` 选择 ID，覆盖能按 ID 回读但原生列表省略的记录。旧会话索引缺少 `project_id` 时，这一路用 App Server 的项目过滤。桌面分支只接受原生归属为空且符合桌面显式关联、目录或 worktree 规则的会话。显式归到其他项目、未映射关联和标记为无项目的记录不进入目录匹配。worktree 可用桌面保存的根提示或 Codex 管理的 Git 注册，保持原生 cwd。
- 两路各自先筛选再分页，本批回读正式摘要并去重排序。默认额度 20 时各取最多 10 条。短页可继续，下一批可能含比前一批页尾更新的对话。一个签名游标绑定实例、查询、桌面归属和根目录，分别携带两路续读位置。桌面分组变化时，Remote 重新请求第一页。Provider 在页与页之间只依赖游标。
- Remote 项目页按查询返回的 ID 和顺序显示，并共享现有摘要状态。同一项目续页追加 ID，第一页替换项目查询窗口。仅由桌面目录规则匹配的成员保留空的 `Conversation.project`。
- Remote 对 resume/get 与 conversation.upserted 的运行时 metadata 采用有限合并：同会话的 project 为 null 时保留已知归属，非空 project 正常覆盖；状态、标题和执行配置仍以新值为准。列表替换不使用此 fallback。当前 DTO 无法区分 missing/null，未来显式解除项目关联必须给出可区分的操作语义，不能让运行时缺值误清归属。详见 [resume 排查](../40-runbooks/codex-resume-no-output-and-incomplete-history.md)。
- `conversation.create` 只在调用方显式传入项目时映射 Codex `thread/start.projectId`；省略即 standalone，不根据 workspaceRoot 推断。
- Codex instance start 在已启用 experimental API 的 session 上探测 `project/list`。只有成功时才整组广告并映射五个项目方法；明确的缺失接口信号表示不支持，其他探测错误记录诊断并保持项目能力不可用，不得连带丢弃已经成功发现的模型和发送能力。原生项目能力不可用时，保留已有会话可用的读写能力，但不绕过项目校验创建会话。
- `project/changed` 只由 observer session 映射为 Provider `event.projectChanged`，再由 Host 转换为 Gateway replayable `project.changed`；conversation writer session 不重复发布。
- 保留 `workspaceRoot = Thread.cwd` 的原始字符串；不执行规范化、canonicalize 或文件系统扫描。
- Codex App Server 的 `Project.id` 与非空 `Thread.projectId` 是 Provider/Gateway 的权威项目契约。桌面设置仅提供归属补齐，读取 SQLite 时用同一条查询获取两项设置。当前 SQLite 中归属缺失视为当前值，避免用迁移前的 JSON 恢复已取消的归属。存储不可读或归属值损坏时返回错误，避免静默把项目会话归为 standalone。

## 来源

2026-09-03 Gateway v1 Project 第一版。实现依据本机 Codex 0.151 App Server 实验 schema 的 `project/list|read|create|update|delete`、`project/changed`、`Thread.projectId` 与 `thread/start.projectId`。此前基于 cwd/Git/worktree 的项目投影规约已被这一上游权威项目模型取代；旧实机重复项目证据仍说明“不能把 cwd 当项目身份”，但不再支持本地推断方案。

2026-09-04 实机确认 Codex Desktop 升级后 `projectsMigrated=true`、`threadAssignmentsMigrated=false`：0.151 与 0.152 都能列出 6 个项目，但 `codepet` 原生筛选为 0；Desktop 私有状态仍记录 117 条 `codepet` assignment，其中 89 条未归档会话位于 worktree。曾引入的 cwd/root 兼容只能返回主工作区约 12–14 条。用户决策改为由 Provider 合并 App Server thread 与 Desktop assignment，同时保持 App Server Project id 和 opaque cursor 边界。

2026-11-25 用户明确：复用 Codex 自己的项目管理，使本 App 创建的会话在原生 App 中归属正确，是选择 Code Pet 而非其他开源客户端的主要原因。这是后续架构和兼容方案必须保留的前提。[项目响应升级排查](../40-runbooks/codex-project-position-schema-drift.md)记录了字段变化与能力发现故障隔离，并记录了原生 ID 正确仍不足以证明桌面显示一致的验收缺口。

2026-11-25 本次兼容修复提交：用户确认暂缓处理既有的“Code Pet 新建 → 原生 App 自动归组”问题。该次提交验证项目列表、原生 ID 保留及两轮对话恢复，侧栏归属的历史失败证据继续保留；这不改变后续产品验收目标。

2026-11-27 实机只读核对发现新版桌面 SQLite 已保存近期归属，迁移前的 JSON 保留旧值。例如「梳理 codepet-task 任务归并」可通过无筛选 `thread/list` 读取，原生项目筛选遗漏该条，桌面 SQLite 中则有可解析的 `codepet` 归属。

## 验证方式

- 产品验收必须使用同一 thread ID 完成「Code Pet 选择原生项目并创建 → 独立回读原生归属 → 原生 App 在正确项目下显示」，覆盖 main 与 worktree。接口成功、fixture 通过或 Code Pet 自己显示分组均不能代替该验收。Desktop 显示不一致须单独记录并排查，不为通过验收篡改私有状态，也不把显示延迟改报成创建 RPC 失败。
- `npm run protocol:check`：校验 schema/manifest、typed map codegen 和生成文件无漂移。
- Codex client 单测断言五个上游方法、精确参数和 `thread/start.projectId` 省略/显式映射。
- Provider 单测覆盖 local host legacy→native 映射、未映射项目不进入 standalone、原生 `Thread.projectId` 优先，以及 filter-bound cursor 拒绝跨项目复用。
- Provider 纵向测试覆盖旧 JSON 兼容、新 SQLite 补入原生筛选漏项和新查询采用当前归属。验证已移出项目的会话不会因旧 JSON 被重新归入项目。
- 项目双游标至少验证两路均多于额度、一路先读完、原生列表查不到但会话索引里有 project ID、正式归属与目录匹配冲突、无项目排除、同时间戳、游标跨项目和归属变化失效。手机验证无正式归属的行可显示、续页按 Provider 批次顺序追加、失效游标从第一页恢复。
- 用 0.151 与 0.152 对同一隔离状态快照执行 `project.list`、逐项目 `conversation.list`、standalone 与 all 全分页；断言 cursor 无重复、会话 ID 无重复、项目/standalone 无交叉且分类总数与 all 闭合。
- Codex Provider vertical fixture 覆盖项目 CRUD、项目筛选、项目归属创建，以及 `project/list = -32601` 时能力与创建均 fail closed。
- 项目归属创建 fixture 必须在独立后续 `thread/read` 中恢复同一个 `projectId`，同时断言 `thread/start` 发送的是显式项目 native id 且没有用 cwd 推断。实机诊断先分别检查 App Server state DB 的 `threads.project_id` 与 Desktop legacy registry；二者不一致时不得归因为 Remote/Host 丢字段。
- Host Gateway 集成测试覆盖五个能力、CRUD、`snapshotCursor`、route-less 项目筛选的路由收敛和项目归属创建。
- mapper 测试断言 `workspaceRoot` 原样等于 native cwd、Project routed identity 和 `project.changed` changeType。
- `cargo test --workspace` 与 Dart Gateway SDK `dart test` 覆盖 Rust/Dart DTO 和 canonical JSON fixtures。
