# OpenCode 用户输入消息流与正文/推理身份碰撞

## 证据与影响

2026-11-08，Remote 截图显示队列出队后没有用户气泡，回复正文进入“思考过程”。核对 OpenCode 官方 v2.2.6 的 `packages/core/src/session/runner/publish-llm-event.ts`，`text`、`reasoning` 分别创建 fragments，每个 fragments 从 0 独立计数。原适配器将两个增量映射成同一个 `ordinal:0` item；历史转换又把 content 数组的全局下标当作 ordinal，因此流式与历史身份也不一致。

用户输入是另一处遗漏。v1.18.25 的 `session.next.prompted` 含原生 `messageID` 与 prompt，并在 message-updater 中写入用户历史；`prompt.admitted` 只是接纳。v2.2.6 的 `session.inbox.enqueued` 含 payload，`session.inbox.delivered` 才将它送入历史，二者使用同一 inboxID。Provider 之前忽略这些事件。普通 turn.start 的调用方能取得 userItem 回执，Host 队列调用方则没有对应 Remote RPC 回执，暴露了消息流缺口。不能据此让 Host 单独制造用户消息或让 Remote 每次轮询队列都补拉整页历史。

引入提交未确认。此前兼容测试错误假设 ordinal 为全局下标，真实模型 fixture 只返回正文，未覆盖同轮推理与正文。

## 修复与模块

- `compatibility.rs`：2.x 正文与推理分别使用 `text:ordinal:N` / `reasoning:ordinal:N`；历史按类型分别计数，工具保留原生 ID。1.x 原生 textID/reasoningID 不改写。
- `provider.rs`：1.x 处理 prompted；2.x 缓存 enqueued 用户文本，仅在 delivered 后生成标准 conversation.item.upserted。关联当前 Provider turn，采用原生消息 ID，普通响应与消息事件可去重。cancelled 清除缓存，不产生气泡；stop/shutdown 清理缓存，缓存有界。无可关联 turn 或缺少 enqueued 时发历史失效提示，不虚构消息。
- `native_turns.rs` / 本地模型脚本：同时输出 reasoning_content 与正文，验证原生流中的用户输入及引导输入各一次、回执身份一致、所有 delta 内容 ID 均存在于历史。
- Remote 独立仓库：队列操作改为条目右侧菜单；旧连接缓存若已有混合内容 item，则按内容类型分别投影，不再把 text 放进推理。

## 验证

在隔离 XDG 配置及 localhost 模型服务下运行，不访问用户模型配置或消费远程模型额度：

```sh
cargo test --offline --manifest-path crates/Cargo.toml -p codepet-provider-opencode --all-targets
python3 scripts/test_opencode_observation.py --executable /opt/homebrew/bin/opencode --remote-turns
python3 scripts/test_opencode_observation.py --executable /tmp/codepet-opencode-1.18.25/opencode --remote-turns
```

2.2.6：批准、拒绝、再批准三轮通过；1.18.25：连续批准两轮通过。两版都在首轮执行 steer，并验证两条独立原生用户消息事件；成功轮同时有推理/正文且身份互异，历史可对齐。1.x 的拒绝完成仍沿用既有已知限制，未把它宣称为已通过。单元测试覆盖取消、非 delivered 不上屏、旧版 prompted 和同编号跨类型碰撞。

## 回归防线与剩余范围

协议身份必须来自各原生事件的作用域，不能从全局数组位置或文本相等推断。新兼容测试应含至少一轮推理与正文，并比较流式/历史身份；接纳与进入消息历史须分开验证。队列 UI 与消息历史各自投影，只有原生消息事件才能确认用户气泡。

本次未验证 Windows/Linux、所有其他 1.x 小版本、外部客户端在订阅建立前已入队的输入、原生媒体附件。缺少 enqueued 的 delivered 仅触发历史失效；这是恢复路径，不是制造用户消息的依据。安装现有桌面 App 时必须同步更新其 OpenCode Provider，单独更新 APK 无法修正上游身份碰撞。

原生源码：[2.2.6 事件发布器](https://github.com/anomalyco/opencode/blob/v2.2.6/packages/core/src/session/runner/publish-llm-event.ts)、[2.2.6 inbox](https://github.com/anomalyco/opencode/blob/v2.2.6/packages/core/src/session/inbox.ts)、[1.18.25 消息投影](https://github.com/anomalyco/opencode/blob/v1.18.25/packages/core/src/session/message-updater.ts)。
