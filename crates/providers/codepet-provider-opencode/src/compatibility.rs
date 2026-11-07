//! OpenCode 2 wire shapes normalized to the adapter's internal legacy model.
//! This module never changes the public Code Pet Provider protocol.
use crate::protocol::{
    OpenCodeEvent, OpenCodeMessagePage, OpenCodePrompt, OpenCodePromptAdmission,
    OpenCodeServerError,
};
use serde::Deserialize;
use serde_json::Value;

fn invalid(message: impl Into<String>) -> OpenCodeServerError {
    OpenCodeServerError::Protocol(message.into())
}

pub(crate) fn decode<T: serde::de::DeserializeOwned>(
    value: Value,
) -> Result<T, OpenCodeServerError> {
    serde_json::from_value(value)
        .map_err(|error| invalid(format!("invalid OpenCode 2 response: {error}")))
}

pub(crate) fn admission(value: Value) -> Result<OpenCodePromptAdmission, OpenCodeServerError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Inbox {
        id: String,
        #[serde(rename = "sessionID")]
        session_id: String,
        time: Time,
        payload: OpenCodePrompt,
        delivery: String,
    }
    #[derive(Deserialize)]
    struct Time {
        created: u64,
    }
    let inbox: Inbox = decode(value)?;
    Ok(OpenCodePromptAdmission {
        id: inbox.id,
        session_id: inbox.session_id,
        prompt: inbox.payload,
        delivery: inbox.delivery,
        time_created: inbox.time.created,
    })
}

pub(crate) fn messages(mut value: Value) -> Result<OpenCodeMessagePage, OpenCodeServerError> {
    if let Some(messages) = value.get_mut("data").and_then(Value::as_array_mut) {
        for message in messages {
            if message["type"] != "assistant" {
                continue;
            }
            if let Some(contents) = message.get_mut("content").and_then(Value::as_array_mut) {
                for (ordinal, content) in contents.iter_mut().enumerate() {
                    if let Some(content) = content.as_object_mut() {
                        // 2.x text/reasoning have no ID. SSE's ordinal is the
                        // position in this same content array (including tools).
                        if content.get("type").and_then(Value::as_str) == Some("tool") {
                            let id = content
                                .get("id")
                                .and_then(Value::as_str)
                                .ok_or_else(|| invalid("OpenCode 2 tool content has no id"))?
                                .to_string();
                            content.insert("callID".into(), id.into());
                        }
                        content.insert("id".into(), Value::String(format!("ordinal:{ordinal}")));
                    }
                }
            }
        }
    }
    decode(value)
}

pub(crate) fn event(mut event: OpenCodeEvent) -> Result<OpenCodeEvent, OpenCodeServerError> {
    let legacy = match event.kind.as_str() {
        "session.step.started" => "session.next.step.started",
        "session.step.ended" => "session.next.step.ended",
        "session.step.failed" => "session.next.step.failed",
        "session.text.delta" => "session.next.text.delta",
        "session.reasoning.delta" => "session.next.reasoning.delta",
        "permission.asked" => "permission.v2.asked",
        "permission.replied" => "permission.v2.replied",
        _ => return Ok(event),
    };
    let data = event
        .data
        .as_object_mut()
        .ok_or_else(|| invalid("OpenCode 2 event data is not an object"))?;
    if event.kind.starts_with("session.") {
        let timestamp = if event.kind == "session.step.started" {
            data.get("started").and_then(Value::as_u64)
        } else {
            event.created
        };
        let timestamp = timestamp
            .ok_or_else(|| invalid(format!("OpenCode 2 {} has no timestamp", event.kind)))?;
        data.insert("timestamp".into(), timestamp.into());
    }
    if matches!(
        event.kind.as_str(),
        "session.text.delta" | "session.reasoning.delta"
    ) {
        let ordinal = data
            .get("ordinal")
            .and_then(Value::as_u64)
            .ok_or_else(|| invalid("OpenCode 2 delta has no ordinal"))?;
        let key = if event.kind == "session.text.delta" {
            "textID"
        } else {
            "reasoningID"
        };
        data.insert(key.into(), format!("ordinal:{ordinal}").into());
    }
    if event.kind == "permission.asked" {
        if let Some(source) = data.get_mut("source").and_then(Value::as_object_mut) {
            let id = source
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("OpenCode 2 permission source has no id"))?
                .to_string();
            source.insert("callID".into(), id.into());
        }
    }
    event.kind = legacy.into();
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn wire(kind: &str, data: Value) -> OpenCodeEvent {
        decode(json!({"id":"evt", "created": 200, "type": kind, "data":data})).unwrap()
    }

    #[test]
    fn modern_admission_uses_upstream_identity_delivery_and_time() {
        let result = admission(
            json!({"id":"msg", "sessionID":"ses", "time":{"created":123},
            "payload":{"text":"hello"}, "delivery":"steer"}),
        )
        .unwrap();
        assert_eq!(
            (
                result.id.as_str(),
                result.session_id.as_str(),
                result.delivery.as_str(),
                result.time_created
            ),
            ("msg", "ses", "steer", 123)
        );
        assert_eq!(result.prompt.text, "hello");
        assert!(admission(
            json!({"id":"msg","sessionID":"ses","payload":{"text":"hi"},"delivery":"queue"})
        )
        .is_err());
    }

    #[test]
    fn modern_stream_and_history_share_ordinals_across_tools_and_repeated_text() {
        let page = messages(
            json!({"data":[{"id":"msg","type":"assistant","time":{"created":1},"content":[
            {"type":"text","text":"same"}, {"type":"tool","id":"call","name":"shell","state":{}},
            {"type":"reasoning","text":"thinking"}, {"type":"text","text":"same"}
        ]}],"cursor":{"previous":null,"next":null}}),
        )
        .unwrap();
        let contents = page.data[0].content.as_ref().unwrap();
        assert_eq!(contents[1].call_id.as_deref(), Some("call"));
        for (ordinal, kind, key) in [
            (0, "session.text.delta", "textID"),
            (2, "session.reasoning.delta", "reasoningID"),
            (3, "session.text.delta", "textID"),
        ] {
            let event = event(wire(kind,json!({"sessionID":"ses","assistantMessageID":"msg","ordinal":ordinal,"delta":"same"}))).unwrap();
            assert_eq!(event.data[key], contents[ordinal].id);
            assert_eq!(event.data["timestamp"], 200);
        }
        assert_ne!(contents[0].id, contents[3].id);
    }

    #[test]
    fn modern_step_uses_actual_started_time_and_rejects_missing_times_or_ordinals() {
        let started = event(wire("session.step.started", json!({"started":100}))).unwrap();
        assert_eq!(started.kind, "session.next.step.started");
        assert_eq!(started.data["timestamp"], 100);
        assert!(event(wire("session.step.started", json!({}))).is_err());
        assert!(event(wire("session.text.delta", json!({"delta":"x"}))).is_err());
        let mut ended = wire("session.step.ended", json!({}));
        ended.created = None;
        assert!(event(ended).is_err());
    }

    #[test]
    fn modern_permissions_preserve_native_source_and_legacy_events_are_unchanged() {
        let result = event(wire(
            "permission.asked",
            json!({"source":{"type":"tool","messageID":"msg","id":"call"}}),
        ))
        .unwrap();
        assert_eq!(result.kind, "permission.v2.asked");
        assert_eq!(result.data["source"]["callID"], "call");
        let legacy = wire(
            "session.next.text.delta",
            json!({"timestamp":5,"textID":"text"}),
        );
        assert_eq!(event(legacy.clone()).unwrap(), legacy);
        assert_eq!(
            event(wire("future.event", json!({}))).unwrap().kind,
            "future.event"
        );
    }
}
