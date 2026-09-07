# Provider 启动握手与后台信息探测

## 规则

Provider 启动只建立可接受业务请求的通信链路。版本、模型目录、项目能力、账号、额度和用量等详细信息在实例 Ready 后后台探测，使用通知更新 Host；探测失败不能将已就绪实例改成 Error。

## 适用场景

三个内置 Provider 的 instance.start、runtime.getInstalled(refresh)、状态通知、停止和重启，以及 Host 合并启动响应与通知的逻辑。

## 反例

Windows OpenCode 原路径在服务 health 成功后继续等待 model/agent/provider，再串行执行 auth list 和 stats。隔离副本实测启动约 20 秒，超过 Host 的 10 秒 RPC 预算。Claude 也在启动中执行版本和账号命令；Codex 在 initialize 后串行等待模型、项目和三个账号接口。

## 推荐做法

- Claude 的适配器就绪即可返回；Codex 保留 app-server spawn、initialize、subscribe；OpenCode 保留 serve 启动、health 和 subscribe。已配置路径的存在性检查仍是配置校验，禁止用版本支持范围决定能否启动。
- CLI 详细探测通过 SDK background_probe 执行；SDK process 统一管理 Windows 隐藏窗口和 Job Object、Unix 进程组。单条 CLI 最多 30 秒，stdout/stderr 各最多 1 MiB，超时或取消释放进程树。命令的参数、工作目录和数据目录沿用实例上下文，不能改进程全局环境。
- 独立探测并发，具有关联关系的数据合并后发布：OpenCode 目录由 model/agent/provider 合成，空模型时才回退 CLI；Codex 模型与项目探测合成能力，额度与用量合成展示。每组完成即通知，不等待其他组。
- 没收到结果时保留缺省/未知；JSON 明确的 loggedIn=false 才是未登录，网络错误不是不支持，命令无输出不是未安装。静态协议能力可直接描述，动态选项不得伪造。
- 安装版本扫描的 blocking worker 同样必须可取消：使用 RuntimeScanner.start_cancellable，子进程登记到 RuntimeProbeControl。scanner.stop 主动终止已登记进程；先取消、后登记的进程也立即终止，不能仅 abort 外层 Tokio task 后等待旧版本命令超时。
- 刷新取消上一轮任务并递增 epoch；停止、失败、销毁和 shutdown 同样取消。结果应用必须在锁内复核 Ready 与 epoch，并与状态通知串行发布。Codex 的共享 RPC 有 30 秒上限，关闭 session 唤醒等待者；刷新取消后的在途 RPC 可以有界收敛，但不得应用旧结果。
- Host 必须原子合并状态：在启动 RPC 期间已经收到的 Ready 详细信息或停止/错误状态，不能被迟到的初始启动响应覆盖。
- 测试通过请求标记和释放栅栏验证并行，而非仅比较耗时比例。业务测试等待模型通知或能力快照，不能固定读取前 N 条事件，也不能假定启动响应包含账号和模型。

## 来源

2026-09-07 用户明确的启动边界，以及 [OpenCode Windows 排查](../40-runbooks/opencode-history-controls-and-completion.md)。

## 验证方式

三个 Provider 的 provider_vertical 覆盖握手、慢探测、并行、通知和取消；SDK background_probe 测试覆盖超时与输出上限；Host startup_snapshot_tests 覆盖通知先于启动响应。native_runtime 使用隔离数据目录、不发模型请求，要求握手在 10 秒内。原生 Windows 结果不代表 macOS 已验证。
