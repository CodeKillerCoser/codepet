# Remote 项目身份规约

## 规则

Remote 项目实体只来自 App Server `Project`，Provider 对外仍只返回 routed `Project.resource` 和 `Conversation.project`。`workspaceRoot` 只表达会话的原生 cwd，不得通过 Git remote、common-dir、worktree 路径、basename 或目录存在性推断项目身份，也不得由项目 roots 反推或改写 cwd。

Codex Desktop 0.151/0.152 若已迁移项目但尚未把 legacy thread assignment 同步到 App Server `Thread.projectId`，Codex Provider 可以只读 `.codex-global-state.json` 的当前 local host `thread-project-assignments` 与 legacy→App Server project id 映射补齐归属。原生非空 `Thread.projectId` 始终优先；私有状态只提供 thread→native project id 关联，不提供 Project 实体或 Conversation 内容。存在 legacy assignment 但无法映射为原生 project id 的 thread 不得误入 standalone。

项目资源必须保留完整 `deviceId + providerPluginId + providerInstanceId + nativeResourceId`。项目筛选和项目归属创建所携带的项目资源必须与目标 Provider route 完全一致；不一致时 Host 和 Provider 都要 fail closed。

## 适用场景

- 修改 Provider/Gateway v1 的 `project.list/get/create/update/delete`。
- 修改 `conversation.list` 的 all、standalone、project 筛选。
- 修改 `conversation.create.project` 或 Conversation DTO 的项目归属。
- 映射 Codex App Server 的 `Project`、`Thread.projectId`、`Thread.cwd` 或 `project/changed`。

## 反例

- 把不同 worktree 的 cwd 按 Git remote 或 basename 合成一个项目：会绕过上游项目身份和排序。
- 当 `Thread.projectId` 为空时按 cwd 猜项目：会把 standalone 会话错误归属。
- 用项目第一个 root 覆盖 `workspaceRoot`：会丢失 App Server 返回的真实执行 cwd。
- 只比较 `nativeResourceId`：不同设备或 Provider instance 的同名项目会串路由。
- App Server 未启用实验 Project API 时仍广告部分项目方法：Remote 会看到不可兑现的能力。
- 把 App Server 全量会话按 cwd 分组，或直接把 legacy project id 暴露为 `Conversation.project`：会漏掉 worktree、破坏 routed identity，并把 Desktop 私有 ID 泄漏到公开协议。
- 项目筛选先过滤整页再透传原生 cursor：稀疏项目会产生短页、漏页或无法稳定续读。

## 推荐做法

- Agent schema 中 Project 使用两段 routed `resource`、`name`、`roots[{path}]`、`metadata: Map<String,String>`、只读 `position`、`createdAt`、`updatedAt`；Provider/Gateway 共同引用该定义。
- `conversation.list(all)` 逐页读取 App Server 会话并用 legacy assignment 补齐缺失的 `projectId`。`standalone` 与 `project` 在原生 thread 流上按“原生 projectId 优先、legacy 映射补缺”分类；Provider cursor 封装 filter identity 与上游 opaque cursor，只有完整消费上游页后才推进，客户端必须原样回传。
- `conversation.create` 只在调用方显式传入项目时映射 Codex `thread/start.projectId`；省略即 standalone，不根据 workspaceRoot 推断。
- Codex instance start 在已启用 experimental API 的 session 上探测 `project/list`。只有成功时才整组广告并映射五个项目方法；`-32601` 表示不支持，其他探测错误使 instance start 失败。
- `project/changed` 只由 observer session 映射为 Provider `event.projectChanged`，再由 Host 转换为 Gateway replayable `project.changed`；conversation writer session 不重复发布。
- 保留 `workspaceRoot = Thread.cwd` 的原始字符串；不执行规范化、canonicalize 或文件系统扫描。
- Codex App Server 的 `Project.id` 与非空 `Thread.projectId` 是 Provider/Gateway 的权威项目契约。Codex Desktop sidebar 的 legacy→App Server id 映射可在原生归属缺失时只读补齐，但 Provider 不写 `.codex-global-state.json`、不伪造 legacy id，也不把 Desktop sidebar 可见性当作 `conversation.create` 成功条件。文件缺失等价于没有兼容数据；文件不可读或 JSON 损坏时 fail closed，避免静默把项目会话归为 standalone。

## 来源

2026-09-03 Gateway v1 Project 第一版。实现依据本机 Codex 0.151 App Server 实验 schema 的 `project/list|read|create|update|delete`、`project/changed`、`Thread.projectId` 与 `thread/start.projectId`。此前基于 cwd/Git/worktree 的项目投影规约已被这一上游权威项目模型取代；旧实机重复项目证据仍说明“不能把 cwd 当项目身份”，但不再支持本地推断方案。

2026-09-04 实机确认 Codex Desktop 升级后 `projectsMigrated=true`、`threadAssignmentsMigrated=false`：0.151 与 0.152 都能列出 6 个项目，但 `codepet` 原生筛选为 0；Desktop 私有状态仍记录 117 条 `codepet` assignment，其中 89 条未归档会话位于 worktree。曾引入的 cwd/root 兼容只能返回主工作区约 12–14 条。用户决策改为由 Provider 合并 App Server thread 与 Desktop assignment，同时保持 App Server Project id 和 opaque cursor 边界。

## 验证方式

- `npm run protocol:check`：校验 schema/manifest、typed map codegen 和生成文件无漂移。
- Codex client 单测断言五个上游方法、精确参数和 `thread/start.projectId` 省略/显式映射。
- Provider 单测覆盖 local host legacy→native 映射、未映射项目不进入 standalone、原生 `Thread.projectId` 优先，以及 filter-bound cursor 拒绝跨项目复用。
- 用 0.151 与 0.152 对同一隔离状态快照执行 `project.list`、逐项目 `conversation.list`、standalone 与 all 全分页；断言 cursor 无重复、会话 ID 无重复、项目/standalone 无交叉且分类总数与 all 闭合。
- Codex Provider vertical fixture 覆盖项目 CRUD、项目筛选、项目归属创建，以及 `project/list = -32601` 时能力与创建均 fail closed。
- 项目归属创建 fixture 必须在独立后续 `thread/read` 中恢复同一个 `projectId`，同时断言 `thread/start` 发送的是显式项目 native id 且没有用 cwd 推断。实机诊断先分别检查 App Server state DB 的 `threads.project_id` 与 Desktop legacy registry；二者不一致时不得归因为 Remote/Host 丢字段。
- Host Gateway 集成测试覆盖五个能力、CRUD、`snapshotCursor`、route-less 项目筛选的路由收敛和项目归属创建。
- mapper 测试断言 `workspaceRoot` 原样等于 native cwd、Project routed identity 和 `project.changed` changeType。
- `cargo test --workspace` 与 Dart Gateway SDK `dart test` 覆盖 Rust/Dart DTO 和 canonical JSON fixtures。
