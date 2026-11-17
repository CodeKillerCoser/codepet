# Codex 最近会话：有界补读与批量状态处理

这是第一阶段记录；后续已增加可选有序 IDs 协议和按页补对象，当前增量及保守回退边界见 [有序高优先级分页](recent-ordered-attention-pagination.md)。

## 现象与证据

2026-11-17，[故障取证](codex-recent-timeout-20261116.md) 确认 Host 首屏 20 条会触发 1506 次 metadata read，其中 1278 次为精确的 thread-not-loaded；实时栈中 Provider 持续等待原生 read，原生持续遍历历史目录。历史补漏和体验边界见 [查询审计](recent-first-page-query-audit.md)。原故障机、隔离快照和本机合成实验的环境不同，性能数字不能混用。

本机真实 Codex、独立临时 home、5040 个空合成 rollout 文件名、96 个不存在的随机 ID：两轮串行耗时 3713/3803ms，两轮 8 路并发耗时 784/778ms；全部仍返回原生 -32600。未读取真实历史、发送 prompt 或控制用户任务。探针为 `/Users/wangxin/Documents/CodePet-builds/recent-root-cause-20261117/native-concurrency-probe.py`，结果为同目录 `native-concurrency-probe.json`。这证明当前原生能从并发受益，不保证所有机器固定提速，也不是整条 recent 的性能验收。

## 根因与引入历史

按页目录补漏的逐个 await，在 Host 全量活动/未读 IDs 查询下放大成串行目录查找；Host 又逐行调用本已支持批处理的共享阅读状态层。建立摘要基线引起的重试会重复此前已完成的查询。最初写入这些行为的全部提交未逐项追溯；不能把双源补漏本身认定为错误。

## 修复与涉及模块

- Codex `provider/directory_list.rs`：同一实例所有目录查询共享 8 个原生 metadata 名额；重叠的同 generation、事实 epoch、会话 ID 读取共用结果。只合并在途请求，不建立跨查询成功/失败缓存。保留原生正式对象、精确错误判断、DB 时间修补、项目映射、候选去重、已访问页重放及成功后提交游标。每个 list 页总预算 20 秒。
- Host `conversation_state.rs`：同 Provider 的摘要直接整批接纳/装饰；混合 Provider 按库分批后恢复原顺序。避免每行重新打开事务、加载整份状态，保留 readerScope 与各 Provider 数据隔离。
- Host `gateway/recent.rs`：整次查询含构建锁、容量等待、枚举及重试最多 25 秒；超时返回 `recent_query_timeout`，不返回部分成功页。同一构建中重试可复用成功查询及其明确落空结果，调用结束即释放；真实来源变化使其失效。
- Host `recent_conversations.rs`：区分来源变化与本轮阅读基线引起的视图失效；两者仍使快照失效，但只有前者丢弃构建内 metadata。自己的 recentChanged 提示不触发重新扫描，无关 Provider 不影响该构建。

取消会丢弃尚未提交的页状态并停止等待中的任务；已进入阻塞线程的 native RPC 无法立即撤回，继续占用名额直到完成或原生请求超时。不会通过关闭共享 App Server 来取消单条查询。

## 范围、风险与回归防线

本轮是保守修复：**活动/未读仍全量确定成员并获取排序对象，尚未做到只补当前页 20 个对象**。现有 active/unread 协议缺少可靠的最终排序键；在解决此缺口前，不任取 20 个 ID、不用 DB 列替代 App Server 正式对象，也不增加启动/周期全历史扫描或推送。

保留空 preview 补漏、未落库 Hook-only 会话、legacy 时间、项目/standalone、活动与未读优先、14 天日期尾页、readerScope、snapshot fence 和手机滚动/实时消息合并。落空结果只在同一有界构建内复用，来源事件会清空，不能据此持久删除历史。

并发会提高瞬时目录 I/O，因此用实例级名额而非每查询各开 8 路；取消后名额仍属于实际 native 工作。持续来源变化或更大的历史仍可能超过预算，错误必须可见，不能宣称所有负载均成功首屏。

## 验证

- Codex：`cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --lib --test provider_vertical -- --test-threads=1`，84 单元与 68 纵向测试通过，两个原有环境依赖测试忽略。新增真实子进程测试覆盖 100 个 ID、双查询只执行 100 次 native read、8 路上限、阻塞时 project.list 可响应、新请求重新读取、缺失不持久缓存、取消不释放仍被 native 占用的名额。
- Host：`cargo test --manifest-path crates/Cargo.toml -p codepet-host --lib --test manager_gateway -- --test-threads=1`，109 单元与 55 集成测试通过。排除启动来源事件干扰的用例确认：构建内基线重试用两个 IDs 批次即可完成 125 条活动聚合；有真实来源事件时仍允许重新查询。现有集成测试继续覆盖事件失效、scope、全量翻页、普通列表隔离与日期按需续页。新增批量状态测试覆盖混合 Provider 顺序、同名 ID 隔离、独立阅读进度及新基线不自失效；总预算测试验证 future 被丢弃且业务错误不被改写。
- 手机：`flutter test --no-pub --concurrency=1 test/application/recent_conversation_controller_test.dart test/home/recent_conversation_feed_test.dart test/conversations/conversation_message_cache_test.dart`，30 项通过。首次运行的 Flutter tester 本机 HTTP 握手被代理环境干扰；对测试子进程清除代理并设置 localhost/127.0.0.1/::1 的 NO_PROXY 后通过，未改手机源码或系统代理配置。
- `cargo check --manifest-path crates/Cargo.toml -p codepet-host -p codepet-provider-codex --tests` 通过，仅有既存 `RecentSnapshots::page` dead-code warning。

`git diff --check` 与本轮文档相对链接校验通过。测试和只读原生对照共完成；本轮没有修改协议/生成物和手机源码，没有安装或重启用户 App，原故障设备的新包端到端验收仍未完成。
