# Provider 活动订阅与 Remote / Pet 双 Gateway

## 背景

2026-09-06：桌宠需要感知用户从不同入口运行的任务。Provider 自己启动的 App Server 只能证明覆盖该连接，不能当作全局桌宠数据源。旧 Hook 又把通知、工具失败、子代理结束混入主任务终态，证据见 [Hook 排查](../30-domains/agent-events/hook-noise-investigation.md)。本文记录本次实现，替代此前的 v2 协议提案。

## 目标

- Provider 安装固定活动 Hook / 原生插件，把通知送到 Host；Host 按显式订阅投递。
- Pet 前端直接调用 Pet Gateway，只显示任务；Remote 客户端保持直连 Remote Gateway。
- 支持 Codex、Claude Code 和 OpenCode 正式版（实测基线 `1.18.25`）；不提供事件类别、matcher 或安装范围选择。
- 保持现有协议版本与 mux 传输。新增方法和 DTO 仍在 v1 IDL 中生成。

## 非目标

不迁移主窗口事件历史、桌宠回复、停止、审批决策、原窗口激活及机器人通知；不新增 Qoder / Cursor。Remote 现有控制能力继续保留。旧功能的模块和历史测试不在本次全面删除，生产桌宠不再调用它们。

## 现状理解

`PluginManager` 继续拥有插件进程和 IPC。`ProviderGatewayService` 继续处理远程实例及业务事件。观察订阅与 Remote heartbeat 驱动的 `instance.start/stop` 分开，启用来源不启动受控 harness。

应用启动已断开旧 `collector`、spool 回放和 Codex Desktop Companion。PetApp 不再读取 Companion snapshot/replay 或发起动作。主窗口 Agent 页改为 Pet 来源状态，主窗口旧事件页不接收新来源。旧 Hook 安装/状态 CLI 明确报停用，避免绕过 Provider 再注入旧脚本。

## 实现路径

```text
Pet UI ── pet_gateway_request / Pet v1 ── PetGateway（来源状态、任务归并）
                                            ↑ 本地订阅
Remote UI ── Remote Gateway ── 原有受控业务链  │
                              PluginManager 订阅登记表
                                            ↑ 单条 Host 订阅
                                Provider event.notification
                                            ↑
                  Codex / Claude Hook 或 OpenCode 原生插件
```

### 订阅和协议

Provider v1 增加 `event.subscribe`、`event.unsubscribe` 和 `event.notification`。通知携带 Host 订阅 ID、事件 ID、接收时间和原始 JSON；Provider 不构造 PetTask，也不知道消费者属于哪个 Gateway。

Host 每个 Provider generation 只建立一条上游订阅，本地多个消费者各有 256 条有界队列。登记首个消费者后订阅 Provider，最后一个取消时退订；按进程身份、generation 和 wire 订阅 ID 校验。慢消费者被单独断开，其 Gateway 可标记缺口并重连。通知不会进入 Remote 的业务路由验证、cursor 或 replay，登记表不解析任务内容来选择 Gateway。

Pet v1 增加来源启停、来源健康和 `waiting-input` / `unknown` 状态；沿用快照，前端每秒读取一个原子快照。来源设置保存到应用数据目录 `pet-sources.json`，默认启用三个来源。已有 `pet.action` wire 定义保留，但本期 Gateway 返回未实现，UI 不暴露动作。没有新增 v2，没有实现历史回放或持久任务存储。

### Provider 接入

- Codex：用户配置目录的 `hooks.json`；固定会话、提交提示、工具前后、权限、Stop、Interrupt 和子代理事件。
- Claude：`settings.json`；在上述语义基础上包含工具失败、StopFailure、Elicitation / Result。普通 Notification 不订阅。
- OpenCode：全局配置目录 `plugins/codepet-observation.ts` 自动发现；不改用户 JSON / JSONC。使用正式版 `Plugin` 函数返回的 `event({ event })` / `dispose`，按 `session.status`、`session.idle`、`session.error` 和 permission / question 事件投递。已有任务的元数据通过正式 SDK `client.session.get({ path: { id } })` 的 `data` 获取，不依赖 Beta `setup(ctx)`、`ctx.event.subscribe` 或 `ctx.session.wait`。Host 按原生 event ID 去重；失败/中断后的 idle 不覆盖终态，未开始任务的 idle 不制造任务。

共用 `codepet-observation` 只处理文件、互斥和投递。首次订阅安装固定桥接资源，托管项更新保留用户自定义 handlers；配置符号链接指向原文件更新。按实际配置文件加跨进程锁，防止两个 CodePet 抢占同一来源。Codex / Claude 使用探测到的 Node 绝对路径，原生 Hook 不输出任何权限决定。

接收器只监听 `127.0.0.1` 随机端口，令牌保存在私有 endpoint 文件。单条 HTTP 负载限制 256 KiB，脚本限制约 250 KB、1 秒网络等待、1.5 秒进程上限，离线也以成功结束。OpenCode 队列上限 128，后续通知携带投递缺口。最后退订或退出关闭接收器并删除 endpoint；托管入口保留，避免每次启动改变需要信任的配置。

### 状态归并

任务以 `provider + 原生 session` 为键；前端只看到标题、摘要、来源、状态、时间和可用目录/工具名。最多保留 120 个任务、2048 个去重事件 ID，每个任务记住最近 16 个已替换轮次。

工具失败仍表示主任务在运行；子代理事件和子代理会话不能结束父任务。普通通知、SessionStart 和 OpenCode session.created 不单独制造任务；等待输入/授权只展示状态。旧时间、旧轮次以及已终结轮次的普通工具事件不能覆盖当前状态。Stop 表示本轮结束；连接中断、来源关闭和可检测的事件缺口使未终结状态变成“待确认”。缺少原生 turn ID 时只能按到达证据推断，不能承诺严格还原迟到事件。

## 涉及模块

- `protocol/{provider,pet}/v1` 与生成 SDK：现有版本上的订阅和只读快照；新增 Pet TypeScript 输出供 UI 使用。
- `crates/providers/codepet-observation` 与三个 Provider：安装、投递、独立观察生命周期；harness 差异留在 Provider。
- `crates/codepet-host/src/providers/manager/subscriptions.rs`：共享 Host 订阅登记与有界分发；不判断 Gateway 业务。
- `crates/codepet-host/src/pet_gateway`：来源健康、偏好和任务投影。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs`、`lib.rs`：薄 Pet RPC 入口与启动/退出；断开旧采集入口。
- `frontend/PetApp.svelte`、`PetSources.svelte`、`petGateway.ts`：只读列表、来源状态及启停；移除 Hook 范围 UI / 命令和偏好写入。

## 风险

- 安装成功不等于实际生效：UI 区分等待首次活动和正在接收；信任仍由原应用处理。用真实 harness 新任务验证。
- OpenCode API 版本差异：以正式 tag `v1.18.25` 的 `packages/opencode/src/plugin/index.ts`、`packages/plugin/src/index.ts`、`session/status.ts` 和 `session/run-state.ts` 为依据。本机同版本二进制在正常正式版入口投递事件；先前只测试 `/api/*` 路径和 Beta 插件，错误地推断正式版无法支持，已移除该实现及升级提示。Remote 自有实例的 `/api/*` 路径与精确版本约束保持原状，不能将其插件加载路径代替普通 CLI 的正式插件验收。
- 观察源丢失最终事件：本期不补读历史，不把沉默当完成；通过断连/缺口显示未知。强制杀死 harness 且无终态通知仍可能留下上次状态，需后续增加经过验证的存活证据。
- 改坏配置或 Gateway 串线：通过重复安装、用户 handler 保留、多进程锁、多个独立 Hook 进程投递和 Host 多订阅测试验证。

## 测试计划与结果

- `node --test crates/providers/codepet-provider-opencode/tests/observation-plugin.test.mjs`：3 项通过，覆盖正式回调、噪声过滤、SDK 元数据、投递失败缺口和 dispose；Pet projection 的 4 项测试覆盖来源区分、权限/输入等待、失败/中断后的 idle 和下一轮。
- `python3 scripts/test_opencode_observation.py --executable /opt/homebrew/bin/opencode`：2026-09-06 在正式版 `1.18.25` 通过。隔离 XDG 目录、固定本地模拟模型、两个独立 `opencode serve` 进程，实际验证成功任务、permission asked/replied、失败后 idle，以及全局插件跨进程投递；没有调用真实模型服务或修改用户安装/配置。脚本要求精确正式版本并清理进程。
- 正式版适配后重新执行 `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-opencode -p codepet-observation -p codepet-host -- --test-threads=1` 全部通过；`npm run providers:stage:dev` 已刷新内置 Provider 资源。此修正未改前端布局或协议，未重复执行前端验收。

- `npm run protocol:check`：IDL、fixtures、生成新鲜度。
- `npx vitest run frontend`、`npm run build`：前端回归及生产构建。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-observation -p codepet-host -p codepet-provider-codex -p codepet-provider-claude -p codepet-provider-opencode -- --test-threads=1`：安装、真实 Node 投递、归并、无 Remote instance 的 Host 多订阅，以及原有控制回归。
- `cargo test --manifest-path sdk/rust/Cargo.toml -p codepet-provider-sdk -p codepet-pet-sdk` 通过；`cargo test --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=1` 的 72 项测试通过。前端 173 项测试与构建通过。全库 `npx tsc --noEmit` 仍被既有 Node 类型、旧测试 fixture 等错误阻断，不能声称全库类型检查通过。
- 首轮 Host 并发测试曾出现既有进程测试的文件描述符计数断言波动；串行执行完整 Host / Provider 回归后通过。
- Playwright 使用 Tauri mock 验证 360px 桌宠、1040px / 390px 来源卡片；检查长文本、折叠、隐藏后轮询、来源启停、键盘焦点和横向溢出。实际 macOS 原生窗口拖拽与穿透沿用旧逻辑，本轮未运行生产 App 修改用户配置。

## 知识沉淀

[协议分层规约](../60-rules/protocol-layer-and-channel-boundaries.md) 和 [Codex 通道隔离规约](../60-rules/codex-provider-channel-isolation.md) 已更新到“控制事件与显式观察订阅隔离”。旧 Companion 设计文档保留为历史，不再作为生产桌宠的数据源要求。

[Harness 正式版 API 验证规约](../60-rules/harness-release-api-validation.md) 记录发布通道与实际入口的验收约束，避免将 Beta 测试代替正式版支持。

## 未知项

Codex / Claude 的真实交互任务（含信任、审批、取消）尚需在用户实际版本上验收；本轮验证了安装和桥接链路，未使用模型执行真实任务。Windows 未实机验证。OpenCode 正式版 `1.18.25` 已验证原生插件和任务流程；其他正式版本、桌面客户端及 question 的真实交互尚未实机验证，不承诺未经验证的 Beta API 兼容性。
