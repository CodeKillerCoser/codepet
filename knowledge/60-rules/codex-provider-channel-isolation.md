# Codex Provider 与 Desktop Companion 必须按 Channel 隔离

## 规则

Codex Provider/App Server 远程链路与 Codex Desktop Companion/私有 IPC 桌宠链路必须完全独立。两者不得共享 session、registry、event bus、replay、activity projection、来源排除状态或动作路由。remote 与 Desktop 出现相同 native thread id 时允许各自保留；本阶段不做去重、排除或同步。

Provider 事件只能进入 `ProviderGatewayService` 的 cursor/replay 与 `runtime-gateway-event`。它不得：

- 写 `CodexThreadScope` 或调用 `CodexDesktopCompanionAdapter::exclude_remote_thread`；
- 发 `codex-desktop-companion-event` 或 `pet-event`；
- 修改 `SharedState` activity、PetProjection 或 companion replay；
- 让桌宠动作调用 Plugin Manager、Provider Protocol 或 App Server。

桌宠 Codex 数据的唯一来源仍是：Codex Desktop IPC → Desktop Companion → Pet Protocol/compat projection → Pet UI。Provider 故障不得 fallback 到 Desktop IPC，Desktop IPC 故障也不得 fallback 到 Provider。

## 证据边界

- `RuntimeGatewayState` 只持有 `CompatProviderGateway`；compat 只调用 `ProviderGatewayService`，不持有 Pet/companion/scope 状态。
- `TurnOutputDeltaEvent` 在 Provider/Gateway v2 中自带 conversation 四段 route，因此 compat 不需要 turn replay map。
- Provider resource 直接携带 `deviceId + providerPluginId + providerInstanceId + nativeResourceId`；compat 不通过 registry 回查 plugin id。
- 生产 `lib.rs` 分别构造 Provider Host/Gateway 和 Desktop Companion；Provider state 不接收 companion state。

## 适用场景

- 修改 Runtime Gateway state、Tauri bridge、Provider registry 或 event stream。
- 修改 PetApp 初始化、snapshot/replay/live event 或 activity projection。
- 修改 App Server create/turn/approval，或 Desktop IPC bootstrap/action。
- 修改 executable refresh、Provider unavailable 或恢复策略。

## 反例

- Provider conversation/event 到达后调用 `mark_remote`，再删除 Desktop 卡片。
- compat 为 delta 保存 `turn -> conversation` map，依赖早先 replay 来补路由。
- PetApp 枚举 remote provider、监听 `runtime-gateway-event`，再在 UI 隐藏不想展示的卡片。
- Desktop IPC unavailable 时把 App Server snapshot 填进 companion store。
- Provider refresh 重启 Desktop IPC 或清空其 projection。
- 为避免同名 thread 而建立跨链路 tombstone、quarantine 或 transient operation fence。

## 推荐做法

- remote 使用 `runtime_gateway_*` 与 `runtime-gateway-event`；companion 使用 `codex_desktop_companion_*` 与 `codex-desktop-companion-event`。
- Provider/Gateway event 自身携带完整 route；每一跳校验身份，不保存兼容层关联状态。
- PetApp 只导入 companion client。交互 capability 同时校验 Desktop namespace/source marker。
- 无法无损映射的 App Server request 以原 id 返回上游 error，不发布可操作 Approval。
- runtime refresh 只替换 Host Codex instance setting 并显式 restart Provider；任何故障只改变 remote Provider。

## 风险与验证

- Provider 污染桌宠：Tauri mock runtime 使用生产 bridge。真实 Provider fixture 产生正常事件、坏帧和 crash 后，只允许 remote channel/replay 有数据；companion replay/event、activity store、Desktop adapter spy 与 pet event 计数必须不变。
- Desktop 数据源被替换：Desktop IPC adapter 测试继续覆盖 bootstrap/snapshot/patch/owner/action；PetApp 静态测试只允许 companion snapshot/replay/event/request。
- 路由状态回流：检查生产源码不包含 `CodexThreadScope`/`mark_remote` 或 companion 写入口，compat 不包含 `turn_conversations` 或 plugin registry lookup；四段 route 的 schema/generated/Host tests 必须通过。
- 生命周期串扰：分别让 Provider 与 Desktop IPC unavailable，断言另一条链路不重启、不清空且仍使用自己的 transport。

## 来源

- `../10-architecture/codex-provider-plugin-runtime.md`
- `../50-decisions/codex-remote-and-desktop-companion-dual-channel.md`
- `../30-domains/agent-control/codex-app-server.md`
- `../30-domains/agent-control/codex-desktop-companion.md`
