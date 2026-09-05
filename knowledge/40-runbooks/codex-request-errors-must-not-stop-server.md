# Codex 单条请求错误误关共享 Server

本文保留 `server-error-isolation-fix` release 的排查与验证证据。其后按用户约束删除原生 JSONL/turn 字节上限、版本驱动 item 分页和 Provider 内缩页；当前使用 full turns 透传分页及 tool item `_meta` 截断，见 [当前规约](../60-rules/provider-item-text-and-pagination.md)。本记录的 stage=app-server 超限与 item fallback 仅适用于当时版本；错误隔离边界继续有效。

## 现象

2026-09-06 Remote 读取大历史后，Provider 管理的 Codex App Server 退出并由心跳重启，其他会话也随之不可用。此前 Harness 版本识别修复避免了错误的 full-turn 路径，但没有隔离单条消息失败与进程故障。

## 证据

- 安装版同环境探针中，原生 `thread/turns/list(itemsView=full, limit=10)` 返回 24,756,517 字节；Provider stderr 随后报告 physical line 超过 16 MiB。详细环境对照见 [历史读取排查](codex-detail-size-and-capability-loading.md)。
- 修复前 `client.rs` 把 reader/handle_message 的所有错误和单次 RPC timeout 都交给 `SessionInner::fail`，后者关闭共享 child 并拒绝全部 pending。stderr 长行也会触发同一路径。
- SDK `StdioEventWriter::publish` 未区分写出前的编码失败与实际 stdout write/flush failure，事件超限也触发全局 cleanup。Codex 事件转发对 publish error 和无法路由队列超限同样关闭 Server。
- 新 fixture 故意发送 id 位于末尾的超大响应、非法 JSON、非法 envelope 和原生 RPC error；每次错误后另一会话仍读取成功，`process/start` 记录保持同一 PID。并发测试同时验证 B 的响应和事件不被 A 的错误打断，服务端请求与客户端请求的同值 id 不串线。

## 根因

请求、消息和日志的失败被当作连接终止。共享 Server 改造后，一个会话的读取错误因此扩散至所有已 resume 会话。Turn 是一次交互执行，可含大量工具与消息 item；20-turn 页不等于 20 条消息。分页选择和错误隔离必须分别成立，不能依赖正常历史恰好不超限来保证进程存活。

## 引入历史

首次引入终止策略的提交未确认；2026-09-06 的版本识别修复并未修改该策略，本修复补齐故障边界。

## 当时的修复方案与涉及模块

- `client/framing.rs`：原生 JSONL 有界读取。超长行流式跳过正文、提取 envelope（包括正文后的 id）并 drain 到换行；只拒绝该消息，保留下一个 frame。stderr 仅保留前 64 KiB，继续 drain 后续内容。
- `client.rs`：已识别的错误响应只完成对应 pending；无可用 id 的坏消息记录诊断，其他请求继续。坏 server request 使用自己的 id 返回小错误，不能碰同值 client pending。RPC timeout 只返回结果未知，晚到响应丢弃，不自动重发写操作。
- `mapper.rs`、`provider.rs`：原生尺寸错误映射 `provider_response_too_large`，details 带 `stage=app-server`、bytes、limit；timeout 为 `provider_request_timeout`。item 读取真实失败向上传递，不伪装成成功的 unknown 历史。旧 CLI 的能力探测兼容分支保留。
- `sdk/rust/codepet-provider-sdk/src/stdio.rs`：事件编码先于写入，编码拒绝不标记 output unavailable，也不发送 terminal 信号。
- `provider/server_events.rs`：事件编码/映射错误记录后继续；无法路由队列保留最新 4,096 条，超出部分丢弃并记录数量。实际 write/flush 失败仍按连接断开清理。

原生 stdout EOF/I/O 失败、初始化无法就绪、显式 stop/shutdown 和连接心跳生命周期仍负责回收进程。业务错误不触发这些路径。

## 当时 release 的验证结果

- `cargo test --manifest-path crates/Cargo.toml`：200 项通过、2 项原有外部依赖测试忽略，其中 Codex 52 单元 + 43 集成通过。覆盖原生 frame 恢复、并发响应/事件隔离、stderr、timeout/晚到响应、真实二进制 PID 保持、item 读取错误可见、事件编码错误后继续转发；其他 Provider 和 Host 回归一同通过。
- `cargo test --manifest-path sdk/rust/Cargo.toml -p codepet-provider-sdk`：25 项通过。超限事件未写出任何字节、未发 terminal，后续合法事件仍可解码；已有真实 Broken pipe 全局清理测试保留。
- 最终 release 包内 Provider 与本机原生 Codex 0.153.1：用独立诊断进程的未知 originator 名有意进入旧 full-turn 分支，再次收到 24,756,517 字节响应。这次返回 `provider_response_too_large`，随后 `project.list` 成功，原生 PID 65712 未变；仅探针结束时关闭该进程，没有发送 turn/resume。
- 正常 Host 环境的包内 Provider 完整读取两条历史，分别 2 页、669 item 和 2 页、1,614 item，无 `history-not-loaded` 占位项，后续项目请求成功。
- release app 签名、DMG 校验、`git diff --check` 通过。新产物 `CodePet-Host-0.1.4-macOS-arm64-server-error-isolation-fix.dmg` 包含此前两项修复；构建信息与验证日志放在 Downloads 的本次 release 目录，尚未替换运行中的 Host。

## 回归防线与规约

已同步 [共享 Server 生命周期规约](../60-rules/codex-provider-turn-writer-lifecycle.md) 与 [协议通道边界](../60-rules/protocol-layer-and-channel-boundaries.md)。以后新增大小限制、解析器、超时或事件队列时，必须验证错误后同一个 Server 的另一请求与后续事件仍可用；只验证返回错误不够。

## 未知项与风险

被拒绝的超大/坏事件及溢出的无法路由事件不能送达，不承诺自动重放；历史可通过后续 get 补取，未来若需要主动缺口通知，应单独设计协议。当时单个 item 超过原生上限仍会失败；后续已删除该上限，当前剩余限制是最终 Provider Frame V1，范围见上述规约。没有 id 的坏响应只能等对应请求超时。真实断连与显式 stop 的进程回收由原有纵向测试继续保护。
