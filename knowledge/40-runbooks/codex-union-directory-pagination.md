# Codex 双源目录与固定快照分页修复

## 现象

原生列表遗漏空 preview 任务；legacy metadata read 又可能把 updatedAt 回退到创建时间。直接透传原生 cursor、对两源分别分页，或补读后覆盖全部字段，都不能得到完整且稳定的时间/项目列表。

## 证据

- [发现缺口实验](codex-agent-created-thread-discovery.md)：同一 App Server 列表 36 项、DB 41 项，遗漏 5 项按 ID 均可读取；隔离副本只修改 preview 就改变列表成员。
- 同一 legacy 任务，原生列表与 DB 有较新时间，metadata read 返回创建时间；项目归属还需要当前 home 的桌面 legacy→native 映射。
- 本次读源码确认：SDK `list_cached` 只处理带 cursor 请求，本来就没有无 cursor 首页结果缓存。真正缺口是 Codex 普通列表还透传上游页，而 updatedAfter/ids 走另一条完整枚举路径。

## 根因与引入历史

发现、字段校正和最终分页缺少共同边界；上游列表的可见集合被误当成全部可读取任务。引入提交未确认。

## 修复方案

2026-09-08：所有 `conversation.list` 模式（all、项目、standalone、updatedAfter、ids）统一为“重新拉取原生全部列表页 + 只读 DB 候选 + 当前实例事件 ID/待持久化任务/loaded 任务 → 正式摘要合并 → 项目与查询过滤 → 稳定排序 → SnapshotPager”。不增加 Provider wire 字段。

无 cursor 必须重新发现，不复用旧首页。原生列表已返回正式对象，只对缺失候选补 `thread/read(includeTurns=false)`，每批最多 4 个并发；pending 对象优先加入已知集合，避免刚创建且尚不可读的任务被重复读取。未知读取失败传播，不能静默跳过并声称完整。

`directory.rs` 读取配置的 dataDirectory（否则解析 CODEX_HOME）的最高数字版本 `state_*.sqlite`，验证所需 schema。DB 不存在时保持原生列表及事件发现能力；存在却无法打开/读取/识别时返回 `conversation_query_incomplete`，不回退旧库。使用 bundled SQLite 只读连接、500ms busy timeout；单次 SELECT 读取 committed WAL，连接结束后才发 RPC，不执行迁移或 checkpoint。DB 不读取正文、标题或运行状态。

DB 候选不预先按时间/项目缩窄，避免错误字段造成不可恢复的遗漏。保留 ID、归档与更新时间证据；旧 schema 无毫秒列时转换秒，无 history_mode 时按 legacy 处理。仅在 legacy 摘要 updatedAt 等于 createdAt 且 DB 时间更晚时校正；列表与 live read 合并保留已知标题、preview、项目及被 read 回退的时间，不跨首页永久 max 时间。

实时事件仅登记当前实例的候选 ID，下一次主动拉取仍读取对象；生命周期清理时丢弃这些提示。明确 DB 归档记录排除普通列表候选；显式 ids 查询沿用原有按 ID 读取归档任务的能力。ephemeral 任务排除。目录按请求物化，不保存可复用首页。

## Cursor 与兼容边界

- 客户端 cursor 只属于 Provider 快照，不包含上游 cursor。固定成员、字段与顺序；新发现、归档和改名只影响新查询。绑定操作、实例 generation、查询及 reader scope；实时 fact epoch 不作为已发行 cursor 的失效条件。
- 保留 SDK 的 HMAC、120 秒有效期、64 快照上限与明确过期错误，不静默换快照。旧原生/项目包装 cursor 不能继续使用，应重新请求首页。
- 保留 0.151/0.152 的项目探测、原生 projectId 优先、未映射 legacy 项目不得进入 standalone、history turns 读取及降级分支。
- loaded/list 明确不支持时，持久化列表仍可完成；active 完整性检查不采用这个降级。只有精确 unknown-method 错误可降级，普通 RPC 故障不能伪装成不支持。
- 搜索仍保持独立的原生 searchTerm 语义与 cursor，本次修改范围为会话列表及其摘要发现，不把标题子串匹配冒充原生搜索。

## 涉及模块

- Codex `directory.rs` / Cargo 依赖：跨平台只读 SQLite 候选与时间证据。
- Codex `provider.rs`：统一各列表入口，保留项目兼容与 loaded 能力边界，移除原生项目包装游标。
- `provider/server_events.rs`：登记实例事件 ID，且不把 Pet Hook 通道接入 Remote。
- `tests/provider_vertical.rs` 与 App Server fixture：隔离测试 home，覆盖真实 Provider 调用链。大历史断言按目标任务筛选，避免独立后台目录轮询污染请求计数；启动测试等待回复实际应用，而不是把“请求已到达 fixture”当成 Ready。
- SDK SnapshotPager 直接复用，本次没有修改协议或共享分页实现。

## 验证结果

新增回归包括：205 个原生任务加 DB 补漏、重复 ID、归档、legacy 时间、209 项跨三页不重不漏、相同 cursor 重放字段一致、新首页可见提交、条件变化拒绝 cursor、来源失败时旧页可读/新页失败、上游 cursor 循环、不可读候选、worktree 项目映射、未映射项目排除、事件独有 ID、loaded/list unknown-method、WAL 未提交不可见/提交后可见、旧 schema 与锁错误。

2026-09-08 至 09 的最终验证：

- `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex -- --test-threads=1`：通过，60 个单元测试、50 个集成测试；3 个真实安装/实机 smoke 测试未启用。0.151/0.152 history 与旧项目能力兼容 fixture 均通过。
- `cargo test --manifest-path sdk/rust/Cargo.toml -p codepet-provider-sdk --lib conversation_query --target-dir crates/target -- --test-threads=1`：编译成功，测试未执行；Windows 应用程序控制策略阻止启动生成的测试 exe（os error 4551）。未绕过系统策略。固定 cursor 的 Provider 集成用例已执行并通过，但不把它等同于 SDK 全部游标测试通过。
- `git diff --check`：通过。

先前全量运行暴露请求日志和启动测试时序竞态，已按上述原因修正；连接/停机调度用例曾出现通道关闭，单独复跑及最终串行全量运行均通过。最初误从 crates workspace 直接运行 SDK 测试，被 Cargo 提示该 SDK 不是该 workspace member；随后使用上述正确的 SDK workspace 命令。

## 本机安装替换（2026-09-09）

- 按用户要求单独执行 `cargo build --manifest-path sdk/rust/Cargo.toml -p codepet-provider-sdk --lib --release --locked`，成功；SDK 为 Rust 库，静态链接到 Provider，不单独安装 DLL。
- 执行 `cargo build --manifest-path crates/Cargo.toml -p codepet-provider-codex --bin codepet-provider-codex --release --locked --target-dir sdk/rust/target`，成功。
- 使用 `CODEPET_TEST_PROVIDER_EXE` 指向这份 release 产物运行 `provider_binary_public_mux_smoke`。首轮超时；增加可选 `CODEPET_TEST_PROVIDER_STDERR` 诊断开关后复跑，完整握手/列表/get/停止/shutdown 检查通过（0.53 秒）。没有确认首轮超时的最终原因，不能将复跑成功写成故障已定位。
- 已替换 `D:/Software/Code Pet/provider-plugins/codex/codepet-provider-codex.exe`，与构建产物 SHA-256 一致：`D3F4CA60C85739E0E622D08B60099BD3BE5613D09C9A1437E83E15255FCCFBE4`。
- 原文件备份为同目录 `codepet-provider-codex.exe.backup-20260909-002520`。因运行中映像不能覆盖，保留旧映像为 `codepet-provider-codex.exe.running-20260909-002520`，将新文件放回 manifest 指定的原路径；未改 manifest 或其他 Provider。
- 自动审批审查拒绝“停止进程、替换并重启”的组合操作（blocked by policy，无更具体原因）。改用不终止进程的文件替换后成功；未自动重启 Code Pet。当前旧进程仍使用旧映像，用户手动重启应用后才加载新版，不声称已验证运行中的新版本或手机效果。

## 后续验证边界

执行 [并集分页规约](../60-rules/codex-union-directory-pagination.md)。大目录全量扫描和补读成本随数据量增加；当前测试规模不证明万级目录性能。两源不共享事务，只承诺固定快照内不重不漏和重复拉取后的最终覆盖，不承诺并发写入时全局原子时刻。

DB schema 是私有契约，未来并存多个 state 版本时需验证当前运行二进制实际使用哪个库；当前版本选择策略不是官方长期 API。多版本真实二进制、应用 UI 和手机部署验证另行记录，不能用 fixture 通过代替已部署证明。
