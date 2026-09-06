# Host 常驻防休眠与手机离线执行

## 背景

用户报告 Mac 锁屏后手机连接断开，任务随后终止。源码证据是 Provider SDK 原 `heartbeat.rs::manage_instance` 只根据客户端集合决定 start/stop，最后连接消失会直接停止整个 Harness；这能解释任务被回收，但尚不能证明实际断线一定源于系统休眠。

## 目标

- Code Pet 在 macOS 运行期间阻止系统自动空闲休眠，允许锁屏和显示器熄灭。
- 手机全部离线后，已开始的执行与审批继续保留；空闲后释放 Harness。
- 保持 Host 故障、显式退出与 Provider 关闭的进程清理边界。

## 非目标

不阻止用户主动睡眠、合盖或关机，不保证系统休眠期间继续执行，不实现跨进程重启恢复。本次防休眠仅覆盖 macOS；Windows/Linux 未新增电源实现。

## 现状理解

三个内置 Provider 共用 Rust SDK 的 mux 与心跳协调器，并发布标准 turn/approval 事件。运行实例各自管理原生 session generation，负责拒绝旧代事件。SDK 不直接访问 Codex 内部 active turn 槽。

## 实现路径

`runtime/activity.rs` 是回收用途的轻量投影，以实例 ID、会话 ID、turn ID 跟踪 queued/running/waiting-approval；completed/failed/interrupted 为终态。审批单独保留，解决或对应 turn 结束后释放。不使用输出静默时长判断结束。终态标记保留到实例停止/出错，避免晚到 start 响应复活旧任务；它不是对外会话状态源。

业务请求持有共享执行门，心跳的空闲 stop 必须独占该门并复核活动状态，覆盖响应尚未返回、turn ID 尚未登记的窗口。当前门覆盖同一 Provider 进程的请求，因此另一实例的慢 RPC 可能推迟空闲回收，但不会取消其任务。start/steer 已接收后由受管理的异步执行继续处理，即使客户端取消响应流也不撤销已发送操作；shutdown/EOF 关闭调度并取消这些执行。不得自动重发未知结果的 turn/start。

无客户端时，每秒复核一次：没有执行中请求、未结束 turn 和待审批，才调用幂等 instance.stop。Host 心跳过期、实例移除、stdin EOF 或 provider.shutdown 仍强制清理，不能把网络离线保留扩展成脱离 Host 的孤儿进程。

macOS `AwakeGuard` 使用 IOKit `PreventUserIdleSystemSleep`，setup 时申请，由 Tauri managed state 持有。应用真正 Exit 时释放；进程异常退出由系统回收进程所属断言。申请失败记录 power 错误日志，不伪报成功。不修改用户持久化电源设置、不要求屏幕常亮、不创建外部 caffeinate 子进程。

## 涉及模块

- `sdk/rust/codepet-provider-sdk/src/runtime/{activity,heartbeat,mux}.rs`：统一执行保留与请求取消边界，三个 Provider 都受影响。
- `tools/cp-sdk-gen/provider-runtime.mjs`：独立导出 SDK 必须携带新增手写模块。
- `src-tauri/src/platform/power.rs`、`src-tauri/src/lib.rs`：macOS 电源断言与应用退出生命周期。
- `crates/providers/codepet-provider-codex/tests/provider_vertical.rs`：通过真实 Provider binary 和原生 fixture 验证共享 Server PID、离线任务与审批。

## 风险

- 迟到响应、其他会话终态误清理任务：SDK 测试覆盖终态不可复活，binary 测试覆盖两个会话分别终结。
- 取消响应后留存执行，在退出后晚到启动：mux 取消/关闭测试与 Codex stop/EOF 回归验证清理。
- Provider 不发布终态可能导致持续保留：不以超时猜测完成，显式 stop/退出仍可清理；依赖 Provider 事件契约。
- 长时间不退出的实例会保留本代终态 ID，内存随 turn 数增长；实例 stopped/error 时清空。
- 防休眠无效或退出后残留：macOS 原生测试通过 `pmset -g assertions` 检查申请和释放。

## 测试计划与验证结果

2026-09-07：

- `cargo test --manifest-path sdk/rust/Cargo.toml -p codepet-provider-sdk`：44 项通过，覆盖活动投影、离线回收、启动保护、响应取消、shutdown/EOF 与 wire。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test builtin_provider_integration`：三个内置 Provider 的 Host 集成测试通过，保留空闲断线回收与重连行为。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --test provider_vertical -- --test-threads=1`：46 项通过，1 项真实 Codex CLI smoke 按原约定忽略。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-claude -p codepet-provider-opencode -- --test-threads=1`：41 项通过，2 项依赖外部真实 runtime 的测试按原约定忽略。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib platform::power::tests -- --nocapture`：原生注册/释放测试通过。
- `cargo check --manifest-path src-tauri/Cargo.toml`：通过。
- `npm run sdkgen:test`：独立导出文件完整性 3 项通过。
- `git diff --check`：通过；未调用代码格式化工具。

安装版人工验收：启动后执行 `pmset -g assertions`，确认 Code Pet 的 PreventUserIdleSystemSleep；运行长任务后锁屏、手机切网络再重连，确认状态和消息恢复；退出后确认断言消失。不把合盖当作自动空闲休眠测试。

## 知识沉淀

更新 `remote-and-provider-connections.md` 与 `60-rules/codex-provider-{turn-writer,instance-session}-lifecycle.md`，替换原“最后连接消失即终止任务”的规则。

## 未知项

尚未在安装版上执行真实手机锁屏、网络切换及重连验收；没有确认用户此次断线的系统日志。没有验证真实 Codex CLI 或其他操作系统的电源行为。
