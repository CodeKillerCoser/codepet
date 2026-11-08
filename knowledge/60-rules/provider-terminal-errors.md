# Provider 执行失败必须保留原因

## 规则

- 区分请求拒绝与已接受 turn 的执行失败。前者返回 Core `ProtocolError`；后者通过 Agent `TurnTask.error` 随 failed 终态发布。不得把已经返回成功的发送 RPC 当作执行成功，也不得等待它再次返回错误。
- `TurnTask.error` 复用 `code/message/retryable/details`。只对 failed 终态设置；同步填充 `displaySummary` 供摘要消费者使用。缺失原生原因时提供明确的未知原因提示，不能只发 failed。
- 原生错误解析属于各自 Provider。只提取必要的错误分类、消息和已知诊断字段；禁止无条件把完整响应、凭据、请求头或原始错误对象放进 details。消息应有长度边界。
- 失败原因不依赖是否已输出正文。先有流式输出、后额度耗尽或网络失败，仍要保留部分输出并单独呈现失败原因。
- 可恢复的中间错误、工具失败、审批拒绝、用户中断与整轮失败分别处理。Codex 的中间 error 通知等待终态判定，不能提前关闭 turn 或污染后续成功状态。
- `retryable=true` 仅表示用户发起新的尝试可能成功，绝不授权自动重发已接受的输入；执行可能已经有副作用。
- 实时事件与回读都应可见。Host 持久化其观察到的失败终态，按 Provider、会话和 turn 身份隔离；重复/晚到事件不能清掉更完整的同版本错误。会话删除时清理记录。
- Gateway `conversation.get.turns` 在初始页返回最近最多 100 条观察到的失败终态，按时间升序。它补充原生历史未保存的失败尝试，不是所有历史 turn 的分页接口；更早记录及修复前未记录的失败不承诺补全。
- Remote 对回读 turn 校验会话和 Provider 路由，按 turn 身份合并事件及快照；无正文失败也显示独立错误卡片，部分正文继续保留，旧 Provider 可回退到 displaySummary 或明确的缺失原因提示。
- 协议从 `protocol/` 修改并生成 SDK。新增可选字段允许新 SDK 读取旧 payload，但严格旧 SDK 不一定接受新字段；Host、Provider 与 Remote 必须协调升级，不能把 OpenCode 1.x/2.x 原生兼容与 SDK wire 兼容混为一谈。

## 回归验证

覆盖请求直接失败、无正文失败、部分正文后失败、中间错误后成功、进程异常退出、重复/晚到事件、无消息历史回读、数据库重新打开、跨 Provider 路由、会话删除、长文本与窄屏大字号。错误处理后还须验证后续请求与审批正常，不能误关共享实例。

实现和本机验证记录见 [Provider 错误链路排查与修复](../40-runbooks/provider-turn-error-propagation.md)。
