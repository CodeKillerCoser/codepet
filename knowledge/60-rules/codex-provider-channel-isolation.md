# Codex 控制链路与桌宠观察订阅隔离

## 规则

2026-09-06 起，生产桌宠使用 Provider 的固定 Hook 活动订阅。Remote 和 Pet 是两个独立 Gateway：远程受控事件进入 `ProviderGatewayService`；观察通知经 Host 显式订阅登记表交给 `PetGateway`。可以共享进程、SDK 和 IPC，不能共用业务状态、动作路由或历史游标。

Provider 只知道 Host 订阅，不识别 Pet/Remote。Host 登记表只按订阅投递，不解析活动内容来推断目标 Gateway。桌宠不以 Provider 启动的 App Server 作为全局活动证据。旧 Desktop Companion 已从应用启动断开，不作为异常回退来源。

## 适用场景

- 修改 Provider 观察、Host 订阅、Gateway、PetApp 或 Tauri bridge。
- 修改 App Server 控制事件、runtime refresh、Provider 重启或快照同步。

## 反例

- PetApp 监听 `runtime-gateway-event`，再用前端过滤掉不想显示的 Remote 任务。
- 把 Hook 通知投入 Remote 业务事件队列，由资源路由或任务内容决定是否交给 Pet。
- 为了保留回复能力把 Companion action 路由移入新 Pet Gateway。
- 观察订阅依赖 Remote 连接数，关闭远程连接也停止全局活动感知。
- 让子代理 Stop、工具失败或普通通知结束主任务。

## 推荐做法

- Pet UI 调用 `pet_gateway_request`；Remote UI 使用已有 Remote Gateway。前端同时需要两类能力时分别建立 client。
- 每个 Provider generation 共用一条 Host wire 订阅，各本地消费者独立有界队列；最后退订时释放上游，慢消费者只影响自身。
- Provider 转发原始通知并过滤固定事件集合；Pet Gateway 负责任务语义、去重、轮次防倒退和未知状态。
- 来源断连/重启允许影响同一个 Provider 的两类能力，但两个 Gateway 各自恢复自身状态；不建立跨 Gateway tombstone 或来源排除关系。

## 来源

- [Provider 活动订阅与双 Gateway](../10-architecture/provider-hook-observation-proposal.md)
- [Hook 噪声排查](../30-domains/agent-events/hook-noise-investigation.md)
- 旧 [Remote / Companion 双通道决策](../50-decisions/codex-remote-and-desktop-companion-dual-channel.md) 中“控制状态不污染观察状态”的约束继续成立；其中 Companion 作为唯一桌宠来源的结论已由本次用户决策替代。

## 验证方式

`observation_fans_out_locally_without_a_remote_instance` 必须证明多个消费者仅创建一条 Host 上游订阅，无 Remote instance 时仍有桌宠任务，禁用桌宠后其他消费者继续接收。Projection 测试覆盖工具失败、子代理、旧轮次、重复事件和断连。原有 Remote route / cursor / replay 测试继续通过，Pet 前端不得导入 Remote 或 Companion 动作 client。
