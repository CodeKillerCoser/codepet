# Conversation 工具载荷唯一所有权

## 规则

Agent v1 的 `ConversationItem`、tool input 和 tool outcome 必须使用带 singleton `kind` 的 `oneOf` 判别联合表达互斥状态；Provider v1 与 Gateway v1 只能引用并重新导出这一份定义，不得复制领域结构。一个领域事实只能有一个完整载荷所有者：工具命令只存在于 command input，结构化参数只存在于 structured input，不透明参数只存在于 opaque input；工具输出只存在于 outcome 的 canonical content blocks。不得在 `ConversationItem.contents`、tool input、command facet、result text、structured result或 error message 之间复制完整正文。

内容是否截断与内容语义正交。所有 item variant 允许可选开放对象 `_meta`；只有 `kind: tool` 先检测超大文本，每字段 256 KiB，UTF-8 安全 head-tail，截断当场写入 `_meta.truncations` 的字段路径、原始/保留字节数和策略。structured JSON 仅改超大字符串叶子，保留结构。command、file-change、message 等独立 item 暂不检测。旧 input/content block 的 truncation 字段继续兼容读取，新策略只在 item `_meta` 记录；UI 展示位置不改变 canonical wire object。详见 [文本与分页规约](provider-item-text-and-pagination.md)。

内容策略必须在 Provider 把原生 harness 数据映射为 Agent 领域对象时完成；共享 Provider SDK Runtime 不实施内容预算或页软预算，只把 wire message 序列化一次，自动选择 raw/zstd，并对最终 encoded frame 执行统一 16 MiB 检查。超限返回稳定错误，由最终调用方缩小分页；Host/Gateway 不做二次有损投影。继续沿用业务协议 v1，transport 使用独立的 Provider Frame V1。

## 适用场景

- 修改 `ConversationItem`、工具调用、工具结果、内容块或截断字段。
- 编写 Codex、Claude、OpenCode 或第三方 Provider 的历史 Mapper。
- 修改 Provider mapper 的内容策略，或 Provider SDK Runtime 的 framing、压缩和 oversized error。
- 修改 Host/Gateway 的 conversation 校验、投影、snapshot/delta identity。
- 修改 Remote 的历史分页、工具展示、contentId 提交或 delta 合并。

## 反例

- command 同时写入 `contents.command`、`input.command` 和 `command.command`。
- stdout 同时写入 item contents 和 tool result；失败时再把全文复制进 `error.message`。
- 同一 JSON 同时存在于 text content 与 `structuredContent`。
- 因 JSON 很大就把 structured input 改称 opaque input。
- 只在 Remote 通过字符串相等去重，或只截断顶层 item contents。
- 为抽屉单独增加详情 payload 或第二份完整 output。
- 单独放宽 Host frame limit，而 Provider 仍在更早的 stdout 写入阶段失败。

## 推荐做法

- 先修改 `agent/v1` schema 并生成所有 SDK，再让 Provider/Gateway 各自引用生成类型；不在边界 schema 或业务 crate 手写临时并行 DTO。
- command/structured/opaque input 与 success/failure outcome 分别使用封闭联合，编解码负向 fixture 必须拒绝同时出现两个 variant 的对象。
- `kind: tool` 内的 command/structured/opaque 输入保持原语义 variant；截断记录归 item `_meta`，不能因大 JSON 改成 opaque。
- 普通消息与 tool outcome 复用同一内容块语义；每个 canonical block 保持稳定 contentId。
- error message 保持短摘要；stdout、stderr、diff 和 JSON 结果分别是内容块。
- Provider mapper 调用 SDK 可选 `truncate_tool_item_text` helper，在 item 生成时记录 `_meta.truncations`；SDK Runtime 不自动应用策略，禁止为通过 frame 上限修改领域字段。
- Remote 从联合分支遍历 canonical contentId，不能依赖旧顶层 contents fallback 作为长期行为。
- 对 title/preview 单独设列表元数据预算；两者相等的问题单独治理，不用它解释 conversation.get 的工具正文膨胀。

## 来源

- `../50-decisions/conversation-item-and-tool-payload-ownership.md`
- `../../reports/conversation-payload-size-analysis-2026-09-04.html`
- `../../protocol/agent/v1/schema.json`
- `../../sdk/rust/codepet-provider-sdk/src/lib.rs`
- `../../crates/providers/codepet-provider-codex/src/mapper.rs`
- `../../crates/providers/codepet-provider-opencode/src/mapper.rs`
- `../../crates/providers/codepet-provider-claude/src/provider.rs`

## 验证方式

- `npm run protocol:check` 验证 v1 schema、fixtures、生成物 freshness，以及所有联合的 discriminator。
- 负向 schema/codec fixtures 验证 command+structured、success+failure、顶层工具 contents+outcome 正文等非法组合被拒绝。
- Provider Mapper 测试断言一条 command 和 output 各只有一个 canonical 全文位置，且三个内置 Provider 产生相同领域形状。
- 超过 16 MiB 的原生 tool 输出验证文本策略、字段路径和 UTF-8；多个中等字段不触发整页预算，非 tool 文本保持完整。SDK Runtime 独立验证 raw/zstd、最终 frame 上限、稳定超限错误和继续服务。
- Host/Gateway 测试验证所有 item variant 的 route、contentId 与 Provider-only extension 边界。
- Remote 测试验证 committed contentId、snapshot+delta 去重、截断提示和多页顺序；抽屉与列表内摘要读取同一 canonical blocks。
