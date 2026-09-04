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
const TRUNCATED_HISTORY_CONTENT_BYTES: usize = 32 * 1024;
const TRUNCATED_HISTORY_NOTICE: &str = "[history content truncated to fit the Provider transport]";

/// Keeps a single-turn history response below the shared Provider/Host frame limit.
///
/// Remote clients reduce `conversation.get.limit` to one after receiving
/// `provider_response_too_large`. At that point the Provider must return bounded
/// history instead of asking the client to shrink the turn page again.
pub fn fit_single_turn_conversation_history(
    requested_limit: Option<u64>,
    mut response: ConversationGetResponse,
) -> ConversationGetResponse {
    if requested_limit != Some(1) || conversation_history_fits(&response) {
        return response;
    }

    for item in &mut response.items {
        truncate_item_content(item);
    }
    if conversation_history_fits(&response) {
        return response;
    }

    if let Some(first) = response.items.first().cloned() {
        let (resource, turn, conversation) = item_identity(first);
        response.items = vec![ConversationItem::UnknownConversationItem(UnknownConversationItem {
            resource,
            turn,
            conversation,
            kind: UnknownConversationItemKind::Unknown,
            status: ConversationItemStatus::Completed,
            title: Some("History omitted".to_string()),
        })];
    } else {
        response.items.clear();
    }
    if !conversation_history_fits(&response) {
        response.conversation.title = truncate_utf8_with_notice(
            &response.conversation.title,
            TRUNCATED_HISTORY_CONTENT_BYTES,
            TRUNCATED_HISTORY_NOTICE,
        );
        response.conversation.preview = response.conversation.preview.as_deref().map(|value| {
            truncate_utf8_with_notice(
                value,
                TRUNCATED_HISTORY_CONTENT_BYTES,
                TRUNCATED_HISTORY_NOTICE,
            )
        });
        response.conversation.workspace_root = None;
        response.conversation.extension = None;
        if let Some(active_turn) = &mut response.conversation.active_turn {
            active_turn.display_summary = active_turn.display_summary.as_deref().map(|value| {
                truncate_utf8_with_notice(
                    value,
                    TRUNCATED_HISTORY_CONTENT_BYTES,
                    TRUNCATED_HISTORY_NOTICE,
                )
            });
            active_turn.extension = None;
        }
    }
    response
}

fn item_identity(item: ConversationItem) -> (RoutedResourceId, RoutedResourceId, RoutedResourceId) {
    match item {
        ConversationItem::MessageConversationItem(value) => (value.resource, value.turn, value.conversation),
        ConversationItem::ReasoningConversationItem(value) => (value.resource, value.turn, value.conversation),
        ConversationItem::CommandConversationItem(value) => (value.resource, value.turn, value.conversation),
        ConversationItem::FileChangeConversationItem(value) => (value.resource, value.turn, value.conversation),
        ConversationItem::ToolConversationItem(value) => (value.resource, value.turn, value.conversation),
        ConversationItem::ApprovalConversationItem(value) => (value.resource, value.turn, value.conversation),
        ConversationItem::UnknownConversationItem(value) => (value.resource, value.turn, value.conversation),
    }
}

fn truncate_item_content(item: &mut ConversationItem) {
    match item {
        ConversationItem::MessageConversationItem(value) => truncate_content_blocks(&mut value.contents),
        ConversationItem::ReasoningConversationItem(value) => truncate_content_blocks(&mut value.contents),
        ConversationItem::FileChangeConversationItem(value) => truncate_content_blocks(&mut value.contents),
        ConversationItem::CommandConversationItem(value) => truncate_tool_content(&mut value.tool),
        ConversationItem::ToolConversationItem(value) => truncate_tool_content(&mut value.tool),
        ConversationItem::ApprovalConversationItem(_) | ConversationItem::UnknownConversationItem(_) => {}
    }
}

fn truncate_tool_content(tool: &mut ToolInvocation) {
    match tool.outcome.as_mut() {
        Some(ToolOutcome::ToolSuccessOutcome(value)) => truncate_content_blocks(&mut value.content),
        Some(ToolOutcome::ToolFailureOutcome(value)) => truncate_content_blocks(&mut value.content),
        None => {}
    }
}

fn truncate_content_blocks(contents: &mut [ContentBlock]) {
    for content in contents {
        match content {
            ContentBlock::TextContentBlock(value) => truncate_text_block(&mut value.text, &mut value.truncation),
            ContentBlock::ReasoningSummaryContentBlock(value) => truncate_text_block(&mut value.text, &mut value.truncation),
            ContentBlock::OutputContentBlock(value) => truncate_text_block(&mut value.text, &mut value.truncation),
            ContentBlock::ActivitySummaryContentBlock(value) => truncate_text_block(&mut value.text, &mut value.truncation),
            ContentBlock::EmbeddedResourceContentBlock(value) => truncate_text_block(&mut value.text, &mut value.truncation),
            ContentBlock::StructuredJsonContentBlock(_)
            | ContentBlock::ImageContentBlock(_)
            | ContentBlock::AudioContentBlock(_)
            | ContentBlock::ResourceLinkContentBlock(_) => {}
        }
    }
}

fn truncate_text_block(text: &mut String, truncation: &mut Option<ContentTruncation>) {
    if text.len() <= TRUNCATED_HISTORY_CONTENT_BYTES {
        return;
    }
    let original_bytes = text.len() as u64;
    *text = truncate_utf8_with_notice(
        text,
        TRUNCATED_HISTORY_CONTENT_BYTES,
        TRUNCATED_HISTORY_NOTICE,
    );
    *truncation = Some(ContentTruncation {
        original_bytes,
        retained_bytes: text.len() as u64,
        strategy: ContentTruncationStrategy::Head,
    });
}

fn conversation_history_fits(response: &ConversationGetResponse) -> bool {
    serde_json::to_vec(response).is_ok_and(|payload| {
        payload.len()
            <= MAX_CONVERSATION_HISTORY_JSON_LINE_BYTES
                .saturating_sub(CONVERSATION_HISTORY_ENVELOPE_RESERVE_BYTES)
    })
}

fn truncate_utf8_with_notice(value: &str, limit: usize, notice: &str) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    let content_limit = limit.saturating_sub(notice.len());
    let mut end = content_limit.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = value[..end].to_string();
    truncated.push_str(notice);
    truncated
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
        assert_eq!(fitted.items.len(), 1);
        assert_eq!(
            fitted
                .page_info
                .as_ref()
                .and_then(|page| page.next_cursor.as_deref()),
            Some("next")
        );
        assert!(matches!(
            &fitted.items[0],
            ConversationItem::UnknownConversationItem(_)
        ));
    }

    #[test]
    fn larger_page_is_left_for_remote_limit_reduction() {
        let original = response(1, MAX_CONVERSATION_HISTORY_JSON_LINE_BYTES);
        let fitted = fit_single_turn_conversation_history(Some(2), original.clone());
        assert_eq!(fitted, original);
    }

    #[test]
    fn utf8_truncation_does_not_split_a_character() {
        let truncated = truncate_utf8_with_notice("好".repeat(100).as_str(), 64, "[cut]");
        assert!(truncated.len() <= 64);
        assert!(truncated.ends_with("[cut]"));
    }
}
