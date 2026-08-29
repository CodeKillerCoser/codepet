# Codex Remote 与 Desktop Companion 必须按 Channel 隔离

## 规则

Codex Remote/AppServer 与 Codex Desktop Companion/IPC 必须拥有独立 Provider registry、Gateway event bus、sequence/replay、LocalTransport、Tauri command/event 和 unavailable 生命周期。桌宠 activity store 只能消费 companion channel；remote 标准事件不能因 provider id 相同而进入桌宠。

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

- `RuntimeGatewayState` 只注册 remote App Server；`CodexDesktopCompanionState` 只注册 Desktop IPC。
- remote 使用 `runtime_gateway_*` 与 `runtime-gateway-event`；companion 使用 `codex_desktop_companion_*` 与 `codex-desktop-companion-event`。
- PetApp 只导入 companion client。交互 capability 同时校验 Desktop namespace/source marker。
- remote create 期间 quarantine 新 Desktop thread；成功/notification 标记 remote，歧义失败保守排除，明确未派发才释放本地候选。
- 历史 remote 动作先以 transient fence 阻止 Desktop 本地派发；resume/dispatch 成功或结果歧义才永久标记 remote，明确 unknown/not-loaded/未派发拒绝必须释放为 local。
- App Server loaded-thread 状态必须按 session 隔离；list/read 不等于 loaded，新 session 首次运行时动作必须重新 `thread/resume`。
- 无法无损映射的 server request 不得发布为可操作 Approval；必须按官方 response schema fail closed。未知 method 必须以同一 request id 返回 JSON-RPC error。
- projection 对 excluded conversation 保存进程内 tombstone，过滤 snapshot 和全部 conversation-scoped event。
- App Server 后台初始化和 refresh 使用 timeout、generation fence、paused replacement activation；任何故障只改变 remote Provider。

## 来源

- `../50-decisions/codex-remote-and-desktop-companion-dual-channel.md`
- `../30-domains/agent-control/codex-app-server.md`
- `../30-domains/agent-control/codex-desktop-companion.md`

## 验证方式

- 向 remote sink 发布 conversation、turn、approval，断言 companion replay 为空。
- 分别把 remote/companion 置 unavailable，断言另一侧仍可请求或保留状态。
- remote list/create 路由 App Server；companion 不广告 list/create。
- Desktop snapshot 仍驱动 running、approval 和 terminal activity。
- create race、notification、ambiguous outcome、并发本地 candidate 和 equal-revision quarantine 测试。
- list/read → resume → action、resume 明确失败/歧义结果、新 session 重载与 remote-operation fence 测试。
- permissions 空 grant、未知 request `-32601`、无 Approval event 和 capability metadata 一致性测试。
- PetApp 静态测试断言只监听/调用 companion；projection 测试断言 tombstone 拒绝 snapshot、conversation、turn、approval 和 output replay。
