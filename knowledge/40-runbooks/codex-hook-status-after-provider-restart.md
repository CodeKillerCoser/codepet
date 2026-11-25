# Codex 重启后 Hook 运行状态缺失与手机被动补拉

## 现象与证据

2026-11-25 用户反馈原生 App 中有两个运行中的任务，Codepad 显示空闲。时间均为 Asia/Shanghai。

- Host PID 68655 于 05:48:07 启动，Codex Provider PID 68710 于 05:48:11 启动。真实 hooks.json 已安装托管命令，endpoint 对应端口确由 Provider 监听。
- `events.jsonl` 中任务 `01a2092b-218a-7010-bf11-2c38a134fb5c`、`01a20e7c-6f55-7b03-ae1d-e1d410fe7ad1` 分别在 05:44:52、05:45:57 收到 running 开始事实，均早于本次 Provider 启动。重启后内容提示持续到达，摘要却仍为 idle。
- 本次排查所在会话在 05:59:27、06:03:00 收到新开始事实后，摘要与模拟器均正常显示运行。证明通知链路可用，不能归因为项目 schema 兼容改动破坏了 Hook。
- 源码还显示：Hook 通常发布摘要，Host 将最近快照失效，每 250ms 合并通知；Remote 原实现收到失效就从第一页刷新，并恢复已加载深度。被动详情收到空 item 也会补拉最新消息。因此密集 Hook 存在反复查询风险。

## 根因与用户澄清

Provider 用 `ActivityProjection::default()` 建立内存事实。旧实现只允许 start/stop 建立活动，工具和审批只能更新已知活动会话。进程重启清空事实后，后续有效 Hook 仍被这个前置条件阻挡。单纯“投影在内存”不是必须增加持久化的理由：用户明确后续事实应当能恢复未知状态。

同一 Provider 内 App Server 启停不清空投影；整个进程退出会清空。但新进程必须能处理晚加入后收到的工具、审批事实。最早引入该限制的提交尚未追溯。

2026-11-25 用户同时要求：被动观察先不主动查询，保留手动刷新。这取代旧规约中“工具不能建立未知活动”和“空 item 自动补拉”的限制，不引入历史重放、持久化或 loaded-list 对账。

## 修复与涉及模块

- `provider/hook_observation.rs`：未知状态收到工具/审批 Hook 即建立 running/waiting；相同状态不重复发布 ActiveChanged。已知 stop 后仍需新 start，防止迟到工具复活。原生摘要不能覆盖已知 Hook 事实。
- `provider_vertical/shared_execution.rs`：合法空 item 提示不能在正文断言中直接 unwrap；保留延迟正文、writer 生命周期和终态转发断言。
- Remote `recent_conversation_controller.dart`：失效通知只废弃 cursor，不发请求；已加载行按完整路由接收摘要更新，保留 Host 成员和顺序。查询期间实时摘要覆盖较旧快照；手动刷新单飞，继续原分页/锚点逻辑。
- Remote `conversation_message_cache.dart`、detail controller：空 item 不触发补拉；手动刷新只读最新一页，合并正文，保留旧页、cursor、实时 fence，期间的较新正文优先。失败保留已有内容。
- Remote 首页与详情：分别提供“刷新最近会话”和“刷新会话”，读取中禁用。初次打开、连接恢复、用户分页等原有加载仍保留；不修改 Provider/Gateway 协议及原生项目归属。

## 验证结果

- Codex `--lib`：89 通过、1 个外部测试忽略；其中 Hook 6 项覆盖新实例从工具/审批恢复 running/waiting、重复状态去重、结束后不复活、原生 idle 不覆盖。
- Codex `provider_vertical -- --test-threads=1`：最终在隔离临时 CODEX_HOME 全量 76 通过、1 忽略。初次真实目录测试碰到合法空 item 的旧 unwrap；修正后首轮隔离跑遇到饱和 EOF 压力测试退出码不稳定，最终完整重跑通过，未修改该压力测试。
- Remote `recent_conversation_controller_test`、`home_recent_conversations_test`、`conversation_message_cache_test`：29 项通过，包含 100 次通知零额外查询、手动刷新单飞、实时状态不被快照覆盖、手动消息合并与失败保留。
- Remote detail screen、home screen、DeviceSession：132 项通过，覆盖既有布局、焦点、发送与事件交互；8 个变动文件静态分析无问题。`flutter build apk --debug --no-pub` 成功。
- `npm run check:source-size`、两仓库 `git diff --check` 通过。Remote 首页 1763 行，沿用现有 SectionTitle.action 放置按钮，不扩改页面职责。
- 本地 Android API36 模拟器 `emulator-5554` 成功覆盖安装调试包，保留配对。Provider 独立替换并签名验证后终止旧进程；验证期间 Host 于 06:20:57、06:24:46 退出，随后发现安装目录中的 Provider 指纹也被另一构建覆盖。最终只补回修复版 Provider，保留 Host 其他改动，重新打开 App 加载；最终 Host/Provider 为 80483/81446，启动时间 06:30:21/06:30:23；最终三条原有会话于 06:30:30/37/43 收到 running 事实，模拟器同时显示原有两个任务运行中。修复版指纹与首轮实测版本一致，记录在 install.json；不将外部整包覆盖误记为本次 Hook 逻辑回归。
- 真实任务在 06:21:21、06:23:20 分别从新进程收到的后续 Hook 恢复 running，无需发送新用户 prompt。模拟器最近及详情均显示“梳理 codepet-task 任务归并”运行中；被动详情仍保留“另一个客户端写入”的正常限制，没有强制接管。
- 最近列表手动刷新一次后，手机诊断日志从基线到后续观察只增加一次 conversation.recent，没有被动重复 recent/list。进入详情只发生正常首次 get，手动消息刷新增加一次 get；在 Host 仍在线的 06:24:01—06:24:38 区间没有被动额外 get/recent/list；后续离线区间不能算在线无请求证据。脱敏 RPC 统计、事件状态、安装指纹与截图在 `artifacts/codex-hook-status-20261125/`。

## 回归边界与风险

没有新 Hook 的期间仍无法即时知道未知状态；不通过独立 App Server 编造活动。已知 stop 后迟到工具不复活，用 stop/工具乱序测试保护。被动事件只改已加载行，新成员、排序及过期 cursor 由用户手动刷新恢复；失效后不自动延续旧 cursor。手动刷新与实时正文并发，用 canonical 优先和版本失效测试保护。实体手机新包、Windows 原生运行未实测，不能用模拟器替代。

## 排查顺序

比较 Provider 进程启动时间与目标会话的 Hook；确认 endpoint 实际监听进程；查看 active 事件、空 item 提示、摘要状态；最后核对 Remote。已收到工具 Hook 却一直未知时先查投影前置条件；新事实也未到达时再查 Hook 安装、信任、字段与发布错误。观察请求次数时区分初次加载、手动刷新及被动事件，不把全部网络流量当成列表补拉。

规约见 [Hook 状态与被动更新](../60-rules/codex-hook-activity-and-content-updates.md)、[Remote 消息数据源](../60-rules/remote-conversation-message-sources.md)。
