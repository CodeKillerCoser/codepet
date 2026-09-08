# Agent 活动感知接入参考

## 当前方案

当前本地活动感知由 Provider 安装 Hook／原生插件，通知经过 Host 订阅分发到 Pet Gateway。生产启动链路已断开旧 collector、spool 回放和 Codex Desktop Companion；保留的旧代码与历史文档不是当前接入方式。

| 来源 | 配置入口 | 方式 |
| --- | --- | --- |
| Codex | 用户配置目录的 `hooks.json` | Provider 安装固定 Hook，观察会话、提示提交、工具、权限、Stop/Interrupt 等事件 |
| Claude Code | 用户配置目录的 `settings.json` | Provider 安装固定 Hook，观察任务与输入/授权状态 |
| OpenCode | 全局配置目录的 `plugins/codepet-observation.ts` | 原生插件投递 session、permission、question 等事件 |

当前 Pet 来源为上述三项，不将旧 Qoder / Cursor Hook 配置表当作新的来源支持列表。Codex / Claude Hook 使用探测到的 Node 绝对路径，原生 Hook 不输出权限决定。

## 数据链路与边界

Hook／原生插件 → Provider `event.notification` → Host 订阅登记表 → Pet Gateway → 只读桌宠任务列表。

Provider 负责工具适配，不构造 PetTask，也不判断消费者属于哪个 Gateway。Host 管理共享上游订阅和各消费者的有界队列，Pet Gateway 归并任务；观察通知不混入 Remote 业务事件的 cursor/replay。

开启活动来源不启动受控 Harness，不获取 Codex 会话写锁。任务列表展示等待授权／输入等状态，但当前不提供桌宠回复、停止或审批动作。Remote 继续对话走独立控制链路，需满足 [会话锁与交互权](../20-product/remote-control-and-plugins.md#codex-会话锁与继续对话) 约束。

## 接收与设置

接收器只监听 `127.0.0.1` 的随机端口，令牌保存在私有 endpoint 文件。最后退订或退出关闭接收器并删除 endpoint；托管入口保留。不要再用旧固定 `47621/hook` 地址或 Desktop socket 验证当前来源。

来源状态与启停在主窗口 Agent 页管理，偏好保存在应用数据目录的 `pet-sources.json`。安装托管项会保留用户自定义 handlers；原应用所需的 Hook／插件信任仍由用户在原应用处理。

## 维护依据与验证

- [Provider 活动订阅与 Remote / Pet 双 Gateway](provider-hook-observation-proposal.md)：当前实现及验证记录。
- `crates/providers/codepet-observation`：共享安装、接收、互斥与投递。
- `crates/codepet-host/src/providers/manager/subscriptions.rs`：Host 订阅隔离。
- `crates/codepet-host/src/pet_gateway`：任务投影与来源状态。
- `frontend/PetApp.svelte`、`PetSources.svelte`、`petGateway.ts`：只读任务与 Pet 协议入口。

检查安装后“等待首次活动”到“正在接收”的变化，并在真实工具中运行一次任务；确认工具失败和子代理结束不会错误结束父任务。首次安装成功不能替代实际投递验证。本次为文档修正，未执行真实工具联调。
