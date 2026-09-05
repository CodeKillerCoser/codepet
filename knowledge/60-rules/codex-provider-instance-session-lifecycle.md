# Codex Provider 子进程必须先登记再等待

## 规则

instance.start 是 Codex 唯一 App Server 创建入口。spawn 前登记 generation 占位，spawn 后立即登记真实 session，initialize/discovery 在锁外等待；stop/fail/shutdown 先取消并取走当前代的槽，等 spawn 收敛并回收进程后再发布 stopped。

## 适用场景

Provider instance 生命周期、SDK 心跳协调、stdio EOF/fatal、App Server 初始化及 conversation.create/resume 的发送边界。

## 反例

旧 observer/one-shot create 逻辑在本地变量里等待 initialize，stop 看不到已存在的 PID；晚到 future 还能在 stop 后发 thread/start。共享 Server 改造删除了 one-shot 创建路径，但 instance.start 仍必须防止这一窗口。

## 推荐做法

- instance session 槽按 Pending→Spawning→Spawned→Finished 推进；cancel 遇到 Spawning 等待 spawn 明确结束，失败也唤醒等待者。
- 注销同时校验 slot ID、instance generation 和槽 identity；旧 reader/future 不能删除新代。
- initialize、model discovery、subscribe 完成且 generation 当前，才发布 Ready；stop 先线性化时，旧 start 返回错误而不能晚到 Ready。
- create/resume 复用已登记的共享 Server，最终 request 写入与 cancel 共用短 send gate；门内只做复核和 frame 写入，不等待 response 或回收进程。
- stop/fail/shutdown drain 唯一 Server 并取消全部会话槽、pending approval、未物化缓存；正常 stop 不按会话重复创建或关闭子进程。
- SDK 先停止心跳协调，再做全局 shutdown/drain。stdout event 写失败必须发送唯一 terminal 信号唤醒 stdio 主循环，不能只在后台打印错误。

## 来源

`../10-architecture/codex-provider-plugin-runtime.md`、`../10-architecture/remote-and-provider-connections.md`，以及此前 stop/EOF 初始化窗口的 fixture 证据。

## 验证方式

延迟共享 Server initialize 连续五轮 start/stop，断言 stop 响应前 PID 退出、没有晚到 Ready；保留 resume/cancel barrier、16+32 饱和 stop/EOF、异步 Broken pipe 清理。连接级 binary 测试确认最后断开关闭 Server、Provider 仍可 describe、重连只启动一个新 PID。已删除只验证“每会话第二次 initialize”的测试：该创建路径已不存在，以共享初始化与进程计数断言替代。
