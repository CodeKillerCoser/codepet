//! Generated CodePet Host ↔ out-of-process Provider plugin SDK.

mod generated;
mod stdio;

pub use generated::*;
pub use generated::ProtocolServer as Provider;
pub use stdio::{
    serve_stdio, serve_stdio_with_io, ProviderEventSink, StdioServerError,
    StdioServerOptions,
};

/// Bounded transport limit for Provider methods that return complete conversation histories.
/// Both the Provider process and Host reader must opt into this limit explicitly.
pub const MAX_CONVERSATION_HISTORY_JSON_LINE_BYTES: usize = 16 * 1024 * 1024;

const CONVERSATION_HISTORY_ENVELOPE_RESERVE_BYTES: usize = 64 * 1024;
const CONVERSATION_HISTORY_SOFT_BYTES: usize = 7 * 1024 * 1024;
const SEMANTIC_CONTENT_BYTES: usize = 128 * 1024;

/// Applies the Provider-side semantic and page budgets before stdout serialization.
pub fn fit_single_turn_conversation_history(
    _requested_limit: Option<u64>,
    mut response: ConversationGetResponse,
) -> ConversationGetResponse {
    apply_history_budget(&mut response, CONVERSATION_HISTORY_SOFT_BYTES);
    if serialized_history_bytes(&response) > 8 * 1024 * 1024 {
        apply_history_budget(&mut response, 5 * 1024 * 1024);
    }
    if !conversation_history_fits(&response) {
        apply_history_budget(&mut response, 0);
        response.conversation.preview = None;
        response.conversation.extension = None;
        if let Some(active_turn) = &mut response.conversation.active_turn {
            active_turn.display_summary = None;
            active_turn.extension = None;
        }
    }
    response
}

fn apply_history_budget(response: &mut ConversationGetResponse, limit: usize) {
    let mut remaining = limit;
    for item in &mut response.items {
        truncate_item_content(item, &mut remaining);
    }
}

fn truncate_item_content(item: &mut ConversationItem, remaining: &mut usize) {
    match item {
        ConversationItem::MessageConversationItem(value) => truncate_content_blocks(&mut value.contents, remaining),
        ConversationItem::ReasoningConversationItem(value) => truncate_content_blocks(&mut value.contents, remaining),
        ConversationItem::FileChangeConversationItem(value) => truncate_content_blocks(&mut value.contents, remaining),
        ConversationItem::CommandConversationItem(value) => truncate_tool(&mut value.tool, remaining),
        ConversationItem::ToolConversationItem(value) => truncate_tool(&mut value.tool, remaining),
        ConversationItem::ApprovalConversationItem(_) | ConversationItem::UnknownConversationItem(_) => {}
    }
}

fn truncate_tool(tool: &mut ToolInvocation, remaining: &mut usize) {
    match &mut tool.input {
        ToolInput::StructuredToolInput(value) => truncate_json_object(&mut value.value, &mut value.truncation, remaining),
        ToolInput::OpaqueToolInput(value) => truncate_text(&mut value.value, &mut value.truncation, remaining),
        ToolInput::CommandToolInput(value) => truncate_text(&mut value.command, &mut value.truncation, remaining),
    }
    match tool.outcome.as_mut() {
        Some(ToolOutcome::ToolSuccessOutcome(value)) => truncate_content_blocks(&mut value.content, remaining),
        Some(ToolOutcome::ToolFailureOutcome(value)) => {
            value.error.message = value.error.message.chars().take(512).collect();
            truncate_content_blocks(&mut value.content, remaining)
        }
        None => {}
    }
}

fn truncate_content_blocks(contents: &mut [ContentBlock], remaining: &mut usize) {
    for content in contents {
        match content {
            ContentBlock::TextContentBlock(value) => truncate_text(&mut value.text, &mut value.truncation, remaining),
            ContentBlock::ReasoningSummaryContentBlock(value) => truncate_text(&mut value.text, &mut value.truncation, remaining),
            ContentBlock::OutputContentBlock(value) => truncate_text(&mut value.text, &mut value.truncation, remaining),
            ContentBlock::ActivitySummaryContentBlock(value) => truncate_text(&mut value.text, &mut value.truncation, remaining),
            ContentBlock::EmbeddedResourceContentBlock(value) => truncate_text(&mut value.text, &mut value.truncation, remaining),
            ContentBlock::StructuredJsonContentBlock(value) => truncate_json_object(&mut value.value, &mut value.truncation, remaining),
            ContentBlock::ImageContentBlock(_)
            | ContentBlock::AudioContentBlock(_)
            | ContentBlock::ResourceLinkContentBlock(_) => {}
        }
    }
}

fn truncate_text(text: &mut String, truncation: &mut Option<ContentTruncation>, remaining: &mut usize) {
    let original_bytes = truncation.as_ref().map_or(text.len() as u64, |value| value.original_bytes);
    let limit = SEMANTIC_CONTENT_BYTES.min(*remaining);
    if text.len() > limit {
        *text = head_tail_utf8(text, limit);
        *truncation = Some(ContentTruncation {
            original_bytes,
            retained_bytes: text.len() as u64,
            strategy: ContentTruncationStrategy::HeadTail,
        });
    }
    *remaining = remaining.saturating_sub(text.len());
}

fn truncate_json_object(value: &mut JsonObject, truncation: &mut Option<ContentTruncation>, remaining: &mut usize) {
    let original_bytes = truncation.as_ref().map_or_else(
        || serde_json::to_vec(value).map_or(usize::MAX as u64, |bytes| bytes.len() as u64),
        |value| value.original_bytes,
    );
    let limit = SEMANTIC_CONTENT_BYTES.min(*remaining);
    if original_bytes > limit as u64 {
        let mut preview = JsonObject::new();
        for (key, entry) in value.iter() {
            preview.insert(key.clone(), entry.clone());
            if serde_json::to_vec(&preview).map_or(usize::MAX, |bytes| bytes.len()) > limit {
                preview.remove(key);
                break;
            }
        }
        *value = preview;
        let retained = serde_json::to_vec(value).map_or(0, |bytes| bytes.len());
        *truncation = Some(ContentTruncation {
            original_bytes,
            retained_bytes: retained as u64,
            strategy: ContentTruncationStrategy::StructuralPreview,
        });
    }
    let retained = serde_json::to_vec(value).map_or(0, |bytes| bytes.len());
    *remaining = remaining.saturating_sub(retained);
}

fn conversation_history_fits(response: &ConversationGetResponse) -> bool {
    serialized_history_bytes(response)
            <= MAX_CONVERSATION_HISTORY_JSON_LINE_BYTES
                .saturating_sub(CONVERSATION_HISTORY_ENVELOPE_RESERVE_BYTES)
}

fn serialized_history_bytes(response: &ConversationGetResponse) -> usize {
    serde_json::to_vec(response).map_or(usize::MAX, |payload| payload.len())
}

fn head_tail_utf8(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    if limit == 0 {
        return String::new();
    }
    let mut head_end = (limit / 2).min(value.len());
    while head_end > 0 && !value.is_char_boundary(head_end) { head_end -= 1; }
    let tail_budget = limit.saturating_sub(head_end);
    let mut tail_start = value.len().saturating_sub(tail_budget);
    while tail_start < value.len() && !value.is_char_boundary(tail_start) { tail_start += 1; }
    format!("{}{}", &value[..head_end], &value[tail_start..])
}

#[cfg(test)]
mod history_tests {
    use super::*;

    fn resource(id: &str) -> RoutedResourceId {
        RoutedResourceId {
            device_id: "device".to_string(),
            provider_plugin_id: "plugin".to_string(),
            provider_instance_id: "instance".to_string(),
            native_resource_id: id.to_string(),
        }
    }

    fn response(item_count: usize, text_bytes: usize) -> ConversationGetResponse {
        let conversation = resource("conversation");
        let turn = resource("turn");
        ConversationGetResponse {
            conversation: ProviderConversation {
                resource: conversation.clone(),
                project: None,
                title: "Conversation".to_string(),
                preview: None,
                status: ConversationStatus::Idle,
                permission_level: None,
                model: None,
                reasoning_effort: None,
                selection: None,
                workspace_root: None,
                created_at: None,
                updated_at: None,
                active_turn: None,
                extension: None,
            },
            items: (0..item_count)
                .map(|index| ConversationItem::MessageConversationItem(MessageConversationItem {
                    resource: resource(&format!("item-{index}")),
                    turn: turn.clone(),
                    conversation: conversation.clone(),
                    kind: MessageConversationItemKind::Message,
                    status: ConversationItemStatus::Completed,
                    role: ConversationItemRole::Assistant,
                    contents: vec![ContentBlock::TextContentBlock(TextContentBlock {
                        content_id: format!("content-{index}"),
                        kind: TextContentBlockKind::Text,
                        text: "x".repeat(text_bytes),
                        truncation: None,
                    })],
                }))
                .collect(),
            page_info: Some(PageInfo {
                next_cursor: Some("next".to_string()),
            }),
        }
    }

    #[test]
    fn limit_one_truncates_by_serialized_size_and_preserves_cursor() {
        let fitted = fit_single_turn_conversation_history(Some(1), response(600, 40_000));
        assert!(conversation_history_fits(&fitted));
        assert_eq!(fitted.items.len(), 600);
        assert_eq!(
            fitted
                .page_info
                .as_ref()
                .and_then(|page| page.next_cursor.as_deref()),
            Some("next")
        );
        assert!(serialized_history_bytes(&fitted) <= 8 * 1024 * 1024);
    }

    #[test]
    fn medium_output_uses_head_tail_with_exact_byte_counts() {
        let original = format!("HEAD{}TAIL", "x".repeat(142 * 1024));
        let mut value = response(1, 0);
        let ConversationItem::MessageConversationItem(item) = &mut value.items[0] else { unreachable!() };
        let ContentBlock::TextContentBlock(block) = &mut item.contents[0] else { unreachable!() };
        block.text = original.clone();
        let fitted = fit_single_turn_conversation_history(Some(10), value);
        let ConversationItem::MessageConversationItem(item) = &fitted.items[0] else { unreachable!() };
        let ContentBlock::TextContentBlock(block) = &item.contents[0] else { unreachable!() };
        assert_eq!(block.text.len(), SEMANTIC_CONTENT_BYTES);
        assert!(block.text.starts_with("HEAD"));
        assert!(block.text.ends_with("TAIL"));
        let truncation = block.truncation.as_ref().expect("truncation metadata");
        assert_eq!(truncation.original_bytes, original.len() as u64);
        assert_eq!(truncation.retained_bytes, SEMANTIC_CONTENT_BYTES as u64);
        assert_eq!(truncation.strategy, ContentTruncationStrategy::HeadTail);
    }

    #[test]
    fn oversized_command_input_uses_head_tail_with_exact_byte_counts() {
        let original = format!("HEAD{}TAIL", "x".repeat(142 * 1024));
        let mut tool = ToolInvocation {
            call_id: "command-one".to_string(),
            name: "shell".to_string(),
            namespace: None,
            category: ToolCategory::Command,
            origin: ToolOrigin {
                kind: ToolOriginKind::Builtin,
                name: None,
            },
            input: ToolInput::CommandToolInput(CommandToolInput {
                kind: CommandToolInputKind::Command,
                command: original.clone(),
                cwd: None,
                shell: None,
                truncation: None,
                actions: None,
            }),
            outcome: None,
            timing: None,
            annotations: None,
            extension: None,
        };
        let mut remaining = SEMANTIC_CONTENT_BYTES;

        truncate_tool(&mut tool, &mut remaining);

        let ToolInput::CommandToolInput(input) = tool.input else { unreachable!() };
        assert_eq!(input.command.len(), SEMANTIC_CONTENT_BYTES);
        assert!(input.command.starts_with("HEAD"));
        assert!(input.command.ends_with("TAIL"));
        let truncation = input.truncation.expect("command truncation metadata");
        assert_eq!(truncation.original_bytes, original.len() as u64);
        assert_eq!(truncation.retained_bytes, SEMANTIC_CONTENT_BYTES as u64);
        assert_eq!(truncation.strategy, ContentTruncationStrategy::HeadTail);
    }

    #[test]
    fn cumulative_page_budget_preserves_items_and_cursor() {
        let fitted = fit_single_turn_conversation_history(Some(100), response(500, 20_000));
        assert_eq!(fitted.items.len(), 500);
        assert!(serialized_history_bytes(&fitted) <= 8 * 1024 * 1024);
        assert_eq!(fitted.page_info.and_then(|page| page.next_cursor), Some("next".to_string()));
    }

    #[test]
    fn utf8_head_tail_does_not_split_a_character() {
        let truncated = head_tail_utf8("好".repeat(100).as_str(), 64);
        assert!(truncated.len() <= 64);
        assert!(truncated.is_char_boundary(truncated.len()));
    }
}
