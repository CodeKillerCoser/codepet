# 事件管线

## 来源

Claude Code、Qoder 和 Cursor 的 hook payload 由 `src-tauri/hooks/code-pet-hook.mjs` 接收，并发送到本地 collector：`http://127.0.0.1:47621/hook`。

Codex 不再进入这条活动管线：脚本消费 stdin 后直接退出，collector 的实时 `/hook` 与启动 spool 回放均显式拒收 Codex。`src-tauri/src/agent/codex_audit.rs` 及其 watcher/回放入口已删除。Token 用量页保留的历史 audit/transcript 扫描不向 `PetEvent` 或桌宠活动写入数据。

Codex 当前有两个标准协议事件源，但只有一个进入桌宠：

- remote App Server event 由 `codepet-provider-codex` 映射到 Provider v1，经 Plugin Manager 校验后只写入 `ProviderGatewayService` replay，并由 compat bridge 通过 `runtime-gateway-event` 面向既有远程客户端；PetApp 不监听该 event，也不读取 remote replay。
- Desktop IPC event 只写入 `CodexDesktopCompanionState` 的 companion event bus，通过专用 snapshot/replay 与 `codex-desktop-companion-event` 驱动 PetApp。

Provider Gateway 与 companion event bus、sequence/replay 物理分离。Provider event 只写 `runtime-gateway-event`，不写 Desktop companion 或 Pet 状态。隔离发生在生产 wiring，而不是靠 UI 过滤；remote 与 Desktop 的同名 native thread 不做跨链路协调。

Claude remote Provider 复用同一 Provider Gateway 单向边界。它的 CLI `system/init`、text delta、result 和 instance/turn 状态只能形成 Provider v1 event；Provider mapper 不把这些 event 归一化为 `PetEvent`。Provider 启动的 CLI turn 继承本机 Claude settings、MCP、Hook 与 plugin；若用户配置了 Code Pet Claude Hook，该 Hook 可以像普通 Claude session 一样独立写入旧 collector。这是本机 Hook 配置的预期行为，不是 Provider v1 event 跨入 Pet 链。

## 归一化

`src-tauri/src/activity/events.rs` 将原始 payload 转换为 `PetEvent`：

- provider、kind、status、title、message、session id、cwd、tool name、source metadata 和 ring 标记。
- Cursor 事件名会被规范化为共享事件词表。
- 空闲通知里的模板化噪声会被抑制。
- 失败信号可以把终态通知转换为失败任务事件。

## 增强与存储

`src-tauri/src/activity/title_resolver.rs` 改善任务标题。`src-tauri/src/app/state.rs` 保存近期事件、限制前端输出数量，并跟踪待审批状态。

## 前端归并

`frontend/lib/activity.ts` 按 provider 加 session 或 cwd 分组，过滤内部/后台事件，隐藏用户配置命中的过滤项，丢弃过期 active work，并只保留属于可见活动的终态卡片。

用户过滤配置按 Agent 生效：`activityFilters.byAgent.<agent>` 只影响同 provider 的事件。旧顶层过滤字段只作为兼容输入，不应作为新 UI 的写入目标。`frontend/PetApp.svelte` 会缓存最近事件；收到 `settings-updated` 后从缓存重建可见任务列表，让新增或移除过滤条件在不重启应用的情况下反映到已显示列表。

## 风险

- 修改事件 identity 可能把无关任务合并，或把一个任务拆成多个卡片。
- 修改终态事件处理可能重新引入孤立完成卡片。
- 过滤逻辑必须在增量批次中保留隐藏 key，否则已过滤的后台任务可能重新出现。
- 设置变更时如果只从当前可见列表删除匹配项，取消过滤后旧任务不会恢复；需要从近期事件缓存重建列表。
- 合并 remote 与 companion bus、复用同一个 Tauri event 或让 PetApp 枚举 remote Provider，会重新引入跨来源污染；双 transport 测试和前端静态测试必须持续守护。

## 验证

- 运行 `npx vitest run frontend/lib/activity.test.ts`。
- 运行 `src-tauri/tests/event_normalizer_tests.rs` 下相关 Rust 归一化测试。
