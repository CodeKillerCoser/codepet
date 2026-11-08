# Codex 立即释放、运行时重启与显式接管

## 规则

`conversation.releaseInteraction` 立即结束该 Codex Provider 实例持有的共享 harness server，然后自动启动一个干净的新进程。它不是单个会话的 interrupt：该实例全部会话的运行任务、审批与交互权一起失效，允许用户在活动任务中调用。仅操作已登记的自有 session/Child 句柄，不扫描进程列表，也不使用强制接管的环境标记筛选。

2026-11-08 用户明确调整语义：运行时自动恢复可用，释放过的会话不自动接管。新进程就绪后项目、历史及新建会话正常可用；手机仍保持主动释放提示，用户通过 conversation.resume/acquireInteraction 接管指定会话。新进程初始化、后台读取和心跳均不得批量 resume 旧会话。界面必须说明释放影响整个 Provider 实例。

## 适用场景

- `protocol/{gateway,provider}/v1`：新增同名方法与 `conversation.releaseInteraction` methods 能力。Gateway 请求使用 RoutedResourceId，Provider 使用 ProviderResourceId；响应为 `{released:true, scope:"providerInstance"}`。Provider 方法放入 control lane，使普通 acquire/turn 请求挂起时仍可释放。
- `codepet-host/src/gateway.rs` 与 `providers/manager.rs`：映射能力及路由；允许具有释放能力的 Stopped 实例进入显式 acquire 路径，其他 Provider 保持必须 Ready 的限制。Host 对未广告能力返回 `provider_capability_unsupported`；直接调用不支持的 Provider 返回 `capability_unsupported`。
- `codepet-provider-codex/src/provider.rs`：复用共享 `start_runtime_owned` 与 stop 生命周期，保存会话暂停标志、取消全部 execution slots、pending approval、metadata 和未物化缓存。释放任务持有 observation/shutdown 上下文，客户端响应流取消不能中断停止及重启；实例内串行释放避免重复回收新进程。启动与恢复沿用同一个注册/generation 检查路径。
- `codepet-provider-codex/src/client.rs`：沿用统一的 SessionControl 接口及 Child.kill/wait 完成终止。kill 失败后 RPC admission 仍关闭，但后续 shutdown 必须重试实际进程控制，不能因 running=false 就返回成功。
- Claude/OpenCode 不广告该能力，直接调用时明确返回不支持。

## 反例

- 仅 unsubscribe 当前 thread：原生 writer 未必释放，不能等价于结束共享 server。
- kill 后新进程批量 resume 旧会话，立即重新占用刚释放的 writer。
- 把“暂停接管”当成“Provider 不可用”，阻止历史、项目和新会话访问。
- Stopped 一律禁止路由 resume，导致用户释放之后无法重新接管。
- 在 acquire 等待原生响应的整个期间持释放互斥锁，导致“立即释放”排在挂起请求之后。
- 先设置 running=false，kill 失败，再把第二次 shutdown 当作“已经停止”返回成功。
- 显式恢复在检查 Stopping/清理失败之前清除暂停标志，导致失败请求放开其他排队交互。

## 推荐做法

主动释放首先暂停会话接管，再走现有 stop：线性化 generation、取消 slot、关闭共享进程并等待退出。随后启动新进程，沿用 `event.instanceStatusChanged` 的 Stopping→Stopped→Starting→Ready 通知，插件仍在线；Gateway 保持 capability methods，runtime.status 如实反映过渡状态。

释放幂等：未重新接管、未新建会话时，重复 release 不再关闭刚启动的干净进程。底层停止或事件发布失败返回 `interaction_release_failed`；终止失败的 session 保留用于重试，未清理成功不得启动替代进程。旧进程已退出但新进程启动失败返回 `interaction_restart_failed`，不假装旧进程仍持有交互权，不无限自动重试。后台 metadata 仍按既有 Starting→Ready/Error 事件收敛。Provider shutdown 必须先设置关闭标志，阻止释放任务在退出期间重新创建 server。

显式 acquire 优先复用已经重启的新 server；未启动时走同一个 start_runtime，等待 Ready，再建立当前会话 execution。只接管本次请求的会话，不遍历旧会话。新建会话可独立使用新 server；成功创建自己的 execution 后解除实例暂停标记。旧 generation 的审批不能写入新 server。排队中的旧 acquire 复核 lifecycle generation，不能借新 session 偷偷取得交互权。

Remote 在释放请求发出前暂停该 Provider 的自动 acquire，丢弃迟到结果；自动重启回到 Ready 不等于重新取得会话交互权。过渡 Stopped 时保留显式恢复入口；请求失败时不能假定旧 writer 仍存在。无需修改协议或靠手机触发 instance.start。

## 来源

2026-09-16 初始实现要求释放后保持停止；2026-11-08 用户明确要求手动释放后自动启动运行时。当前规约以前述新要求为准，见 [手动释放后自动恢复运行时](../40-runbooks/codex-release-runtime-restart.md)。SDK 心跳现已不驱动 start/stop，自动空闲维护见 Runtime 生命周期文档。

## 验证方式

- Client mock：首次 kill 返回失败，第二次 shutdown 仍调用进程控制并成功，验证不会误报。
- Provider fixture：活动 A/B 共用 server，release 返回前旧 PID 退出、自动生成一个新 PID；重复 release 幂等；心跳、instance.start 和 get 不额外增加 PID、不 resume A/B；显式 acquire 复用新进程，其他会话不会自动恢复；另一个 Provider 的 harness 仍存活。
- Provider fixture：挂起 acquire 时通过 control lane 释放并取消请求；审批存在时释放，重启后旧审批返回错误。
- 真实 Host/Gateway + fixture 二进制：release 自动回到 Ready，历史、新建会话与显式 resume 可用；Host mock 验证未广告能力时拒绝路由。另覆盖重启失败、重启初始化期间 shutdown、重启后的再次自动空闲回收。
- 协议生成一致性检查以及 Remote Dart SDK `cp-sdk-gen --check`。没有在真实桌面或真实用户任务上调用 release。

## 未知项

Windows 上使用 mock/fixture 验证；本次没有 macOS 实机运行。业务层无新增平台分支，继续使用已有跨平台 Child 进程控制。外部客户端明确发送 acquire 时可重新启动，这是显式恢复语义；仅靠后端不能判断某个客户端的 acquire 是按钮点击还是错误的自动重试。

## 本次验证记录（2026-09-16）

以下为历史版本验证，其中“保持停止”已被 2026-11-08 的新语义替代；当前结果见上述修复记录。

以下 Cargo 命令均使用 `--manifest-path crates/Cargo.toml --target-dir D:/17633/Documents/Code/codepet/crates/target`：

- `cargo test -p codepet-provider-codex --lib`：77 项通过。
- `cargo test -p codepet-provider-codex --lib --test provider_vertical release_ -- --test-threads=1`：1 项 mock、3 项 Provider fixture 通过。
- `cargo test -p codepet-provider-codex --test provider_vertical sdk_heartbeat -- --test-threads=1`：2 项心跳回归通过。
- `cargo test -p codepet-host --test manager_gateway gateway_release_interaction_rejects_provider_without_capability`：通过。首次断言误用了直接 Provider 的错误码，修正为 Host 既有错误码后通过。
- `cargo test -p codepet-host --test builtin_provider_integration gateway_release_interaction_stops_codex_and_explicit_resume_restarts_it`：通过。
- `cargo check -p codepet-provider-codex -p codepet-provider-claude -p codepet-provider-opencode -p codepet-host --tests`：通过；存在未使用代码告警。
- `node tools/protocol-codegen/generate.mjs --check`、`node --test tools/protocol-codegen/test.mjs`：通过；Remote 输出目录的 `cp-sdk-gen --check` 通过。

早期 fixture 运行曾在初始 configure/instance.start 阶段超时，尚未进入 release；串行重跑通过，未确认该超时根因。未运行全部跨 Provider 集成测试或 macOS 实机测试。

2026-09-17 整合配对分支时，桌面 `cargo check` 在 `provider_host_compat.rs::map_provider` 报 E0004：遗漏 `ConversationReleaseInteraction` 枚举。旧兼容协议没有该方法，显式映射为 `None` 后检查通过。以后新增 Gateway 能力需同步检查桌面兼容层；不要向旧协议广告未实现的能力。验证命令为 `cargo check --manifest-path src-tauri/Cargo.toml --locked --jobs 4`，`TAURI_CONFIG` 使用 `src-tauri/tauri.bin.conf.json` 的内容以排除打包资源要求。
