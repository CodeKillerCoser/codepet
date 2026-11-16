//! Source normalization belongs to the Provider, never to the Host/Pet view.
use super::*;
use codepet_provider_sdk::{common::*, ProtocolMethod};
use std::collections::{BTreeMap, VecDeque};

pub struct HookEvent {
    pub event_id: String,
    pub received_at: u64,
    pub gap: bool,
    pub payload: serde_json::Map<String, Value>,
}
pub trait HookObserver: Send + Sync {
    fn observe(&self, event: &HookEvent) -> Result<(), ProtocolError>;
}
impl<F> HookObserver for F
where
    F: Fn(&HookEvent) -> Result<(), ProtocolError> + Send + Sync,
{
    fn observe(&self, event: &HookEvent) -> Result<(), ProtocolError> {
        self(event)
    }
}
struct Activity {
    key: String,
    notice: Option<Notice>,
    turn: Option<String>,
    retired: VecDeque<String>,
    at: u64,
}
pub(super) struct Publisher {
    source: &'static str,
    sink: Arc<dyn ProviderEventSink>,
    rows: BTreeMap<String, Activity>,
    seen: VecDeque<String>,
}
fn text(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| {
        v.get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned)
    })
}
impl Publisher {
    pub(super) fn new(source: &'static str, sink: Arc<dyn ProviderEventSink>) -> Self {
        Self {
            source,
            sink,
            rows: BTreeMap::new(),
            seen: VecDeque::new(),
        }
    }
    async fn call(&self, method: ProtocolMethod, value: Value) -> Result<Notice, ProtocolError> {
        let mut last = None;
        for attempt in 0..3 {
            match self.sink.request_host(method, value.clone()).await {
                Ok(value) => {
                    return serde_json::from_value::<codepet_provider_sdk::NoticePublishResponse>(
                        value,
                    )
                    .map(|r| r.notice)
                    .map_err(error)
                }
                Err(e) if e.retryable && attempt < 2 => {
                    last = Some(e);
                    tokio::time::sleep(std::time::Duration::from_millis(100 * (attempt + 1))).await;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| error("Notice publication failed")))
    }
    pub(super) async fn apply(&mut self, event: HookEvent) -> Result<(), ProtocolError> {
        if self.seen.contains(&event.event_id) {
            return Ok(());
        }
        let raw = Value::Object(event.payload.clone());
        let name = text(&raw, &["hook_event_name", "type"]).unwrap_or_default();
        let native = if self.source == "opencode" {
            raw.get("properties").unwrap_or(&raw)
        } else {
            &raw
        };
        // Subagent/tool errors are not parent terminal evidence.
        if name.starts_with("Subagent")
            || raw.get("agent_id").is_some()
            || raw["session"]["parentID"].is_string()
        {
            return Ok(());
        }
        let Some(session) = text(native, &["session_id", "sessionID", "thread_id"]).or_else(|| {
            native
                .get("session")
                .or_else(|| native.get("info"))
                .and_then(|s| text(s, &["id"]))
        }) else {
            return Ok(());
        };
        let turn = text(native, &["turn_id", "assistantMessageID"]);
        let mut finish = false;
        let (value, label) = match name.as_str() {
            "PreToolUse"
                if matches!(
                    raw["tool_name"].as_str(),
                    Some("AskUserQuestion" | "request_user_input")
                ) =>
            {
                ("waiting_input", "等待输入")
            }
            "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "PostToolUseFailure"
            | "permission.replied" | "ElicitationResult" | "question.replied"
            | "question.rejected" => ("running", "进行中"),
            "PermissionRequest" | "permission.asked" => ("waiting_permission", "等待授权"),
            "Elicitation" | "question.asked" => ("waiting_input", "等待输入"),
            "session.status" => match native.pointer("/status/type").and_then(Value::as_str) {
                Some("busy" | "retry") => ("running", "进行中"),
                Some("idle") => {
                    finish = true;
                    ("completed", "本轮结束")
                }
                _ => return Ok(()),
            },
            "Stop" | "session.idle" => {
                finish = true;
                ("completed", "本轮结束")
            }
            "Interrupt" => {
                finish = true;
                ("cancelled", "已取消")
            }
            "StopFailure" => {
                finish = true;
                ("failed", "失败")
            }
            "session.error" => {
                finish = true;
                if native.pointer("/error/name").and_then(Value::as_str)
                    == Some("MessageAbortedError")
                {
                    ("cancelled", "已取消")
                } else {
                    ("failed", "失败")
                }
            }
            "SessionEnd" | "session.deleted" => ("unknown", "状态待确认"),
            _ => return Ok(()),
        };
        let at = raw
            .get("codepet_observed_at")
            .and_then(Value::as_u64)
            .unwrap_or(event.received_at)
            .min(event.received_at);
        let starts = name == "UserPromptSubmit"
            || (name == "session.status"
                && value == "running"
                && self
                    .rows
                    .get(&session)
                    .is_none_or(|r| r.notice.as_ref().is_some_and(|n| n.finished)));
        let previous = self.rows.get(&session);
        if previous
            .is_some_and(|r| at < r.at || turn.as_ref().is_some_and(|t| r.retired.contains(t)))
        {
            return Ok(());
        }
        if previous.is_none() && self.rows.len() >= 256 {
            if let Some(oldest) = self
                .rows
                .iter()
                .min_by_key(|(_, r)| r.at)
                .map(|(k, _)| k.clone())
            {
                self.rows.remove(&oldest);
            }
        }
        let previous = self.rows.get(&session);
        if starts || previous.is_none() {
            if previous.is_none()
                && ((finish && self.source == "opencode")
                    || name == "SessionEnd"
                    || name == "session.deleted")
            {
                return Ok(());
            }
            let mut retired = previous.map(|r| r.retired.clone()).unwrap_or_default();
            if let Some(old) = previous.and_then(|r| r.turn.clone()) {
                if Some(&old) != turn.as_ref() {
                    retired.push_back(old);
                }
            }
            while retired.len() > 16 {
                retired.pop_front();
            }
            let key = format!(
                "{}:{}:{}",
                self.source,
                session,
                turn.as_deref().unwrap_or(&event.event_id)
            );
            self.rows.insert(
                session.clone(),
                Activity {
                    key,
                    notice: None,
                    turn: turn.clone(),
                    retired,
                    at,
                },
            );
        }
        let row = self.rows.get(&session).unwrap();
        if !starts && row.notice.as_ref().is_some_and(|n| n.finished) {
            return Ok(());
        }
        if turn.is_some() && row.turn.is_some() && turn != row.turn {
            return Ok(());
        }
        let title = text(&raw, &["codepet_title", "title", "thread_name", "prompt"])
            .or_else(|| raw.get("session").and_then(|s| text(s, &["title"])))
            .or_else(|| text(native, &["title"]))
            .unwrap_or_else(|| {
                row.notice
                    .as_ref()
                    .map(|n| n.content.title.clone())
                    .unwrap_or_else(|| format!("{} 活动", self.source))
            });
        let summary = (if starts {
            None
        } else {
            text(&raw, &["last_assistant_message"])
        })
        .or_else(|| text(&raw, &["prompt", "error"]))
        .or_else(|| {
            raw.get("tool_input")
                .and_then(|s| text(s, &["description", "command", "file_path"]))
        })
        .or_else(|| {
            native
                .pointer("/error/data")
                .and_then(|e| text(e, &["message"]))
        })
        .or_else(|| text(&raw, &["tool_name"]))
        .or_else(|| {
            if starts {
                None
            } else {
                row.notice
                    .as_ref()
                    .and_then(|n| n.content.progress.as_ref())
                    .and_then(|p| p.text.clone())
            }
        });
        let gap = event.gap;
        let status = NoticeStatus {
            value: if gap && !finish { "unknown" } else { value }.into(),
            label: if gap && !finish {
                "状态待确认"
            } else {
                label
            }
            .into(),
        };
        let content = NoticeContent {
            title: title.chars().take(1024).collect(),
            progress: summary.map(|s| NoticeProgress {
                text: Some(s.chars().take(8192).collect()),
                percent: None,
            }),
        };
        let mut actions = vec![];
        if let Some(id) = text(&raw, &["codepet_application_id"]) {
            actions.push(NoticeAction::NoticeActivateAction(NoticeActivateAction {
                kind: NoticeActivateActionKind::Activate,
                id: "activate".into(),
                label: "打开应用".into(),
                enabled: true,
                target: Some(NoticeActivationTarget::NoticeApplicationTarget(
                    NoticeApplicationTarget {
                        kind: NoticeApplicationTargetKind::Application,
                        application_id: id,
                    },
                )),
            }));
        }
        let (key, old) = {
            let row = self.rows.get(&session).unwrap();
            (row.key.clone(), row.notice.clone())
        };
        let notice = if let Some(old) = old {
            old
        } else {
            self.call(ProtocolMethod::NoticeCreate,serde_json::json!({"creationKey":key,"sourceId":self.source,"status":status,"content":content,"actions":actions})).await?
        };
        if notice.finished {
            self.rows.get_mut(&session).unwrap().notice = Some(notice);
            return Ok(());
        }
        let method = if finish {
            ProtocolMethod::NoticeFinish
        } else {
            ProtocolMethod::NoticeUpdate
        };
        let notice=self.call(method,serde_json::json!({"noticeId":notice.notice_id,"operationId":event.event_id,"expectedRevision":notice.revision,"status":status,"content":content,"actions":actions})).await?;
        let row = self.rows.get_mut(&session).unwrap();
        row.notice = Some(notice);
        row.at = at;
        self.seen.push_back(event.event_id);
        while self.seen.len() > 2048 {
            self.seen.pop_front();
        }
        if self.rows.len() > 256 {
            if let Some(oldest) = self
                .rows
                .iter()
                .filter(|(_, r)| r.notice.as_ref().is_some_and(|n| n.finished))
                .min_by_key(|(_, r)| r.at)
                .map(|(k, _)| k.clone())
            {
                self.rows.remove(&oldest);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    #[derive(Default)]
    struct Sink {
        rows: Mutex<BTreeMap<String, Notice>>,
        calls: Mutex<Vec<ProtocolMethod>>,
    }
    impl ProviderEventSink for Sink {
        fn publish(&self, _: codepet_provider_sdk::ProtocolEvent) -> Result<(), ProtocolError> {
            panic!("raw Hook must never become a Provider wire event")
        }
        fn request_host<'a>(
            &'a self,
            method: ProtocolMethod,
            p: Value,
        ) -> codepet_provider_sdk::ProtocolFuture<'a, Value> {
            Box::pin(async move {
                self.calls.lock().unwrap().push(method);
                let mut rows = self.rows.lock().unwrap();
                if method == ProtocolMethod::NoticeCreate {
                    let key = p["creationKey"].as_str().unwrap().to_owned();
                    let next = rows.len() + 1;
                    let notice=rows.entry(key).or_insert_with(||serde_json::from_value(json!({"noticeId":format!("n{next}"),"pluginId":"test","sourceId":p["sourceId"],"status":p["status"],"content":p["content"],"actions":p["actions"],"revision":1,"finished":false,"createdAt":1,"updatedAt":1})).unwrap());
                    return Ok(json!({"notice":notice}));
                }
                let n = rows
                    .values_mut()
                    .find(|n| n.notice_id == p["noticeId"])
                    .unwrap();
                assert_eq!(n.revision, p["expectedRevision"].as_u64().unwrap());
                n.revision += 1;
                n.status = serde_json::from_value(p["status"].clone()).unwrap();
                n.content = serde_json::from_value(p["content"].clone()).unwrap();
                n.actions = serde_json::from_value(p["actions"].clone()).unwrap();
                n.finished |= method == ProtocolMethod::NoticeFinish;
                Ok(json!({"notice":n}))
            })
        }
    }
    fn event(id: &str, name: &str, turn: &str, time: u64, extra: Value) -> HookEvent {
        let mut raw = json!({"session_id":"session","hook_event_name":name,"turn_id":turn});
        for (k, v) in extra.as_object().unwrap() {
            raw[k] = v.clone();
        }
        HookEvent {
            event_id: id.into(),
            received_at: time,
            gap: false,
            payload: raw.as_object().unwrap().clone(),
        }
    }
    #[tokio::test]
    async fn hook_rounds_create_update_and_finish_without_host_source_inference() {
        let sink = Arc::new(Sink::default());
        let mut publisher = Publisher::new("codex", sink.clone());
        publisher
            .apply(event(
                "1",
                "UserPromptSubmit",
                "a",
                10,
                json!({"prompt":"实现功能","last_assistant_message":"旧回复"}),
            ))
            .await
            .unwrap();
        publisher.apply(event("2","PreToolUse","a",11,json!({"tool_name":"Shell","tool_input":{"command":"cargo test"},"codepet_application_id":"com.test.app"}))).await.unwrap();
        let first = sink.rows.lock().unwrap().values().next().unwrap().clone();
        assert_eq!(
            first.content.progress.unwrap().text.as_deref(),
            Some("cargo test")
        );
        assert_eq!(first.actions.len(), 1);
        publisher
            .apply(event(
                "3",
                "PostToolUseFailure",
                "a",
                12,
                json!({"error":"工具失败"}),
            ))
            .await
            .unwrap();
        assert!(!sink.rows.lock().unwrap().values().next().unwrap().finished);
        publisher
            .apply(event(
                "4",
                "Stop",
                "a",
                13,
                json!({"last_assistant_message":"最终结果"}),
            ))
            .await
            .unwrap();
        let closed = sink.rows.lock().unwrap().values().next().unwrap().clone();
        assert!(closed.finished);
        assert_eq!(closed.status.label, "本轮结束");
        let calls = sink.calls.lock().unwrap().len();
        publisher
            .apply(event("4", "Stop", "a", 13, json!({})))
            .await
            .unwrap();
        publisher
            .apply(event("late", "PreToolUse", "a", 12, json!({})))
            .await
            .unwrap();
        assert_eq!(calls, sink.calls.lock().unwrap().len());
        publisher
            .apply(event(
                "5",
                "UserPromptSubmit",
                "b",
                20,
                json!({"prompt":"第二轮"}),
            ))
            .await
            .unwrap();
        publisher
            .apply(event("old-stop", "Stop", "a", 21, json!({})))
            .await
            .unwrap();
        let rows = sink.rows.lock().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows
            .values()
            .any(|n| !n.finished && n.content.title == "第二轮"));
    }
    #[tokio::test]
    async fn subagent_noise_gaps_and_disconnect_cannot_fake_completion() {
        let sink = Arc::new(Sink::default());
        let mut publisher = Publisher::new("claude", sink.clone());
        publisher
            .apply(event(
                "1",
                "UserPromptSubmit",
                "a",
                1,
                json!({"prompt":"任务"}),
            ))
            .await
            .unwrap();
        let count = sink.calls.lock().unwrap().len();
        publisher
            .apply(event("2", "SubagentStop", "a", 2, json!({})))
            .await
            .unwrap();
        assert_eq!(count, sink.calls.lock().unwrap().len());
        let mut gap = event("3", "PostToolUse", "a", 3, json!({}));
        gap.gap = true;
        publisher.apply(gap).await.unwrap();
        let n = sink.rows.lock().unwrap().values().next().unwrap().clone();
        assert_eq!(n.status.value, "unknown");
        assert!(!n.finished);
        publisher
            .apply(event("4", "SessionEnd", "a", 4, json!({})))
            .await
            .unwrap();
        assert!(!sink.rows.lock().unwrap().values().next().unwrap().finished);
    }
    #[tokio::test]
    async fn opencode_status_and_permission_are_standard_notices_without_fake_actions() {
        let sink = Arc::new(Sink::default());
        let mut publisher = Publisher::new("opencode", sink.clone());
        for (id, name, props) in [
            (
                "1",
                "session.status",
                json!({"sessionID":"oc","status":{"type":"busy"}}),
            ),
            ("2", "permission.asked", json!({"sessionID":"oc"})),
            (
                "3",
                "session.error",
                json!({"sessionID":"oc","error":{"name":"MessageAbortedError"}}),
            ),
        ] {
            publisher
                .apply(HookEvent {
                    event_id: id.into(),
                    received_at: id.parse().unwrap(),
                    gap: false,
                    payload: json!({"type":name,"properties":props})
                        .as_object()
                        .unwrap()
                        .clone(),
                })
                .await
                .unwrap();
        }
        let n = sink.rows.lock().unwrap().values().next().unwrap().clone();
        assert!(n.finished);
        assert_eq!(n.status.value, "cancelled");
        assert!(n.actions.is_empty());
    }
}
