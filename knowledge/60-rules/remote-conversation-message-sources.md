# Remote 消息数据源、手动分页与 LRU 缓存

## 规则

会话消息是持续扩充的数据源：首次加载最近一页，上翻由用户手动请求旧页并前插；实时事件追加或原位更新。任务完成、失败、停止通知只更新状态，不能触发完整历史查询或替换消息列表。

## 适用场景

Remote Gateway adapter 的 get/resume、详情 controller、消息模型、设备会话缓存和详情滚动布局。

## 反例

原生 Codex 历史投影可能落后于执行事件。终态时重新 get 并替换列表会清除已经收到的新回复；首屏沿游标自动拉完也会随会话增长增加传输与内存成本。

## 推荐做法

- 首屏复用 resume 返回的单页和事件窗口 fence。每页初始 limit=20，只在尺寸错误时以相同 cursor 按 10→5→1 重试。
- 较早页按 `(turnId, itemId)` 去重并前插，已观察到的内容、顺序、任务状态和实时窗口保持有效；不能用旧页 snapshotCursor 重设实时 fence。分页单飞，失败保留 cursor，循环 cursor 报错。
- delta 更新同一 item，canonical item 原位替换，跨轮次保留首次观察顺序。终态保留输出并停止流式显示。
- `DeviceSession` 拥有按完整路由身份分隔的缓存；默认容量 8，正在查看的源不因容量/闲置淘汰。进入和离开页面更新访问顺序，后台消息不会提升访问顺序。闲置 15 分钟后可淘汰，每分钟及访问时清理。
- 未淘汰的数据源继续接收事件，返回页面复用消息并重新获取交互权；淘汰关闭窗口，重新访问才加载首屏。Host 连接失效清空缓存，Provider generation/revision/availability 变化定向失效。
- 缓存数不是单会话字节上限。大且持续访问的单会话仍需按实际内存观察；不得通过自动重拉来“整理”数据。用户显式刷新、未知发送结果恢复和连接恢复可以重新获取首屏。

## 来源

用户于 2026-09-06 确认的加载逻辑和经典 LRU 策略；[resume 无更新与历史不完整排查](../40-runbooks/codex-resume-no-output-and-incomplete-history.md)。

## 验证方式

在 `codepet-remote` 执行 `flutter test --no-pub`：269 项通过。重点覆盖：手动分页与实时事件并发、失败重试和循环 cursor、终态零额外 get、canonical 原位更新、跨轮输出顺序、缓存后台事件/复用/淘汰/过期、断线和能力变化，以及 390×844 布局的阅读锚点。最后 Provider 定向失效改动再运行 `flutter test --no-pub test/devices/device_session_test.dart test/conversations/conversation_message_cache_test.dart test/conversations/conversation_detail_screen_test.dart`：87 项通过。相关源码及测试 `dart analyze` 无问题。

未安装手机新包；真实手机长时间挂起、超大单会话的内存峰值仍需设备验证。原生 ordinal 损坏和 warning 未透传问题仍独立存在，缓存失效后的历史读取可能仍旧。
