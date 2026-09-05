# Codex 详情超限、能力误判与项目漏加载

本记录保留分阶段证据。2026-09-06 后续按用户约束统一为 full turns 透传 caller cursor/limit，删除原生整行上限与版本驱动 item 分页，增加 tool item `_meta` 文本截断。下文 10-turn、notLoaded/item probe 与原生超限数据均描述当时版本，当前行为见 [文本与分页规约](../60-rules/provider-item-text-and-pagination.md)。

## 现象

2026-09-05 Android 模拟器打开“你每天干错事情，我的token都被你浪费完了”时提示 Provider 不支持加载详情；另一长会话提示尺寸超限，重连后项目列表为空。本记录保留分阶段排查证据。覆盖安装后，2026-09-06 再次复现历史读取超限并定位到 Host 启动环境下的 Harness 版本解析，见下文。

## 复现路径

- 本机 `wangxin-2.local`，ADB 位于 `/opt/homebrew/share/android-commandlinetools/platform-tools/adb`，设备 `emulator-5554`，包名 `com.codepet.remote`。
- 手机通过 `10.0.2.2:47622` 连接 Host。正在运行的安装版 Codex Provider 与磁盘二进制 inode 一致；不能仅凭文件时间早于 Git 提交判断进程过期。
- 原生 CLI 为 `/Applications/ChatGPT.app/Contents/Resources/codex`，版本 `0.153.1`。独立 Provider 探针使用安装版二进制和 Frame V1，不发送 turn/start；探针结束关闭其子进程。

## 证据

Android 日志通过 `adb exec-out run-as com.codepet.remote cat files/diagnostics/logs/codepet.log` 读取。以下时间为北京时间：

- 17:37:19：`project.list` 成功，随后记录 6 个项目。
- 20:04:58 至 20:05:04：重连、describe、Provider changed 与 conversation.list 交错发生；没有 project.list 请求，最终报告 0 个项目。
- 20:05:22：会话 `01a06f66-089b-7483-8bb8-7ecc09fbcf69` 的 conversation.get 报 `App Server physical line exceeds 16777216 bytes`；随后 acquireInteraction 报 session shut down，另一会话报 Provider instance not ready。
- 17:45:31：会话 `01a06c35-79f9-7b10-a03f-769c8a4b7e18` 的 conversation.get 也曾报同一 physical-line 错误。

安装版独立探针与原生请求对照：

| 请求 | 结果 |
| --- | --- |
| 第一会话 conversation.get(limit=40) | 成功，669 个投影 item |
| 第二会话 conversation.get(limit=40) | 成功，1,614 个投影 item |
| 第一会话 acquireInteraction | 稳定返回 provider_protocol_error，原始行超过 16 MiB |
| acquireInteraction 失败后同一独立 Provider 执行 get | 仍成功，669 个 item |
| 第一会话原生 thread/resume，默认参数 | 30,496,423 字节 |
| 同会话 thread/resume，excludeTurns=true | 1,733 字节，成功 |
| 同会话 thread/turns/list(full, limit=10) | 24,756,517 字节 |
| 同会话 thread/turns/list(full, limit=1) | 1,279,567 字节 |
| 原生 project/list(limit=1) | 成功，1 个项目 |

本机 CLI 生成的 `v2/ThreadResumeParams.json` 明确支持 excludeTurns：只返回 metadata 和 live-resume state，由 turns/items 分页读取历史。探针字节数包含 JSON 物理行；不保存会话正文。临时探针和结果位于 `/tmp/codepet-investigation/`，不是持久交付依赖。

## 初步影响范围

- `crates/providers/codepet-provider-codex/src/client.rs`：resume 只传 threadId，默认全量历史可突破 reader 的 16 MiB 限制。reader 错误结束对应 session，影响交互权获取。
- `codepet-remote/lib/application/conversations/conversation_detail_controller.dart`：reload 同时启动 acquireInteraction 和历史读取；methods 为空即显示“不支持”，没有区分能力尚未加载。
- `codepet-remote/lib/gateway/generated_gateway_mapper.dart`：ProviderSummary 映射为 capabilitiesLoaded=false、methods=[]，这是摘要状态，不能直接解释为能力缺失。
- `codepet-remote/lib/application/sessions/device_session.dart`：能力刷新与初始项目请求之间缺少补拉机制，导致真实支持项目的 Provider 仍可能显示空列表。

## 当前判断

1. **resume 超限已复现。** Provider Frame 压缩和 Remote 的 provider_response_too_large 缩页都处于下游，无法解决上游 reader 的 provider_protocol_error。不能简单调大传输上限。
2. **详情能力状态存在确定的恢复缺口。** provider.changed 的新 revision 摘要先进入会话状态，空 methods 触发“不支持”；describe 返回同 revision 的完整能力后，`_CapabilityBinding` 只比较 generation、lease、providerId、revision，不比较 capabilitiesLoaded。`_sessionChanged` 走相同 binding 分支，仅通知 UI，不 reload 或清理原错误。
3. **项目存在确定的漏请求路径。** 初次连接只遍历当时有 project.list 的 Provider；`_refreshProviderDescription` 仅替换描述并通知，不补请求项目。日志“无 project.list 请求但 0 项目”与该路径一致，不能据此认定项目被删除。

## 已排除方向

- 不是项目接口完全不支持：原生接口与安装版启动 capabilities 均包含 Project 能力。
- 不是安装版完全没有分页或压缩：Frame V1 和独立 get 已成功验证。
- 不能把所有 get 失败都归因于 resume：独立探针中 acquire 失败没有破坏 observer，get 仍成功；生产中的 get 原始行超限还需定位具体上游消息。

## 本次修复与验证

- 原生 resume 增加 excludeTurns=true；49 项 Codex Provider 单元测试通过。构建后的真实 Provider 对第一条大历史返回 ACQUIRE OK、GET(limit=20) OK（602 个 item）。没有发送用户消息。
- Gateway 新增 conversation.resume；Host 复用既有 acquire/get，返回 interactionAcquired、可选 interaction/interactionError、可选 history/historyError。history 直接使用 ConversationGetResponse，保留 snapshot cursor 和 pageInfo。Provider Protocol 继续保留独立方法，不复制映射或传输裁剪。
- Remote 先消费交互结果，再消费首屏并自动拉完全部历史；每页初值 20，缩页梯度 20→10→5→1。首屏为空但存在时不补 get，交互失败且无历史时才走只读 fallback；获取成功后不再续租。event window 在合并请求前建立，旧 epoch 结果不能安装。
- 能力未加载时等待 describe；binding 纳入 capabilitiesLoaded，修复同 revision 完整能力到达后无法 reload 的问题。
- 23 项 Host manager/gateway 集成测试、17 项 protocol:check、88 项 Remote 定向测试及 257 项 Remote 全量测试通过；Remote 对改动文件的 analyze 无问题。新增用例覆盖合并首屏复用、空历史不重复请求、交互失败后的只读 fallback、首屏超限缩页、resume 期间实时事件及能力恢复。
- 生成 Gateway Rust/TypeScript/Dart SDK，并同步 Remote 的 Gateway Dart SDK。新版 Remote 合并入口需要新版 Host；当前未执行覆盖安装。项目与会话列表已补上 capability/Ready 恢复后的加载路径。

## 下一步排查与验证计划

### 安装版历史读取导致 Server 重启（2026-09-06）

**证据：** 模拟器持久日志在 00:07:34、00:07:41 先记录交互权获取成功，随后首个历史页返回 `provider_protocol_error: App Server physical line exceeds 16777216 bytes`。00:36:40 的另一次尝试先返回 `conversation_write_conflict`，只读 get 仍触发相同超限。交互权冲突与历史超限是不同结果，不能只看合并入口 `conversation.resume` 的名字就归因于原生 resume。

同一安装版 Provider、相同 CLI 0.153.1、相同会话，移除父进程的 `CODEX_INTERNAL_ORIGINATOR_OVERRIDE` 后独立探针稳定复现：instance 返回的 Harness 缺失 version，get 超限，stderr 为 `Codex server App Server failed: App Server physical line exceeds 16777216 bytes`。探针只读取历史，不发送 turn。

| 原生请求/环境 | 实测结果 |
| --- | --- |
| Codex 工具环境 initialize | `Codex Desktop/0.153.1 ...` |
| 移除 originator override，使用 Host 的 clientInfo.name | `code-pet/0.153.1 ... (code-pet; 0.1.0)` |
| thread/read(includeTurns=false) | 919 字节 |
| thread/turns/list(notLoaded, limit=10) | 2,180 字节 |
| thread/turns/list(full, limit=10) | 24,756,517 字节 |

**根因与影响：** `client.rs::harness_version_from_user_agent` 原先只认识带 codex 的 product，遗漏客户端名 `code-pet`。版本解析为 None 后，`provider.rs::load_conversation_turns` 走旧版 full-turn 路径。16 MiB 上游物理行限制因此被触发；reader 调用 `SessionInner::fail` 主动关闭子进程，SDK 心跳随后重新启动 Harness。没有证据显示此例是 Codex 原生 panic；触发进程关闭的是 Provider 的超限处理。Remote 的 wire-frame 缩页无法修复上游 reader 已终止的问题。

**修复与涉及模块：** `client.rs` 复用 initialize 的客户端名常量识别 `code-pet/<native-version>`，保留 CLI/Desktop 兼容且不误取 Shell 或括号中的客户端版本；无需修改分页上限或 Remote。binary fixture 与 `provider_vertical.rs` 补充正常 Host userAgent，检查请求实际进入 `notLoaded` + `thread/items/list` 路径。

**回归验证：** 新 binary 测试修复前失败，证明确实缺少 item 请求；修复后 `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex` 为 49 个单元测试、40 个集成测试通过，1 个依赖外部安装的原有测试忽略。移除 override 的真实 Provider 完整翻页：第一会话 2 页、669 个投影 item；第二会话 2 页、1,614 个 item；之后同一 Server 的 project.list 仍成功。每个 Provider 页仍按 20 turns 请求，上游逐 item 分页。诊断只记录方法、字节数及计数，不保存会话正文。

Host release app/DMG 已重建；从最终 `.app` 内提取的 Provider 重复上述完整翻页验证通过，并断言没有 `history-not-loaded` 占位项。应用签名验证和 DMG 校验通过；产物 `CodePet-Host-0.1.4-macOS-arm64-codex-history-fix.dmg` 同时包含 Host Runtime 启动状态修复，未替换用户正在运行的 Host。

**回归防线：** 真实程序探针必须至少覆盖正常 Host 环境和 Codex 工具继承环境；不能用只在工具环境成功的首屏结果证明安装版完整历史可读。Harness 版本条件的 fixture 必须覆盖上游以 clientInfo.name 作为 product 名的响应。首次引入该解析缺口的提交未确认。

上述版本识别修复之后，继续完成了[单条错误与共享 Server 的隔离](codex-request-errors-must-not-stop-server.md)：超限、坏响应、RPC timeout 与事件编码失败不再触发服务终止。当时版本识别用于选择分页路径；后续已取消该分支。错误隔离继续保证异常数据不能拖垮其他会话。

### App Server 生命周期与写入权释放（2026-09-05 实测）

- 本机 `codex 0.153.1` 生成的协议提供 `thread/unsubscribe`，状态为 `unsubscribed`、`notSubscribed`、`notLoaded`，没有独立的立即卸载接口。对应版本的[官方说明](https://github.com/openai/codex/blob/rust-v0.153.1/codex-rs/app-server/README.md#unsubscribe-from-a-thread)规定最后一个订阅取消后，连续无订阅且无活动 30 分钟才卸载；该版本的 `thread_unsubscribe_keeps_thread_loaded_until_idle_timeout` 测试也明确验证取消订阅后仍在 loaded 列表。
- 双进程原生探针：A resume 成功，B resume 报 writer conflict；A unsubscribe 返回 unsubscribed 后，B 在约 1 秒内的五次重试仍冲突，而 A 的 project/list 成功、进程继续存活。没有发送用户消息。临时证据为 `/tmp/codepet-investigation/unsubscribe-results.jsonl`。
- 给本机可执行文件传入 `-c thread_unload_delay_secs=0` 后重复探针，仍未观察到立即释放。上游 main 的新文档支持该配置，但不能据此认定已安装版本支持；对应结果在 `unsubscribe-immediate-results.jsonl`。未实测等待完整 30 分钟。
- 排查时的旧实现通过关闭单会话进程释放 writer；这不是原生 unsubscribe 的要求。后续用户明确选择连接级共享 Server，已删除 per-conversation close/renew/reaper。
- 生命周期已按 [连接架构](../10-architecture/remote-and-provider-connections.md) 改造：每实例一个 Server，SDK 心跳携带客户端集合；最后连接离开停止 Server，退出详情不释放。binary fixture 已验证 64 会话同 PID、两端保留/最后断开/重连、审批和终态隔离。无需依赖 unsubscribe 的延迟卸载行为。

### 详情与项目恢复

- 覆盖安装后验证实际配置选择、writer 释放和两个真实会话的完整历史；真实探针已确认当前 CLI 的 metadata-only resume 可用，其他 CLI 版本仍需各自验证。
- 模拟器验证能力摘要到完整描述时不闪现“不支持”，保持现有路由与旧请求丢弃行为。
- 项目/会话能力恢复或 Provider Ready 后自动补拉首屏；回归覆盖多 Provider 状态更新、旧 generation 丢弃，以及初次连接尚不可用的延迟加载。运行版仍需验证实际项目入口。
- 对生产 get 请求补充仅含 method/id/bytes/generation 的 reader 诊断，识别是 metadata、turn/item 页还是通知超限；不要记录内容。两条历史已通过正常 Host 环境及最终打包 Provider 的完整翻页探针；安装后的 Remote 交互仍需验证。
- UI 验证覆盖错误提示、加载态、项目入口、返回重开、焦点与窄屏布局。恢复 Provider 后重复打开大历史，确认项目和其他会话仍可用。

## 未知项

- 本次安装版 get 超限已通过相同二进制、正常 Host 环境与原生请求对照定位；其他 CLI 版本及单个 item 自身超限的场景仍需独立验证。
- 项目与“不支持”提示的代码路径已确认，但当前日志未保存每次能力状态，仍需可控事件时序回归。
- 引入提交未确认；2026-09-06 修复已通过完整翻页探针，新安装包覆盖后的真实 Remote 验收尚未执行。
