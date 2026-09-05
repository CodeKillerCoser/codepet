# 在 Conversation v1 中用判别联合约束工具载荷所有权

## 背景

2026-09-04 对本机历史对话的黑盒采样显示，40 turns 的 `conversation.get` 响应包含 2,350 个 items，序列化后约 15.1 MB。870 条 command 的命令原文同时存在于 `ConversationItem.contents`、`ToolInvocation.input.command` 和 `ToolInvocation.command.command`；801 条 output 同时存在于 `ConversationItem.contents` 和 `ToolResult.content`。仅逐字相同的额外副本就有 6,129,554 bytes，占该响应约 40.5%。

这不是单个 Mapper 偶然多写了一次。Provider v1 允许 `contents`、`input`、`rawInput`、`command`、`result.content`、`structuredContent` 和 `error.message` 同时持有同一事实，却没有表达互斥关系。Codex 与 OpenCode 选择双写，Claude 选择主要写入 tool，说明不同实现都能合法地产生不同权威位置。只在投影器或 Remote 去重既不能阻止 Provider stdout frame 先超限，也不能防止副本在截断、脱敏或增量同步后发生漂移。

工具详情当前在消息列表内展开还是在抽屉展示只是 UI 布局选择。协议应表达 Provider 实际提供的领域事实和载荷完整性，不应为抽屉单独建立详情 DTO、预览副本或加载语义。

## 决策

继续修改 Provider v1 与 Gateway v1，不提升版本号。项目尚未现场发布，当前收益足以接受同步更新 Host、内置 Provider 与 Remote 的不兼容改动。

`ConversationItem` 改为由 `kind` 判别的封闭联合。message、reasoning、tool/command、file change、approval 与 unknown 各分支只声明自身合法字段；不能再用一个包含大量 optional 字段的对象表达互斥领域状态。unknown 分支只携带安全的通用元数据，不携带原生 Provider payload。

工具调用只拥有一份输入和一份结果：

- `CommandToolInput` 表达真正的命令执行，拥有 `command`、`cwd` 和可选 shell/actions。工具内部碰巧调用 shell 不足以把它归为 command。命令过长时可对 `command` 做 UTF-8 安全的 head-tail 截断，并在 input 上携带与内容块同构的截断元数据。
- `StructuredToolInput` 表达已可靠解析的 JSON 参数。
- `OpaqueToolInput` 表达无法可靠理解语义的原始输入。输入过大与输入不透明是两个维度；大 JSON 仍是 structured，只对其内容执行预算策略。
- 三种输入必须通过 `oneOf` 与 singleton `kind` 互斥，不再并列提供 `input + rawInput + command` 三个完整载荷位置。

工具结果使用 success/failure 判别联合。内容块是输出正文的唯一所有者；结构化 JSON 作为一种内容块表达，不再同时序列化为 `content` 与 `structuredContent`。failure 的 `error.message` 只保存有界的人类可读摘要，stdout/stderr/诊断正文仍分别属于内容块，不能把完整输出复制进错误消息。exit code、process id 等执行结果属于 outcome，不属于输入。

消息正文和工具结果共用同一套稳定内容块身份与截断元数据。截断元数据位于被截断的内容块或 tool input，至少包含 `originalBytes`、`retainedBytes` 和 `strategy`；`omittedBytes` 可由协议要求显式提供或从前两者推导，但所有生成 SDK 必须保持同一选择。没有截断元数据即表示对应载荷完整。`head-tail` 用于命令、日志、Shell 和测试输出，结构化输入可以使用 `structural-preview`，不能仅用一个布尔值让客户端猜测丢失多少内容。

内容截断只能作为原生 harness 数据映射为 Agent 领域对象时的显式内容策略，不能作为传输层为了通过帧上限而执行的降级。共享 Provider SDK Runtime 只负责序列化、压缩、最终 frame 检查和稳定错误，不理解或修改 tool/message/preview 等业务字段。`conversation.get` 必须真实遵守 cursor/limit；最终帧超限时返回 `provider_response_too_large`，由最终调用方保持 cursor 并缩小 limit，Host/Gateway 不做二次有损投影。

`conversation.get` 的 item/content identity 继续支持 snapshot 与 `turn.outputDelta` 去重。客户端提交集合必须遍历每个判别分支的 canonical content blocks；协议不能要求客户端通过文本相等判断副本。详情抽屉、列表内折叠和独立详情页都读取同一 canonical blocks，并根据截断元数据提示内容不完整。

## 备选方案

- 只在 Codex Mapper 停止双写：可快速降低当前样本，但 Schema 仍允许其他 Provider 或未来代码恢复双权威，不采用为最终方案。
- 在 Host 投影器或 Remote 按文本相等去重：发生得太晚，Provider stdout 可能已经超限；文本相等也不是稳定身份，不采用。
- 保留旧字段并约定“通常互斥”：无法被生成器、编译器与 schema fixture 强制验证，不采用。
- 因抽屉需要详情而保留第二份正文：抽屉只是 canonical 内容的另一种视图，不构成协议事实，不采用。
- 整体采用 ACP：ACP 的消息内容块和 tool call 结构可作设计参考，但 Code Pet 仍需 project、conversation 操作、常驻 harness server、Provider instance 与 Gateway 路由语义，不采用为核心协议。
- 升级为 v2 并长期双栈：当前没有现场兼容负担，会增加无收益的迁移层，不采用。

## 取舍理由

协议级判别联合使非法组合无法生成，能同时解决体积、权威漂移和跨 Provider 行为不一致。内容级截断元数据让任意 UI 都能诚实展示不完整状态，而不把展示位置编码进领域协议。保持 v1 可避免为尚未发布的结构维护双栈；代价是 Code Pet Host、三个内置 Provider、生成 SDK 与 codepet-remote 必须作为一个变更集同步完成。

## 影响范围

- `protocol/provider/v1`、`protocol/gateway/v1` 及 Rust/Dart/TypeScript 生成物需要同步修改；生成器必须继续保留 `oneOf` 的 discriminator 互斥语义。
- Codex、Claude 与 OpenCode Mapper 必须只填充一个输入 variant 和一个 outcome，不再向顶层 contents 写工具正文副本。
- Provider mapper 可按明确内容策略生成带 truncation metadata 的领域内容；Provider SDK Runtime 只能对最终编码 frame 执行 16 MiB 检查，禁止按块截断、响应软预算或 History omitted 等传输降级。
- Host/Gateway 校验和投影必须按 item variant 遍历 canonical contents，并保持 Provider-only extension 不出 Host。
- codepet-remote 的解码、提交 contentId 集合、delta 合并、工具展示和分页策略必须读取新联合结构；抽屉交互本身不要求协议特例。
- `conversation.list` 的 title/preview 是独立问题。相等样本证明存在元数据冗余，但当前实测列表未达到 16 MiB；不要把它与工具正文治理混为同一个根因。

## 后续观察

- 用 2,350 items、870 command、801 output 的等价 fixture 锁定“无完整正文副本”和整页大小。
- 用 65–142 KiB 的中等输出以及数百个 8–20 KiB 输出验证 Provider mapper 的显式内容策略与 truncation metadata，不再建立传输层累计页预算。
- 记录 method、route、cursor、limit、JSON 字节数、最终 wire 字节数、encoding、按 kind 数量和截断字节数，不记录正文。
- 协议实现完成后，分别验收 Provider frame、Host/Gateway 投影和 Remote snapshot/delta，不以 UI 能打开作为唯一通过标准。
