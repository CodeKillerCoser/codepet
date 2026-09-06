# Codex Remote 会话自动标题

## 规则

Codex Provider 接受用户发送后，若原生会话没有 name，异步生成并保存摘要标题；标题任务的失败不能改变发送成功结果。生成任务不得混入用户会话、历史或事件列表。

## 适用场景

`codepet-provider-codex/src/provider.rs` 的 turn.start、`client/title.rs` 的原生调用，以及 `provider/server_events.rs` 的事件转发。首次发送和继续未命名旧会话使用同一机制。已有 name 的会话跳过。

## 反例与证据

Remote 原先只执行 thread/start 和 turn/start。2026-09-06 核对的 HTTP/2 与 hello 会话 name 均为空，界面标题来自 preview。当前桌面安装包有独立的 ThreadMetadataGenerationService：创建临时线程生成结构化标题，再调用 thread/name/set；它不是原生 thread/start 的必然副作用。

## 推荐做法

- 成功受理后才调度，不 await 标题推理再返回 accepted。每实例最多 2 个标题任务并发；同一运行期每个会话最多尝试一次，失败记录带 conversation_id 的诊断，不自动重试或重发用户任务。运行时重建清理尝试状态，已有原生 name 仍会跳过。
- 复用同一个 App Server，在 ephemeral、read-only、never-approval 的临时线程生成。禁用 hooks、MCP、apps、plugins、shell tool、multi-agent 和 web search；提示只把原始输入当作待总结数据。使用当前选择的模型及 low effort，这是一项额外模型请求。
- 优先使用原生 preview，空时使用本次输入，最多 6,000 Unicode 字符。通过 outputSchema 约束 JSON title，客户端再验证非空、单行、最多 60 字符。
- 创建前订阅事件，处理 item/completed 和 turn/completed 早于 RPC response 的顺序。等待推理最多 60 秒（原生 RPC 各有已有的请求超时）；错误后尽力 interrupt，并在结束时 unsubscribe 临时线程。
- client 在收到 ephemeral ThreadStarted、以及收到创建响应时登记临时 ID。Server 事件泵过滤这些 ID 的事件，正常会话继续转发。保存原会话标题后的 thread/name/updated 仍按既有 conversation.upserted 通路同步到 Host/Remote。
- 生成前及保存前读取原生 name；观察到用户或其他客户端已经命名就放弃自动结果。

## 边界

本机生成的 ThreadNameSet 接口没有 expectedName/CAS 参数。两次检查覆盖推理期间的命名变化，但最后一次读取与写入之间极短的跨客户端并发窗口没有原子保证；若将来要求严格的并发不覆盖，需要原生提供条件写入能力，不能宣称本地锁能锁住外部客户端。当前失败仅诊断，不显示成发送失败。

## 来源

[Remote resume 与标题排查](../40-runbooks/codex-resume-no-output-and-incomplete-history.md)。原生参数核对来自本机 `/Applications/ChatGPT.app/Contents/Resources/codex app-server generate-json-schema --out /tmp/codepet-title-schema`；桌面编排核对来自 app.asar 中 ThreadMetadataGenerationService 与结构化生成函数。未以私有桌面函数作为运行依赖。

## 验证方式

`cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex`：55 个单元测试、45 个纵向测试通过，1 个依赖真实 Codex 环境的测试默认忽略。新增回归覆盖已有名字跳过、推理期间改名、先事件后响应、Unicode 输入限长、无效 JSON、超时取消与清理；真实 Provider 进程加原生 fixture 验证只保存一次、标题事件到达、生成失败不阻断后续发送、临时会话事件不泄漏。

未调用格式化工具。尚未用真实模型跑标题质量测试，也未重新打包或替换正在运行的桌面 Host；部署时需更新桌面包中内置 Codex Provider，Remote 不需要新的协议或 UI 改动。
