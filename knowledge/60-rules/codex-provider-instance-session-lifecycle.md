# Codex Provider 子进程必须先登记再等待

## 证据与结论

observer start 与 `conversation.create` 曾在本地变量中完成 spawn/initialize 后才进入 instance 状态。由此产生两个可复现窗口：stop 看不到已存在但尚未 initialize 的 PID；EOF/shutdown 也无法取消 one-shot create，后者甚至可能在 stop 后继续发送 `thread/start`。另一个独立窗口是异步 Provider event 的 stdout write/flush failure 只返回给后台线程，stdio 主循环仍继续运行。

因此，所有 instance-owned App Server session 必须服从同一条生命周期规则：spawn 前登记 generation 占位，spawn 后立即登记真实 session，任何长 initialize/discovery 都在锁外等待；stop/fail/shutdown 先取消并取走当前 generation 的槽，等 spawn 收敛并回收进程后，才发布 stopped 或完成 shutdown。stdout event 写失败必须进入主循环的唯一 terminal 路径。

## 必须保持的约束

- 槽按 `Pending → Spawning → Spawned → Finished` 推进。cancel 遇到 `Spawning` 必须等待 spawn 成功或失败的确定结果；spawn 失败路径也必须通知等待者。
- 注销同时校验 session id、instance generation 与槽 identity。旧 future 或旧 reader 不得删除新一代 observer/create。
- observer 只有在 initialize、model discovery、subscribe 完成且 generation 仍当前时才能把 instance 置为 Ready。stop 先线性化后，旧 start 只能以原 request id 返回错误，不能发布晚到 Ready。
- one-shot create 的最终 `thread/start` 与 cancel 共用短 send gate。门内只做取消/当前性复核和 frame 写入，不持有 initialize、response wait 或进程回收。
- `instance.stop`、observer failure、`provider.shutdown` 与 stdin EOF 必须 drain observer、one-shot create 和 execution；stopped/accepted 只表示相关子进程已退出。并发 stop/shutdown 等待同一清理，不得提前伪造终态。
- `instance.destroy` 与 start/stop 使用同一 transition。destroy 只能标记并移除非运行实例；已取得旧 runtime 引用的异步 start 也必须因 destroyed/generation 复核失败。
- event/response 共用 stdout writer 只负责串行化 frame。event write/flush 失败应在释放 writer lock 后幂等地通知主循环；全局 cleanup 不得依赖再次写 stdout。

## 反例

- `spawn()` 包含 initialize，返回后才把 observer 或 one-shot session 存入 instance。
- stop 只 drain Ready observer/execution，忽略正在 spawn/initialize 的 create。
- 先把状态设为 Stopped，再异步 kill/wait 子进程。
- cancel 修改状态后不与 `thread/start` 最终 frame 写入共享线性化点。
- 后台 event publish 记录 broken pipe 后继续服务 stdin，或在持有 stdout mutex 时通知会触发 shutdown 的主循环。

## 验证路径

- 让 observer initialize 永不响应，连续至少五轮交错 start/stop；每轮 stop response 前 PID 必须消失，start response 保留原 id 且为错误，事件中不得出现 Ready。
- 让 one-shot create initialize 永不响应，分别触发 `instance.stop` 与 stdin EOF；两者都必须在 2 秒内清零 PID，且 fixture 无 `thread/start`。
- 在 active execution 存在时关闭 Provider stdout，并制造 observer 异步 fault；忽略 SIGPIPE 后 Provider 仍必须由显式 terminal 信号退出并清零 execution PID。
- 保留既有 resume/cancel barrier、stale generation、16 active + 32 pending 饱和 stop/EOF、terminal repetition 测试；新 registry 不得削弱 execution slot 的线性化保证。
- 运行 Codex Provider 全测试、垂直测试五轮、workspace 测试、Clippy `-D warnings`、前端测试/build 与 `git diff --check`。

## 来源

- `../10-architecture/codex-provider-plugin-runtime.md`
- `../30-domains/agent-control/codex-app-server.md`
- `../40-runbooks/codex-app-server-unavailable.md`
