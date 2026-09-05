//! Optional content policy for Provider mappers, never applied by the transport Runtime.
use crate::{ContentBlock, ConversationItem, JsonObject, ToolInput, ToolOutcome};
use serde_json::{json, Value};

pub const DEFAULT_TOOL_TEXT_BYTES: usize = 256 * 1024;

/// Truncate oversized textual payloads in `kind: tool` items at item generation.
/// Preserve item identity, structured JSON shape and unrelated `_meta` entries.
/// Command, file-change, message and reasoning items are intentionally untouched.
pub fn truncate_tool_item_text(item: &mut ConversationItem, max_bytes: usize) {
    let ConversationItem::ToolConversationItem(item) = item else { return; };
    let mut changes = Vec::new();
    match &mut item.tool.input {
        ToolInput::CommandToolInput(input) => {
            truncate_text(&mut input.command, "/tool/input/command", max_bytes, &mut changes);
        }
        ToolInput::StructuredToolInput(input) => {
            truncate_object(&mut input.value, "/tool/input/value", max_bytes, &mut changes);
        }
        ToolInput::OpaqueToolInput(input) => {
            truncate_text(&mut input.value, "/tool/input/value", max_bytes, &mut changes);
        }
    }
    if let Some(outcome) = &mut item.tool.outcome {
        let content = match outcome {
            ToolOutcome::ToolSuccessOutcome(outcome) => &mut outcome.content,
            ToolOutcome::ToolFailureOutcome(outcome) => &mut outcome.content,
        };
        for (index, block) in content.iter_mut().enumerate() {
            let path = format!("/tool/outcome/content/{index}");
            match block {
                ContentBlock::TextContentBlock(block) => truncate_text(&mut block.text, &format!("{path}/text"), max_bytes, &mut changes),
                ContentBlock::OutputContentBlock(block) => truncate_text(&mut block.text, &format!("{path}/text"), max_bytes, &mut changes),
                ContentBlock::ReasoningSummaryContentBlock(block) => truncate_text(&mut block.text, &format!("{path}/text"), max_bytes, &mut changes),
                ContentBlock::ActivitySummaryContentBlock(block) => truncate_text(&mut block.text, &format!("{path}/text"), max_bytes, &mut changes),
                ContentBlock::StructuredJsonContentBlock(block) => truncate_object(&mut block.value, &format!("{path}/value"), max_bytes, &mut changes),
                _ => {} // Resource identities, URIs and media data are not text previews.
            }
        }
    }
    if !changes.is_empty() {
        let meta = item.meta.get_or_insert_with(JsonObject::new);
        let entries = meta.entry("truncations".to_string()).or_insert_with(|| json!([]));
        if let Some(entries) = entries.as_array_mut() { entries.extend(changes); }
        else { *entries = Value::Array(changes); }
    }
}

fn truncate_object(object: &mut JsonObject, path: &str, limit: usize, changes: &mut Vec<Value>) {
    for (key, value) in object {
        truncate_value(value, &format!("{path}/{}", pointer_token(key)), limit, changes);
    }
}

fn truncate_value(value: &mut Value, path: &str, limit: usize, changes: &mut Vec<Value>) {
    match value {
        Value::String(text) => truncate_text(text, path, limit, changes),
        Value::Array(values) => {
            for (index, value) in values.iter_mut().enumerate() {
                truncate_value(value, &format!("{path}/{index}"), limit, changes);
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                truncate_value(value, &format!("{path}/{}", pointer_token(key)), limit, changes);
            }
        }
        _ => {}
    }
}

fn pointer_token(key: &str) -> String { key.replace('~', "~0").replace('/', "~1") }

fn truncate_text(text: &mut String, path: &str, limit: usize, changes: &mut Vec<Value>) {
    const MARKER: &str = "\n…\n";
    if text.len() <= limit { return; }
    let original_bytes = text.len();
    let marker = if limit >= MARKER.len() { MARKER } else { "" };
    let available = limit - marker.len();
    let mut head = available / 2;
    while !text.is_char_boundary(head) { head -= 1; }
    let mut tail = text.len() - (available - available / 2);
    while !text.is_char_boundary(tail) { tail += 1; }
    *text = format!("{}{}{}", &text[..head], marker, &text[tail..]);
    changes.push(json!({
        "path": path, "originalBytes": original_bytes,
        "retainedBytes": text.len(), "strategy": "head-tail",
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_item() -> ConversationItem {
        serde_json::from_value(json!({
            "kind": "tool", "status": "completed",
            "resource": { "providerId": "fixture", "nativeResourceId": "tool" },
            "turn": { "providerId": "fixture", "nativeResourceId": "turn" },
            "conversation": { "providerId": "fixture", "nativeResourceId": "thread" },
            "tool": {
                "callId": "tool", "name": "read", "category": "read", "origin": { "kind": "builtin" },
                "input": { "kind": "structured", "value": { "a/b~c": ["界".repeat(200), 17] } },
                "outcome": { "kind": "success", "content": [
                    { "contentId": "result", "kind": "output", "text": "start".to_string() + &"x".repeat(300) + "end" }
                ] }
            }, "_meta": { "vendor": { "trace": "preserved" } }
        })).unwrap()
    }

    #[test]
    fn tool_text_policy_preserves_structure_identity_utf8_and_records_paths_on_item() {
        let mut item = tool_item();
        truncate_tool_item_text(&mut item, 64);
        let value = serde_json::to_value(&item).unwrap();
        let input = value.pointer("/tool/input/value/a~1b~0c/0").unwrap().as_str().unwrap();
        assert!(input.len() <= 64);
        assert_eq!(value.pointer("/tool/input/value/a~1b~0c/1"), Some(&json!(17)));
        assert_eq!(value["resource"]["nativeResourceId"], "tool");
        assert_eq!(value["_meta"]["vendor"]["trace"], "preserved");
        assert_eq!(value["_meta"]["truncations"][0]["path"], "/tool/input/value/a~1b~0c/0");
        assert_eq!(value["_meta"]["truncations"][0]["originalBytes"], 600);
        assert_eq!(value["_meta"]["truncations"][0]["retainedBytes"], input.len());
        assert_eq!(value["_meta"]["truncations"][1]["path"], "/tool/outcome/content/0/text");
        let output = value["tool"]["outcome"]["content"][0]["text"].as_str().unwrap();
        assert!(output.starts_with("start") && output.ends_with("end") && output.len() <= 64);
        assert!(value["tool"]["input"].get("truncation").is_none());
        truncate_tool_item_text(&mut item, 64);
        assert_eq!(serde_json::to_value(item).unwrap(), value);
    }

    #[test]
    fn complete_tool_and_non_tool_items_have_no_new_metadata_or_text_changes() {
        let mut value = serde_json::to_value(tool_item()).unwrap();
        value.as_object_mut().unwrap().remove("_meta");
        for kind in ["tool", "command"] {
            value["kind"] = json!(kind);
            let mut item: ConversationItem = serde_json::from_value(value.clone()).unwrap();
            truncate_tool_item_text(&mut item, if kind == "tool" { 1024 } else { 64 });
            assert_eq!(serde_json::to_value(item).unwrap(), value);
        }
    }
}
