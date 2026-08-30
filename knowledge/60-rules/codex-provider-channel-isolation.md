# Codex Remote 与 Desktop Companion 必须按 Channel 隔离

## 规则

Codex Remote/Provider Host/App Server 与 Codex Desktop Companion/IPC 必须拥有独立的进程/session、registry、event bus、sequence/replay、Tauri command/event 和 unavailable 生命周期。桌宠 activity store 只能消费 companion channel；remote 标准事件不能因 provider id 相同而进入桌宠。

进程外 Provider Host 是 remote 的唯一运行时：`ProviderHostState` 管理 Plugin Manager 与 Gateway service，compat `RuntimeGatewayState` 只能薄适配这个 service。Provider 事件必须进入远程 `runtime-gateway-event`/replay，但不得进入 `codex-desktop-companion-event`、companion replay、`pet-event` 或 `SharedState` activity。插件失败不得回退到 Desktop IPC，桌宠动作不得调用 Plugin Manager。

两路唯一允许共享的是最小 remote thread provenance 与 transient remote-operation fence。它们只能阻止 companion 投影/动作竞态，不得携带 App Server session、loaded-thread cache、Desktop owner、revision、request router 或原生 payload。

## 适用场景

- 修改 Runtime Gateway state、Tauri bridge、Provider registry 或 event stream。
- 修改 PetApp 初始化、snapshot/replay/live event 或 activity projection。
- 修改 App Server create/turn/approval，或 Desktop IPC bootstrap/action。
- 修改 executable refresh、Provider unavailable 或恢复策略。

## 反例

- 在同一个 registry 注册 AppServer 和 IPC，并依赖不同 provider id 或 UI 过滤。
- PetApp 枚举 remote provider、监听 `runtime-gateway-event`，再隐藏不想显示的卡片。
- Desktop IPC unavailable 时把 App Server snapshot 填进 companion store。
- 只删除 remote thread 当前卡片，不阻止 replay/snapshot 重新创建。
- remote create 响应未确定时先发布 Desktop auto-load 事件，之后再撤下。
- runtime executable refresh 重启 Desktop IPC 或清空其 projection。

## 推荐做法

- `RuntimeGatewayState` 只持有 `ProviderGatewayService` 的 compat 薄适配，不注册 Provider、不启动 App Server；`CodexDesktopCompanionState` 只注册 Desktop IPC。
- `ProviderHostState` 持有 Plugin Manager 与 Gateway v1 service；Manager update 只有一个有界 Gateway consumer。
- remote 使用 `runtime_gateway_*` 与 `runtime-gateway-event`；companion 使用 `codex_desktop_companion_*` 与 `codex-desktop-companion-event`。
- PetApp 只导入 companion client。交互 capability 同时校验 Desktop namespace/source marker。
- remote create 进入 Gateway 后 quarantine 新 Desktop thread；成功/notification 标记 remote，调用返回错误则因标准协议缺少交付细分证据而保守按歧义失败排除。provider lookup 失败发生在 guard 之前，不得影响 Desktop 候选。
- 历史 remote 动作先以 transient fence 阻止 Desktop 本地派发。Provider Protocol 当前没有跨进程携带 Success/NotSent/ExplicitRpcReject/SentOutcomeUnknown 细分证据；compat 对已进入 Gateway 的歧义错误保守标记 remote，不能重新引入 Tauri 内直连来恢复旧证据。若未来需要减少误排除，应先扩展标准协议错误证据并同步生成 SDK。
- App Server loaded-thread 状态必须按 session 隔离；list/read 不等于 loaded，新 session 首次运行时动作必须重新 `thread/resume`。
- 无法无损映射的 server request 不得发布为可操作 Approval。未知或当前不支持的 method 必须以同一 request id 返回 JSON-RPC error，不能伪造空 grant 或成功。
- projection 对 excluded conversation 保存进程内 tombstone，过滤 snapshot 和全部 conversation-scoped event。
- App Server 初始化只发生在 Provider `instance.start`。runtime refresh 只替换 Host instance setting 并显式重启 Codex 插件；任何故障只改变 remote Provider。

## 来源

- `../50-decisions/codex-remote-and-desktop-companion-dual-channel.md`
- `../30-domains/agent-control/codex-app-server.md`
- `../30-domains/agent-control/codex-desktop-companion.md`

## 验证方式

- 向 remote sink 发布 conversation、turn、approval，断言 companion replay 为空。
- 用 Tauri mock runtime 的真实 `AppHandle` 同时监听 `runtime-gateway-event`、`codex-desktop-companion-event` 和 `pet-event`；真实 Provider fixture 触发 event、坏帧和 crash，断言 Provider payload 只进入 remote event/replay，companion replay、activity store、companion/Pet event 与 Desktop adapter 计数 spy 不变。
- 用延迟 initialize/shutdown fixture 并发调用两次 `ProviderHostState::shutdown_once`，断言两次都等待相同完成结果、force kill 后子进程已结束（Unix 额外用 PID 复核），且 Manager shutdown gate 阻止后续 spawn。
- 分别把 remote/companion 置 unavailable，断言另一侧仍可请求或保留状态。
- remote list/create 路由 App Server；companion 不广告 list/create。
- Desktop snapshot 仍驱动 running、approval 和 terminal activity。
- create race、notification、ambiguous outcome、并发本地 candidate 和 equal-revision quarantine 测试。
- list/read → resume → action、真实 `-32600/no rollout`、写前 Shutdown、写后断线、并发失败 waiter、新 session 重载与 conservative remote-operation fence 测试。
- permissions/未知 request `-32601`、无 Approval event 和 capability metadata 一致性测试。
- PetApp 静态测试断言只监听/调用 companion；projection 测试断言 tombstone 拒绝 snapshot、conversation、turn、approval 和 output replay。
