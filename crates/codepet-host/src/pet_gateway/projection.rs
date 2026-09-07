use codepet_pet_sdk::{PetSnapshot, PetSource, PetTask, PetTaskStatus as Status};
use codepet_provider_sdk::ProviderNotificationEvent;
use serde_json::Value;
use std::{collections::{BTreeMap, VecDeque, HashSet}, time::{SystemTime, UNIX_EPOCH}};

struct Activity { task: PetTask, turn: Option<String>, retired_turns: VecDeque<String>, time: u64 }
pub(super) struct Projection {
    pub sources: BTreeMap<String, PetSource>,
    tasks: BTreeMap<String, Activity>,
    seen: HashSet<String>, order: VecDeque<String>, revision: u64,
}
fn text(raw: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| raw.get(k).and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(|s| s.chars().take(500).collect()))
}
fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64 }
fn terminal(s: Status) -> bool { matches!(s, Status::Completed | Status::Failed | Status::Interrupted) }
impl Projection {
    pub fn new(sources: BTreeMap<String, PetSource>) -> Self { Self { sources, tasks: BTreeMap::new(), seen: HashSet::new(), order: VecDeque::new(), revision: 0 } }
    pub fn source_status(&mut self, id: &str, status: &str, message: &str) {
        let source = self.sources.get_mut(id).unwrap();
        if source.status == status && source.message == message { return; }
        source.status = status.into(); source.message = message.into(); self.revision += 1;
        if matches!(status, "error" | "connecting" | "disabled") {
            for a in self.tasks.values_mut().filter(|a| a.task.provider_id.as_deref() == Some(id)) {
                if !terminal(a.task.status) { a.task.status = Status::Unknown; }
            }
        }
    }
    pub fn snapshot(&self) -> PetSnapshot {
        let mut tasks: Vec<_> = self.tasks.values().filter(|a| self.sources[a.task.provider_id.as_deref().unwrap()].enabled).map(|a| a.task.clone()).collect();
        tasks.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
        PetSnapshot { revision: self.revision, generated_at: now(), tasks, approvals: vec![], sources: Some(self.sources.values().cloned().collect()) }
    }
    pub fn apply(&mut self, provider: &str, event: ProviderNotificationEvent) {
        self.source_status(provider, "receiving", "正在接收活动");
        let key = format!("{provider}:{}", event.event_id);
        if !self.seen.insert(key.clone()) { return; }
        self.order.push_back(key);
        if self.order.len() > 2048 { if let Some(id) = self.order.pop_front() { self.seen.remove(&id); } }
        let envelope = serde_json::to_value(&event.payload).unwrap();
        let observation = envelope.get("codepet_observation");
        let raw = observation.and_then(|value| value.get("raw")).unwrap_or(&envelope).clone();
        if observation.and_then(|value| value.get("gap")).or_else(|| raw.get("codepet_gap")).and_then(Value::as_bool) == Some(true) { self.source_status(provider, "error", "活动事件存在缺口，部分任务状态待确认"); }
        let name = text(&raw, &["hook_event_name", "type"]).unwrap_or_default();
        if name == "codepet.source.unavailable" {
            self.source_status(provider, "error", &text(&raw, &["message"]).unwrap_or_else(|| "当前来源不支持活动订阅".into()));
            return;
        }
        let native = if provider == "opencode" { raw.get("properties").unwrap_or(&raw) } else { &raw };
        let session = text(native, &["session_id", "sessionID"]).or_else(|| native.get("session").or_else(|| native.get("info")).and_then(|v| text(v, &["id"])));
        let Some(session) = session else { return; };
        // Subagent events and subagent tool calls must not finish or replace the parent's task.
        if name.starts_with("Subagent") || raw.get("agent_id").and_then(Value::as_str).is_some() || raw.pointer("/session/parentID").and_then(Value::as_str).is_some() { return; }
        let id = format!("{provider}:{session}");
        let time = raw.get("codepet_observed_at").and_then(Value::as_u64).unwrap_or(event.received_at).min(event.received_at);
        let turn = text(native, &["turn_id", "assistantMessageID"]);
        let status = match name.as_str() {
            "PreToolUse" if matches!(raw.get("tool_name").and_then(Value::as_str), Some("AskUserQuestion" | "request_user_input")) => Status::WaitingInput,
            "session.status" => match native.pointer("/status/type").and_then(Value::as_str) { Some("busy" | "retry") => Status::Running, Some("idle") => Status::Completed, _ => return },
            "SessionStart" | "session.created" | "session.updated" => Status::Idle,
            "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "PostToolUseFailure" | "permission.replied" => Status::Running,
            "PermissionRequest" | "permission.asked" => Status::WaitingApproval,
            "Elicitation" | "question.asked" => Status::WaitingInput,
            "ElicitationResult" | "question.replied" | "question.rejected" => Status::Running,
            "Stop" | "session.idle" => Status::Completed,
            "session.error" if native.pointer("/error/name").and_then(Value::as_str) == Some("MessageAbortedError") => Status::Interrupted,
            "StopFailure" | "session.error" => Status::Failed,
            "Interrupt" => Status::Interrupted,
            "SessionEnd" | "session.deleted" => Status::Unknown,
            _ => return,
        };
        let starts = name == "UserPromptSubmit" || (name == "session.status" && status == Status::Running);
        if let Some(previous) = self.tasks.get(&id) {
            if time < previous.time { return; }
            if turn.as_ref().is_some_and(|turn| previous.retired_turns.contains(turn)) { return; }
            if status == Status::Idle { return; }
            if !starts && turn.is_some() && previous.turn.is_some() && turn != previous.turn { return; }
            if terminal(previous.task.status) && !starts { return; }
        } else if matches!(status, Status::Idle | Status::Unknown) || (provider == "opencode" && status == Status::Completed) { return; }
        let title = text(&raw, &["title", "thread_name", "prompt"]).or_else(|| raw.get("session").and_then(|s| text(s, &["title"]))).or_else(|| text(native, &["title"]));
        let cwd = text(&raw, &["cwd"]);
        let summary = text(&raw, &["last_assistant_message", "prompt", "error"])
            .or_else(|| native.pointer("/error/data").and_then(|v| text(v, &["message"])))
            .or_else(|| raw.get("tool_input").and_then(|v| text(v, &["description", "command", "file_path"])));
        let a = self.tasks.entry(id.clone()).or_insert_with(|| Activity { task: PetTask { id, title: title.clone().unwrap_or_else(|| "正在处理任务".into()),
            summary: None, status, updated_at: time, provider_id: Some(provider.into()), cwd: cwd.clone(), tool_name: None }, turn: turn.clone(), retired_turns: VecDeque::new(), time });
        if starts {
            if a.turn != turn { if let Some(previous) = a.turn.take() { a.retired_turns.push_back(previous); if a.retired_turns.len() > 16 { a.retired_turns.pop_front(); } } }
            a.turn = turn; a.task.summary = None; a.task.tool_name = None; }
        if let Some(title) = title { a.task.title = title.chars().take(80).collect(); }
        if cwd.is_some() { a.task.cwd = cwd; }
        if summary.is_some() { a.task.summary = summary; }
        a.task.tool_name = text(&raw, &["tool_name"]);
        a.task.status = status; a.task.updated_at = time; a.time = time;
        self.revision += 1;
        if self.tasks.len() > 120 {
            if let Some(oldest) = self.tasks.iter().min_by_key(|(_, a)| a.time).map(|(id, _)| id.clone()) { self.tasks.remove(&oldest); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn state() -> Projection { Projection::new(BTreeMap::from([("codex".into(), PetSource { id:"codex".into(), display_name:"Codex".into(), enabled:true, status:"installed".into(), message:"".into() })])) }
    fn event(id: &str, name: &str, turn: &str, time: u64) -> ProviderNotificationEvent {
        ProviderNotificationEvent { subscription_id:"pet".into(), event_id:id.into(), received_at:time, payload:json!({"hook_event_name":name,"session_id":"session","turn_id":turn,"codepet_observed_at":time}).as_object().unwrap().clone().into_iter().collect() }
    }
    #[test]
    fn observation_envelope_keeps_existing_activity_projection() {
        let mut projection = state();
        let mut input = event("wrapped", "UserPromptSubmit", "turn", 10);
        let raw = serde_json::to_value(&input.payload).unwrap();
        input.payload = json!({"codepet_observation":{"raw": raw, "gap":false}}).as_object().unwrap().clone().into_iter().collect();
        projection.apply("codex", input);
        assert_eq!(projection.snapshot().tasks[0].status, Status::Running);
    }

    #[test]
    fn tool_failure_subagent_and_old_turn_do_not_end_current_task() {
        let mut p=state();
        p.apply("codex",event("1","UserPromptSubmit","a",10));
        p.apply("codex",event("2","PostToolUseFailure","a",11));
        assert_eq!(p.snapshot().tasks[0].status,Status::Running);
        p.apply("codex",event("3","SubagentStop","a",12));
        assert_eq!(p.snapshot().tasks[0].status,Status::Running);
        p.apply("codex",event("4","Stop","a",13));
        p.apply("codex",event("5","UserPromptSubmit","b",14));
        p.apply("codex",event("6","Stop","a",15));
        assert_eq!(p.snapshot().tasks[0].status,Status::Running);
        p.apply("codex",event("stale-start","UserPromptSubmit","a",16));
        assert_eq!(p.snapshot().tasks[0].status,Status::Running);
        p.apply("codex",event("7","Interrupt","b",17));
        assert_eq!(p.snapshot().tasks[0].status,Status::Interrupted);
        p.apply("codex",event("8","PostToolUse","b",18));
        assert_eq!(p.snapshot().tasks[0].status,Status::Interrupted);
    }
    #[test]
    fn duplicates_idle_notifications_and_disconnect_are_not_completion() {
        let mut p=state();
        p.apply("codex",event("1","SessionStart","a",10));
        assert!(p.snapshot().tasks.is_empty());
        p.apply("codex",event("2","UserPromptSubmit","a",11));
        let revision=p.snapshot().revision;
        p.apply("codex",event("2","UserPromptSubmit","a",11));
        p.apply("codex",event("3","Notification","a",12));
        assert_eq!(p.snapshot().revision,revision);
        p.source_status("codex","error","disconnected");
        assert_eq!(p.snapshot().tasks[0].status,Status::Unknown);
    }
    #[test]
    fn opencode_stable_status_permission_and_question_events_keep_sources_distinct() {
        let mut p = state();
        p.sources.insert("opencode".into(), PetSource { id:"opencode".into(), display_name:"OpenCode".into(), enabled:true, status:"installed".into(), message:"".into() });
        let native = |id: &str, name: &str, time: u64| ProviderNotificationEvent { subscription_id:"host".into(), event_id:id.into(), received_at:time,
            payload:json!({"type":name,"properties":{"sessionID":"session","status":{"type":"busy"}},"session":{"title":"OpenCode task"}}).as_object().unwrap().clone().into_iter().collect() };
        p.apply("opencode", native("idle-only", "session.idle", 1));
        p.apply("opencode", native("created", "session.created", 2));
        assert!(p.snapshot().tasks.is_empty());
        p.apply("codex", event("1", "UserPromptSubmit", "turn", 10));
        p.apply("opencode", native("1", "session.status", 11));
        p.apply("opencode", native("2", "message.part.updated", 12));
        assert_eq!(p.snapshot().tasks.len(), 2);
        assert_eq!(p.snapshot().tasks[0].status, Status::Running);
        p.apply("opencode", native("3", "permission.asked", 13));
        assert_eq!(p.snapshot().tasks[0].status, Status::WaitingApproval);
        p.apply("opencode", native("replied", "permission.replied", 14));
        assert_eq!(p.snapshot().tasks[0].status, Status::Running);
        p.apply("opencode", native("question", "question.asked", 15));
        assert_eq!(p.snapshot().tasks[0].status, Status::WaitingInput);
        p.apply("opencode", native("answered", "question.replied", 16));
        assert_eq!(p.snapshot().tasks[0].status, Status::Running);
        p.apply("opencode", native("4", "session.idle", 17));
        assert_eq!(p.snapshot().tasks[0].status, Status::Completed);
        let mut subagent = native("5", "session.status", 18);
        subagent.payload.insert("session".into(), json!({"parentID":"parent"}));
        p.apply("opencode", subagent);
        assert_eq!(p.snapshot().tasks[0].status, Status::Completed);
    }

    #[test]
    fn opencode_failure_and_interrupt_survive_trailing_idle_until_next_run() {
        let source = PetSource { id:"opencode".into(), display_name:"OpenCode".into(), enabled:true, status:"installed".into(), message:"".into() };
        let mut p = Projection::new(BTreeMap::from([("opencode".into(), source)]));
        let mut sequence = 0;
        let mut apply = |p: &mut Projection, name: &str, extra: Value| {
            sequence += 1;
            let mut properties = extra.as_object().unwrap().clone();
            properties.insert("sessionID".into(), json!("session"));
            p.apply("opencode", ProviderNotificationEvent { subscription_id:"host".into(), event_id:sequence.to_string(), received_at:sequence,
                payload:json!({"type":name,"properties":properties}).as_object().unwrap().clone().into_iter().collect() });
        };
        apply(&mut p, "session.status", json!({"status":{"type":"busy"}}));
        apply(&mut p, "session.error", json!({"error":{"name":"UnknownError","data":{"message":"Model not found"}}}));
        apply(&mut p, "session.status", json!({"status":{"type":"idle"}}));
        apply(&mut p, "session.idle", json!({}));
        assert_eq!(p.snapshot().tasks[0].status, Status::Failed);
        assert_eq!(p.snapshot().tasks[0].summary.as_deref(), Some("Model not found"));
        apply(&mut p, "session.status", json!({"status":{"type":"busy"}}));
        assert_eq!(p.snapshot().tasks[0].status, Status::Running);
        apply(&mut p, "session.error", json!({"error":{"name":"MessageAbortedError"}}));
        apply(&mut p, "session.idle", json!({}));
        assert_eq!(p.snapshot().tasks[0].status, Status::Interrupted);
    }

}
