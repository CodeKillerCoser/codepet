# Codex 立即释放与显式恢复

## 规则

`conversation.releaseInteraction` 立即结束该 Codex Provider 实例持有的共享 harness server。它不是单个会话的 interrupt：该实例全部会话的运行任务、审批与交互权一起失效，允许用户在活动任务中调用。仅操作已登记的自有 session/Child 句柄，不扫描进程列表，也不使用强制接管的环境标记筛选。

释放后必须保持主动释放状态，在线心跳、后台 list/get、刷新与普通 instance.start 都不能重新启动 server；只有显式 conversation.resume/acquireInteraction 才能解除并恢复。界面必须说明影响整个 Provider 实例，不能把释放按钮当成无副作用的页面退出。

## 适用场景

- `protocol/{gateway,provider}/v1`：新增同名方法与 `conversation.releaseInteraction` methods 能力。Gateway 请求使用 RoutedResourceId，Provider 使用 ProviderResourceId；响应为 `{released:true, scope:"providerInstance"}`。Provider 方法放入 control lane，使普通 acquire/turn 请求挂起时仍可释放。
- `codepet-host/src/gateway.rs` 与 `providers/manager.rs`：映射能力及路由；允许具有释放能力的 Stopped 实例进入显式 acquire 路径，其他 Provider 保持必须 Ready 的限制。Host 对未广告能力返回 `provider_capability_unsupported`；直接调用不支持的 Provider 返回 `capability_unsupported`。
- `codepet-provider-codex/src/provider.rs`：复用共享 `start_runtime` 与 stop 生命周期，保存主动释放标志、取消全部 execution slots、pending approval、metadata 和未物化缓存。释放任务不随客户端响应流取消而中断。启动与恢复沿用同一个注册/generation 检查路径。
- `codepet-provider-codex/src/client.rs`：沿用统一的 SessionControl 接口及 Child.kill/wait 完成终止。kill 失败后 RPC admission 仍关闭，但后续 shutdown 必须重试实际进程控制，不能因 running=false 就返回成功。
- Claude/OpenCode 不广告该能力，直接调用时明确返回不支持。

## 反例

- 仅 unsubscribe 当前 thread：原生 writer 未必释放，不能等价于结束共享 server。
- kill 后在线 heartbeat 又调用 instance.start，立即创建新进程并重新占锁。
- Stopped 一律禁止路由 resume，导致用户释放之后无法重新接管。
- 在 acquire 等待原生响应的整个期间持释放互斥锁，导致“立即释放”排在挂起请求之后。
- 先设置 running=false，kill 失败，再把第二次 shutdown 当作“已经停止”返回成功。
- 显式恢复在检查 Stopping 状态之前清除释放标志：恢复虽失败，但后续心跳会在 stop 完成后自动启动。

## 推荐做法

主动释放首先设置抑制启动标志，再走现有 stop：线性化 generation、取消 slot、关闭共享进程并等待退出。沿用 `event.instanceStatusChanged` 的 Stopping→Stopped 通知，插件仍在线；Gateway 保持 capability methods，ProviderSummary.runtime.status 如实为 stopped。

释放幂等。底层停止或事件发布失败返回 `interaction_release_failed`，不返回 released=true；终止失败的 session 保存在实例内，重复 release 重试它，显式启动在失败未解决时也拒绝。没有运行中的 server 时释放成功，且仍设置主动释放状态。Provider 实际进程退出才结束本次内存生命周期；该状态不跨显式插件重启持久化。

显式 acquire 通过同一 start_runtime 创建一个新 server，等待 Ready，再建立当前会话 execution。启动参数、harness 标记、metadata 探测与旧路径一致。恢复只解锁本次请求的会话，不遍历并自动 resume 旧会话。旧 generation 的审批不能写入新 server。排队中的旧 acquire 复核 lifecycle generation，释放发生后不能借新 session 偷偷重新取得交互权。

后台读取在释放后可能收到 Provider 的 `interaction_released` 或 Host 的 `provider_instance_unavailable`。Remote 应在释放请求发出前暂停该 Provider 的自动 acquire，丢弃迟到结果；收到 Stopped 后即便 isAvailable 为 false，也允许用户显式重新接管。请求失败时不能假定 server 仍持有旧 writer。

## 来源

2026-09-16 用户要求直接结束当前 Provider 启动的 harness server。源码证据：一个 CodexInstanceRuntime 只登记一个 server，stop 已 drain 全部 sessions/executions；SDK heartbeat 会在在线客户端存在时持续调用 instance.start，Host 历史读取也会触发恢复。因此仅新增 kill 方法不足以实现“释放后保持释放”。本规约补充共享 server 生命周期规约与强制接管规约。

## 验证方式

- Client mock：首次 kill 返回失败，第二次 shutdown 仍调用进程控制并成功，验证不会误报。
- Provider fixture：活动 A/B 共用 server，release 返回前旧 PID 退出；重复 release 幂等；心跳、instance.start 和 get 不增加 PID；显式 acquire 只新建一个 server，其他会话不会自动恢复；另一个 Provider 的 harness 仍存活。
- Provider fixture：挂起 acquire 时通过 control lane 释放并取消请求；审批存在时释放，重启后旧审批返回错误。
- 真实 Host/Gateway + fixture 二进制：验证能力广告、release→Stopped、后台 get 不重启、resume→Ready 且历史可读取；Host mock 验证未广告能力时拒绝路由。
- 协议生成一致性检查以及 Remote Dart SDK `cp-sdk-gen --check`。没有在真实桌面或真实用户任务上调用 release。

## 未知项

Windows 上使用 mock/fixture 验证；本次没有 macOS 实机运行。业务层无新增平台分支，继续使用已有跨平台 Child 进程控制。外部客户端明确发送 acquire 时可重新启动，这是显式恢复语义；仅靠后端不能判断某个客户端的 acquire 是按钮点击还是错误的自动重试。

## 本次验证记录（2026-09-16）

以下 Cargo 命令均使用 `--manifest-path crates/Cargo.toml --target-dir D:/17633/Documents/Code/codepet/crates/target`：

- `cargo test -p codepet-provider-codex --lib`：77 项通过。
- `cargo test -p codepet-provider-codex --lib --test provider_vertical release_ -- --test-threads=1`：1 项 mock、3 项 Provider fixture 通过。
- `cargo test -p codepet-provider-codex --test provider_vertical sdk_heartbeat -- --test-threads=1`：2 项心跳回归通过。
- `cargo test -p codepet-host --test manager_gateway gateway_release_interaction_rejects_provider_without_capability`：通过。首次断言误用了直接 Provider 的错误码，修正为 Host 既有错误码后通过。
- `cargo test -p codepet-host --test builtin_provider_integration gateway_release_interaction_stops_codex_and_explicit_resume_restarts_it`：通过。
- `cargo check -p codepet-provider-codex -p codepet-provider-claude -p codepet-provider-opencode -p codepet-host --tests`：通过；存在未使用代码告警。
- `node tools/protocol-codegen/generate.mjs --check`、`node --test tools/protocol-codegen/test.mjs`：通过；Remote 输出目录的 `cp-sdk-gen --check` 通过。

早期 fixture 运行曾在初始 configure/instance.start 阶段超时，尚未进入 release；串行重跑通过，未确认该超时根因。未运行全部跨 Provider 集成测试或 macOS 实机测试。