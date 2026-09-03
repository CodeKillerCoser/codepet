# Provider 会话内容标识规约

## 规则

Provider 输出的 `ConversationItem.resource.nativeResourceId` 和 `ConversationContent.contentId` 必须在会话范围内稳定且唯一。当原生 Provider 的 part/content ID 只在单条消息内唯一时，适配器必须将其父 message/item ID 纳入标识。历史快照与实时 `turn.outputDelta` 必须使用完全相同的 item/content ID 派生规则。

## 适用场景

适用于所有 Provider 的 `conversation.get`、历史投影、流式 delta、item upsert 与 Gateway 转换。新增内容类型、接入新 Agent，或调整原生事件映射时，都要先确认原生 ID 的唯一性作用域，不得默认它在整个会话内唯一。

## 反例

OpenCode 的不同 assistant message 都可以包含原生 part ID `text-0`。如果适配器仅派生 `text-0:text`，Host 会以 `duplicate_conversation_content` 拒绝整份历史。另一个反例是快照使用 `<messageId>:<partId>:text`，delta 仍使用 `<partId>:text`；Remote 无法将实时文本与权威快照合并，会重复显示或遗留过期内容。

## 推荐做法

- 优先直接使用原生会话级唯一 item ID。
- 原生 part ID 仅局部唯一时，使用 `<messageId>:<partId>` 作为 item identity，再按语义位置派生 `<itemIdentity>:text`、`<itemIdentity>:summary:0` 等 content identity。
- ID 只依赖原生稳定标识和语义位置，不得依赖文本内容、加载顺序或随机值。
- 同一原生内容在历史快照、item upsert 和 delta 中必须收敛到相同 ID。
- Host 保留会话范围的重复 ID 校验并失败关闭，不在 Host 中根据文本修补 Provider ID。

## 来源

2026-09-03 真实 OpenCode 1.18.25 会话 `Greeting` 返回两条不同 assistant message，它们的 text part ID 均为 `text-0`。`codepet-provider-opencode` 原实现仅生成 `text-0:text`，触发 Host 的 `Provider conversation history contains a duplicate content identity`。引入历史确认该规则由提交 `3b34901c` 引入。

## 验证方式

- Provider mapper 测试必须构造两条共用同一原生 part ID 的不同 message，确认 item/content ID 不重复。
- Provider 纵向测试必须确认 text/reasoning delta 同时包含 assistant message ID 和原生 part ID，且 `contentId` 由该 `itemId` 稳定派生。
- Host 保留空 ID、重复 item ID 和重复 content ID 的边界校验测试。
- 真实 Provider smoke 应重新加载曾触发冲突的会话，确认 Host 不再返回 `duplicate_conversation_content`。
