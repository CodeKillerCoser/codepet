# Harness 启动探测与 Ready 边界

## 规则

Provider 插件连通不等于 Harness Ready。Harness 建立通信后保持 Starting，版本、模型、项目支持、账号、额度和用量等本次探测并发执行；全部结束后，原子应用结果并一次发布 Ready。Remote 在 Ready 前不读取详细能力、会话或项目列表。

## 适用场景

三个内置 Provider 的 instance.start、启动通知、停止与重启，Host 合并启动响应和通知，以及 Remote 首次连接、状态更新和列表补拉。

## 反例

- 握手后先发布 Ready，再按探测项分别通知能力：Remote 可能缓存临时目录，随后漏加载项目或重复请求。
- 将整轮探测放进同步启动 RPC：Windows 慢 CLI 会超过 Host 的 10 秒 RPC 预算。
- Starting 和 Ready 的 revision 相同时，继续复用 Starting 期间取得的临时能力。

## 推荐做法

- `instance.start` 完成必要通信建立后可返回 Starting；后台任务持有生命周期所有权，整轮探测结束才进入 Ready。重复 start 返回当前状态，不重复启动探测。Provider 插件可以一直在线，Harness 状态独立展示。
- 各项并发启动，依赖结果在本轮内部合成。所有项成功、失败或超时都算本轮结束；失败值沿用现有未知/缺省语义，不伪造模型、登录状态或项目支持。网络错误不是接口不支持。
- 首次探测之前不发布 Ready，不对外发布中间结果。Ready 后显式重检保留可用状态，同一轮仍汇总后一次通知。
- 本次不改变 revision 方案。Remote 必须以 Starting→Ready 边界丢弃临时能力；Ready 后一次 describe，同代同 revision 的在途 describe 合并，完整结果到达后加载会话和项目。重复心跳不重复读取列表。
- CLI 使用 SDK background_probe 和 process：Windows 隐藏窗口及 Job Object、Unix 进程组；命令、数据目录、超时、输出预算沿用现有约束。Codex RPC 有 30 秒上限，关闭 session 唤醒等待者。OpenCode 模型 HTTP 查询为空时允许有界 CLI 回退。
- 停止、失败、销毁与 shutdown 取消探测并使 epoch 失效。结果应用在锁内复核 Starting/Ready 与 epoch，并与状态通知串行；迟到结果不得让已停止实例回到 Ready。
- Host 原子合并状态：在启动 RPC 期间收到的完整 Ready 或停止/错误状态，不能被迟到的 Starting 响应覆盖。
- Runtime 安装扫描与 Harness 启动探测是两个阶段。RuntimeScanner 仍后台扫描并通知库存；阻塞版本子进程必须登记到 RuntimeProbeControl，以便 stop 清理进程树。

## 来源

2026-09-07 用户明确调整启动边界：全部探测结束前 Harness 保持启动状态，完成后统一通知 Host；revision 暂不修改。证据见 [Mac 发现与项目加载排查](../40-runbooks/runtime-discovery-and-project-catalog-revision.md)。此约定替代先 Ready 后分批探测的旧实现。

## 验证方式

三个 Provider 的 provider_vertical 用 PID/请求标记和释放栅栏验证并发、Starting、完整 Ready 和停止取消；Remote device_session 测试验证 Starting 无数据请求、同 revision Ready 后只拉一次、重复心跳及在途合并。Host manager_gateway 和 startup_snapshot_tests 验证通知与响应顺序。真实 native_runtime 使用隔离目录、不发送模型请求；初始 RPC 10 秒内返回，另等待完整 Ready。
