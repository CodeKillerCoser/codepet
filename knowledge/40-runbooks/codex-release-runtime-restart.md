# Codex 手动释放后自动恢复运行时

## 现象与证据

2026-11-08，用户反馈手机释放对话控制权后 Codex 不再启动，并明确说明没有点击“重新接管”，期望运行时自动启动。

源码确认旧行为是明确设计，并非本次已证明的进程崩溃：`conversation_release_interaction` 设置 `interaction_released=true` 后停止共享 App Server；`start_runtime` 拒绝非显式恢复的启动；`ready_server` 同时拒绝历史、项目等读取。旧回归 `release_interaction_stops_shared_server_and_stays_stopped_until_explicit_acquire` 也要求保持停止。

自动空闲回收是另一条路径：分析当时 SDK 默认窗口 300 秒，Codex main 启用 `restart_after_idle=true`，空闲 stop 后会 start；Claude/OpenCode 未启用重启。随后用户明确空闲回收只适配 Codex，SDK 已改为默认关闭，详见 [默认空闲回收误停其他 Provider](provider-idle-policy-opt-in.md)。以上操作回收的是插件持有的 Harness 子进程，Provider 插件进程仍在线。手动 release 的旧暂停标记不能由空闲维护自动解除，因而符合当时观察。

## 原因与行为调整

旧实现把“放开旧会话交互权”和“维持整个运行时停止”绑定。用户的新要求覆盖旧规约：释放时结束旧进程、取消该实例所有旧任务与审批；随后自动启动干净的新进程。新进程可服务项目、历史和新建会话，但启动与读取不自动 resume 旧会话。手机仍保留会话暂停状态，点击“重新接管”只接管指定会话。

## 引入历史

2026-09-16 的立即释放规约已描述保持停止的意图；未追溯具体引入提交。2026-11-08 依据用户明确的新语义调整，不把旧逻辑误写成已经还原的随机启动故障。

## 实现与模块

- Codex `provider.rs`：手动释放复用已有 stop 和唯一的 `start_runtime_owned` 创建路径；克隆 observation/shutdown 上下文，使客户端取消不能把实例留在停止与重启之间。新进程仍使用既有初始化、metadata、generation 和 session 登记机制。
- 实例内 `release_transition` 串行化重复释放；尚未重新接管或新建会话时，重试不再次关闭干净的新进程。旧进程终止失败仍保留句柄供重试；重启失败单独返回 `interaction_restart_failed`，不把释放旧进程失败和新进程启动失败混淆。
- `ready_server` 不再把暂停会话当成整个运行时不可读。显式 acquire 可复用已经 Ready/Starting 的新进程；成功新建会话登记自己的 execution 后解除暂停标记。旧 acquire 继续复核 generation，旧审批不得写入新进程。
- Provider shutdown 先设置共享关闭标志，再收回 observation 和实例；启动路径检查该标志，退出期间不得在停止后复活进程。
- Host/Gateway 的接口、Remote 协议和 UI 无修改。Remote 已在发出 release 前暂停该 Provider 自动 acquire，Ready 状态本身不会解除手机暂停。

## 验证结果

```sh
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --test provider_vertical release_interaction -- --test-threads=1
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --lib
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --test provider_vertical idle_ -- --test-threads=1
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --test provider_vertical provider_binary_stop_linearizes -- --test-threads=1
cargo test --manifest-path crates/Cargo.toml -p codepet-host --test builtin_provider_integration -- --test-threads=1
```

分别通过 6、80、2、1、3 项；idle_ 与 release_interaction 有 1 项重合。Codex lib 的真实本机 ChatGPT 安装依赖测试按原配置忽略 1 项。Remote 工作区运行 `flutter test --no-pub test/conversations/conversation_detail_screen_test.dart --plain-name 'immediate release' --reporter expanded`，6 项通过。`git diff --check` 通过。

## 风险与回归防线

- 旧 writer 被重新占用：真实 Provider fixture 断言旧 PID 退出、新 PID 自动出现、A/B 的 thread/resume 次数不因重启或历史读取增加；显式 acquire 复用新 PID，仅恢复 A。
- 重复回收与其他实例受影响：重复 release 只产生一份替代进程，独立 Provider 的 PID 保持存活；清理失败 mock 仍验证句柄重试。
- 无法使用新进程：Host 集成在不发送显式恢复请求之前验证 Ready、历史读取、新建会话，随后验证旧会话 resume。
- 生命周期竞态：挂起 acquire 可由 control lane 释放；重启 initialize 挂起时 provider.shutdown 能回收所有已创建 PID；既有连续五轮 start/stop 仍无晚到 Ready。
- 重启失败与后续空闲：fixture 拒绝 initialize 时返回新错误码并回收新旧 PID，不发生重启循环；手动释放后的干净进程仍能经历下一次自动空闲重启。
- 手机误接管：既有 widget/controller 测试验证 release 后暂停自动 acquire、跨页面和重连保持暂停、显式接管及迟到恢复响应隔离。

## 规约更新

已更新 [立即释放规约](../60-rules/codex-explicit-interaction-release.md)、共享 Server 生命周期规约、Runtime 生命周期和连接架构；区分插件在线、运行时 Ready 和会话交互权三个状态。

## 未知项

尚未构建、安装或发布 Release，也未在用户手机与 Mac mini 上端到端复测。原生 Codex App 的会话锁实际体验仍需安装版确认。Claude 的插件握手失败是独立问题，本改动不宣称解决它。
