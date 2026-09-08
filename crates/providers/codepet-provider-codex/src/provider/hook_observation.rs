use super::*;
use codepet_provider_sdk::{ConversationActiveChangedEvent, ConversationItemUpsertedEvent, ConversationStatus, ProviderNotificationEvent, RoutedResourceId};
use std::collections::VecDeque;

pub(super) const INTERNAL_SUBSCRIPTION: &str = "codepet-codex-activity-observation";

#[derive(Default)]
struct SessionFacts {
    summary: Option<Conversation>,
    status: Option<ConversationStatus>,
    received_at: u64,
    lifecycle_at: u64,
}

/// A consumer projection for active.list and summary reads. It never tracks
/// writer ownership and never decides whether a content update is published.
#[derive(Default)]
pub(super) struct ActivityProjection {
    sessions: HashMap<String, SessionFacts>,
    event_ids: VecDeque<String>,
}

impl ActivityProjection {
    fn project(&mut self, conversation: &mut Conversation) {
        let facts = self.sessions.entry(conversation.resource.native_resource_id.clone()).or_default();
        if let Some(status) = facts.status {
            conversation.status = status;
            if !conversation_atoms::active(status) { conversation.active_turn = None; }
        }
        if facts.received_at != 0 {
            conversation.updated_at = Some(conversation.updated_at.unwrap_or(0).max(facts.received_at));
        }
        facts.summary = Some(conversation.clone());
    }

    pub(super) fn active_rows(&self, route: &ProviderInstanceRoute) -> Vec<Conversation> {
        self.sessions.iter().filter_map(|(id, facts)| {
            let status = facts.status.filter(|status| conversation_atoms::active(*status))?;
            // Only resource/status/version leave active.list. No placeholder
            // summary is ever published as real conversation metadata.
            let mut row = facts.summary.clone().unwrap_or_else(|| Conversation {
                resource: RoutedResourceId { provider_id: route.provider_instance_id.clone(), native_resource_id: id.clone() },
                project: None, title: id.clone(), preview: None, status,
                permission_level: None, model: None, reasoning_effort: None, selection: None,
                workspace_root: None, created_at: None, updated_at: None, active_turn: None, read_state: None,
            });
            row.status = status;
            row.updated_at = Some(facts.received_at);
            Some(row)
        }).collect()
    }

    fn apply(&mut self, route: &ProviderInstanceRoute, event: &ProviderNotificationEvent) -> Result<Vec<ProtocolEvent>, ProtocolError> {
        let Some(raw) = event.payload.get("codepet_observation").and_then(|value| value.get("raw")) else { return Ok(vec![]); };
        let Some(id) = raw.get("session_id").or_else(|| raw.get("thread_id")).and_then(Value::as_str).filter(|id| !id.is_empty()) else { return Ok(vec![]); };
        let Some(signal) = hook_signal(raw) else { return Ok(vec![]); };
        if self.event_ids.contains(&event.event_id) { return Ok(vec![]); }
        let facts = self.sessions.entry(id.into()).or_default();
        let lifecycle = matches!(&signal, HookSignal::Start | HookSignal::Stop);
        if lifecycle && event.received_at < facts.lifecycle_at { return Ok(vec![]); }
        let previous_status = facts.status;
        let latest_update = event.received_at >= facts.received_at;
        self.event_ids.push_back(event.event_id.clone());
        if self.event_ids.len() > 2048 { self.event_ids.pop_front(); }
        facts.received_at = facts.received_at.max(event.received_at);
        if lifecycle { facts.lifecycle_at = event.received_at; }
        let status = match signal {
            HookSignal::Start => Some(ConversationStatus::Running),
            HookSignal::Stop => Some(ConversationStatus::Idle),
            // Tool/approval hooks can refine an active session, never start it
            // or resurrect it after stop. They still invalidate its content.
            HookSignal::Update(status) => facts.status.filter(|status| latest_update && conversation_atoms::active(*status)).map(|_| status),
        };
        if let Some(status) = status { facts.status = Some(status); }
        let resource = RoutedResourceId { provider_id: route.provider_instance_id.clone(), native_resource_id: id.into() };
        let mut events = vec![ProtocolEvent::EventConversationItemUpserted {
            jsonrpc: "2.0".into(), params: ConversationItemUpsertedEvent {
                conversation: Some(resource.clone()), item: None, update_id: Some(event.event_id.clone()),
            },
        }];
        if let Some(status) = status.filter(|_| lifecycle || status != previous_status) {
            let version = if conversation_atoms::shared_state_configured() {
                codepet_provider_data::conversation_state::SharedConversationStateStore::from_env()?.activity_versions(&[resource])?.remove(0)
            } else { format!("hook:{}", event.event_id) };
            events.push(ProtocolEvent::EventConversationActiveChanged {
                jsonrpc: "2.0".into(), params: ConversationActiveChangedEvent {
                    conversation: ProviderResourceId { device_id: route.device_id.clone(), provider_plugin_id: route.provider_plugin_id.clone(), provider_instance_id: route.provider_instance_id.clone(), native_resource_id: id.into() },
                    status, active: conversation_atoms::active(status), activity_version: version, revision: format!("hook:{}", event.event_id),
                },
            });
        }
        if let Some(summary) = &mut facts.summary {
            if let Some(status) = facts.status { summary.status = status; }
            if !conversation_atoms::active(summary.status) { summary.active_turn = None; }
            summary.updated_at = Some(summary.updated_at.unwrap_or(0).max(event.received_at));
            events.push(ProtocolEvent::EventConversationUpserted {
                jsonrpc: "2.0".into(), params: ConversationUpsertedEvent { conversation: summary.clone() },
            });
        }
        Ok(events)
    }
}

/// Native data continues to provide metadata and items, while Hook lifecycle
/// facts remain authoritative for the summary's activity state.
pub(super) struct SummaryEvents {
    pub projection: Arc<Mutex<ActivityProjection>>,
    pub sink: Arc<dyn ProviderEventSink>,
}
impl ProviderEventSink for SummaryEvents {
    fn publish(&self, event: ProtocolEvent) -> Result<(), ProtocolError> { self.publish_batch(vec![event]) }
    fn publish_batch(&self, mut events: Vec<ProtocolEvent>) -> Result<(), ProtocolError> {
        for event in &mut events {
            if let ProtocolEvent::EventConversationUpserted { params, .. } = event {
                lock(&self.projection).project(&mut params.conversation);
            }
        }
        self.sink.publish_batch(events)
    }
}

pub(super) struct HookEvents {
    pub state: std::sync::Weak<Mutex<ProviderState>>,
    pub sink: Arc<dyn ProviderEventSink>,
}
impl ProviderEventSink for HookEvents {
    fn publish(&self, event: ProtocolEvent) -> Result<(), ProtocolError> {
        if let ProtocolEvent::EventNotification { params, .. } = &event {
            if let Some(state) = self.state.upgrade() {
                let runtimes = lock(&state).instances.values().cloned().collect::<Vec<_>>();
                for runtime in runtimes {
                    if let Err(error) = runtime.observe_hook(params) {
                        eprintln!("Codex Hook projection failed: {}", error.message);
                    }
                }
            }
            if params.subscription_id == INTERNAL_SUBSCRIPTION { return Ok(()); }
        }
        self.sink.publish(event)
    }
}

impl CodexInstanceRuntime {
    pub(super) fn project_conversation(&self, conversation: &mut Conversation) {
        lock(&self.hook_activity).project(conversation);
    }

    fn observe_hook(&self, event: &ProviderNotificationEvent) -> Result<(), ProtocolError> {
        // Server stop/start and writer acquisition are not session activity facts.
        let state = lock(&self.mutable);
        if state.destroyed { return Ok(()); }
        let events = lock(&self.hook_activity).apply(&self.route, event)?;
        self.events.publish_batch(events)
    }
}

enum HookSignal { Start, Stop, Update(ConversationStatus) }
fn hook_signal(raw: &Value) -> Option<HookSignal> {
    Some(match raw.get("hook_event_name")?.as_str()? {
        "SessionStart" | "UserPromptSubmit" => HookSignal::Start,
        "SessionEnd" | "Stop" | "Interrupt" => HookSignal::Stop,
        "PreToolUse" if matches!(raw.get("tool_name").and_then(Value::as_str), Some("AskUserQuestion" | "request_user_input")) => HookSignal::Update(ConversationStatus::WaitingUserInput),
        "PreToolUse" | "PostToolUse" => HookSignal::Update(ConversationStatus::Running),
        "PermissionRequest" => HookSignal::Update(ConversationStatus::WaitingApproval),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Events(Mutex<Vec<ProtocolEvent>>);
    impl ProviderEventSink for Events {
        fn publish(&self, event: ProtocolEvent) -> Result<(), ProtocolError> {
            lock(&self.0).push(event); Ok(())
        }
    }

    fn runtime() -> (CodexInstanceRuntime, Arc<Events>) {
        let sink = Arc::new(Events::default());
        let request = InstanceCreateRequest {
            route: ProviderInstanceRoute { device_id: "device".into(), provider_plugin_id: CODEX_PLUGIN_ID.into(), provider_instance_id: "codex".into() },
            instance_kind: CODEX_INSTANCE_KIND.into(), display_name: "Codex".into(), settings: Default::default(),
        };
        let runtime = CodexInstanceRuntime::new(request, CodexInstanceSettings {
            app_server_executable: std::env::current_exe().unwrap(), app_server_args: vec![], data_directory: None,
        }, sink.clone(), Arc::new(NoopExecutionLifecycleHook));
        lock(&runtime.mutable).status = InstanceStatus::Ready;
        (runtime, sink)
    }

    fn notification(id: &str, name: &str, time: u64) -> ProviderNotificationEvent {
        ProviderNotificationEvent {
            subscription_id: INTERNAL_SUBSCRIPTION.into(), event_id: id.into(), received_at: time,
            payload: serde_json::from_value(json!({"codepet_observation":{"raw":{
                "hook_event_name":name, "session_id":"thread"
            }}})).unwrap(),
        }
    }

    #[test]
    fn updates_are_published_before_resume_and_do_not_change_active_membership() {
        let (runtime, sink) = runtime();
        runtime.observe_hook(&notification("tool", "PostToolUse", 1)).unwrap();
        assert!(lock(&runtime.hook_activity).active_rows(&runtime.route).is_empty());
        assert!(lock(&sink.0).iter().any(|event| matches!(event, ProtocolEvent::EventConversationItemUpserted { params, .. } if params.item.is_none())));
        // Merely holding an execution slot must not suppress Hook updates.
        lock(&runtime.mutable).executions.insert("thread".into(), Arc::new(ExecutionSlot::new()));
        runtime.observe_hook(&notification("owned-tool", "PostToolUse", 2)).unwrap();
        assert_eq!(lock(&sink.0).iter().filter(|event| matches!(event, ProtocolEvent::EventConversationItemUpserted { .. })).count(), 2);
    }

    #[test]
    fn only_hook_start_stop_changes_activity_even_across_server_lifecycles() {
        let (runtime, sink) = runtime();
        let start = notification("start", "SessionStart", 10);
        runtime.observe_hook(&start).unwrap();
        runtime.observe_hook(&start).unwrap();
        assert_eq!(lock(&runtime.hook_activity).active_rows(&runtime.route).len(), 1);
        assert!(lock(&sink.0).iter().any(|event| matches!(event, ProtocolEvent::EventConversationActiveChanged { params, .. } if params.active)));
        lock(&runtime.mutable).status = InstanceStatus::Stopped;
        runtime.observe_hook(&notification("end", "SessionEnd", 20)).unwrap();
        runtime.observe_hook(&notification("late-tool", "PostToolUse", 21)).unwrap();
        runtime.observe_hook(&notification("old-start", "SessionStart", 9)).unwrap();
        assert!(lock(&runtime.hook_activity).active_rows(&runtime.route).is_empty());
        assert!(lock(&sink.0).iter().any(|event| matches!(event, ProtocolEvent::EventConversationActiveChanged { params, .. } if !params.active)));
        assert_eq!(lock(&sink.0).iter().filter(|event| matches!(event, ProtocolEvent::EventConversationItemUpserted { .. })).count(), 3);
        runtime.observe_hook(&notification("next", "UserPromptSubmit", 30)).unwrap();
        assert_eq!(lock(&runtime.hook_activity).active_rows(&runtime.route).len(), 1);
    }

    #[test]
    fn content_updates_do_not_mask_a_late_lifecycle_fact_or_repeat_start() {
        let (runtime, sink) = runtime();
        runtime.observe_hook(&notification("start", "SessionStart", 10)).unwrap();
        runtime.observe_hook(&notification("tool", "PostToolUse", 30)).unwrap();
        assert_eq!(lock(&sink.0).iter().filter(|event| matches!(event, ProtocolEvent::EventConversationActiveChanged { .. })).count(), 1);
        runtime.observe_hook(&notification("stop", "Stop", 20)).unwrap();
        assert!(lock(&runtime.hook_activity).active_rows(&runtime.route).is_empty());
        runtime.observe_hook(&notification("next-tool", "PostToolUse", 50)).unwrap();
        runtime.observe_hook(&notification("next-start", "UserPromptSubmit", 40)).unwrap();
        assert_eq!(lock(&runtime.hook_activity).active_rows(&runtime.route).len(), 1);
    }

    #[test]
    fn native_summary_status_cannot_start_or_stop_hook_activity() {
        let (runtime, sink) = runtime();
        let mut summary: Conversation = serde_json::from_value(json!({
            "resource":{"providerId":"codex","nativeResourceId":"thread"},
            "project":null,"title":"Real title","status":"running"
        })).unwrap();
        runtime.project_conversation(&mut summary);
        assert!(lock(&runtime.hook_activity).active_rows(&runtime.route).is_empty());
        runtime.observe_hook(&notification("start", "SessionStart", 10)).unwrap();
        summary.status = ConversationStatus::Idle;
        runtime.events.publish(ProtocolEvent::EventConversationUpserted { jsonrpc: "2.0".into(), params: ConversationUpsertedEvent { conversation: summary.clone() } }).unwrap();
        assert!(matches!(lock(&sink.0).last().unwrap(), ProtocolEvent::EventConversationUpserted { params, .. } if params.conversation.status == ConversationStatus::Running));
        runtime.observe_hook(&notification("end", "Stop", 20)).unwrap();
        summary.status = ConversationStatus::Running;
        runtime.project_conversation(&mut summary);
        assert_eq!(summary.status, ConversationStatus::Idle);
        assert_eq!(summary.title, "Real title");
        runtime.observe_hook(&notification("subagent", "SubagentStop", 30)).unwrap();
        assert!(lock(&runtime.hook_activity).active_rows(&runtime.route).is_empty());
    }
}
