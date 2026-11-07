//! No timer lives in the Provider. Host requests are checked atomically at admission.
use super::activity::Activity;
use crate::generated::{
    InstanceReleaseControlOutcome as Outcome, InstanceReleaseControlRequest,
    InstanceReleaseControlResponse, ProtocolError, ProtocolServer,
};
use std::time::Duration;

pub(super) async fn handle<P: ProtocolServer>(
    provider: &P,
    activity: &Activity,
    request: InstanceReleaseControlRequest,
) -> Result<InstanceReleaseControlResponse, ProtocolError> {
    // Never queue a release behind ongoing work: it may become unsafe while waiting.
    let Ok(_exclusive) = activity.gate.try_write() else {
        return Ok(InstanceReleaseControlResponse {
            outcome: Outcome::Busy,
        });
    };
    let outcome = activity.release_control_outcome(
        &request.route,
        Duration::from_secs(request.idle_timeout_seconds),
    );
    if outcome != Outcome::Released {
        return Ok(InstanceReleaseControlResponse { outcome });
    }
    let id = request.route.provider_instance_id.clone();
    // A failed/no-op adapter must not be retried every scheduler tick.
    activity.touch(&id);
    let result = provider.instance_release_control(request).await;
    activity.touch(&id);
    result
}

#[cfg(test)]
async fn release_idle<P: ProtocolServer>(provider: &P, activity: &Activity, timeout: Duration) {
    let route = crate::generated::ProviderInstanceRoute {
        device_id: "d".into(),
        provider_plugin_id: "p".into(),
        provider_instance_id: "i".into(),
    };
    handle(
        provider,
        activity,
        InstanceReleaseControlRequest {
            route,
            idle_timeout_seconds: timeout.as_secs(),
        },
    )
    .await
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::*;
    use serde_json::json;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };
    use std::time::Instant;

    fn instance(id: &str, status: InstanceStatus) -> ProviderInstance {
        serde_json::from_value(json!({
            "route":{"deviceId":"d","providerPluginId":"p","providerInstanceId":id},
            "pluginId":"p","instanceKind":"fixture","displayName":"Fixture",
            "harness":{"id":"fixture","displayName":"Fixture"},
            "status": status,"capabilities":{"revision":"1","methods":["instance.releaseControl"],"extensions":[]}
        })).unwrap()
    }
    fn status(activity: &Activity, id: &str, value: InstanceStatus) {
        activity.event(&ProtocolEvent::EventInstanceStatusChanged {
            jsonrpc: "2.0".into(),
            params: InstanceStatusChangedEvent {
                instance: instance(id, value),
                previous_status: None,
            },
        });
    }
    fn turn(activity: &Activity, id: &str, value: TurnStatus) {
        activity.event(&ProtocolEvent::EventTurnUpserted { jsonrpc: "2.0".into(), params: TurnUpsertedEvent {
            turn: serde_json::from_value(json!({"resource":{"providerId":"i","nativeResourceId":id},
                "conversation":{"providerId":"i","nativeResourceId":format!("c-{id}")}, "status":value})).unwrap()
        }});
    }
    fn fixture() -> (Arc<Mutex<Instant>>, Arc<Activity>, Provider) {
        let clock = Arc::new(Mutex::new(Instant::now()));
        let read_clock = clock.clone();
        let activity = Arc::new(Activity::with_clock(Arc::new(move || {
            *read_clock.lock().unwrap()
        })));
        let provider = Provider {
            stops: AtomicUsize::new(0),
            activity: activity.clone(),
        };
        status(&activity, "i", InstanceStatus::Ready);
        (clock, activity, provider)
    }
    fn advance(clock: &Mutex<Instant>, seconds: u64) {
        *clock.lock().unwrap() += Duration::from_secs(seconds);
    }
    struct Provider {
        stops: AtomicUsize,
        activity: Arc<Activity>,
    }
    impl ProtocolServer for Provider {
        fn instance_release_control<'a>(
            &'a self,
            request: InstanceReleaseControlRequest,
        ) -> ProtocolFuture<'a, InstanceReleaseControlResponse> {
            Box::pin(async move {
                self.stops.fetch_add(1, Ordering::SeqCst);
                // Fixture stops here so tests can distinguish fresh Ready events from polling.
                self.activity.instance(&instance(
                    &request.route.provider_instance_id,
                    InstanceStatus::Stopped,
                ));
                Ok(InstanceReleaseControlResponse {
                    outcome: Outcome::Released,
                })
            })
        }
    }
    const WINDOW: Duration = Duration::from_secs(300);
    #[tokio::test]
    async fn unsupported_providers_and_wrong_routes_are_noops() {
        let (clock, activity, provider) = fixture();
        advance(&clock, 3600);
        let mut unsupported = instance("i", InstanceStatus::Ready);
        unsupported.capabilities.methods.clear();
        activity.instance(&unsupported);
        let mut request = InstanceReleaseControlRequest {
            route: unsupported.route,
            idle_timeout_seconds: 300,
        };
        assert_eq!(
            handle(&provider, &activity, request.clone())
                .await
                .unwrap()
                .outcome,
            Outcome::Unsupported
        );
        request.route.provider_plugin_id = "wrong-plugin".into();
        assert_eq!(
            handle(&provider, &activity, request).await.unwrap().outcome,
            Outcome::Unavailable
        );
        assert_eq!(provider.stops.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn repeated_requests_and_explicit_stops_do_not_restart_instances() {
        let (clock, activity, provider) = fixture();
        advance(&clock, 300);
        release_idle(&provider, &activity, WINDOW).await;
        release_idle(&provider, &activity, WINDOW).await;
        advance(&clock, 3600);
        release_idle(&provider, &activity, WINDOW).await;
        assert_eq!(provider.stops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn idle_deadline_is_independent_of_healthy_missing_or_removed_presence() {
        for online in [true, false] {
            let (clock, activity, provider) = fixture();
            let heartbeat = super::super::heartbeat::HostHeartbeat::new();
            for sequence in 1..=5 {
                advance(&clock, 60);
                heartbeat
                    .ping(ProviderPingRequest {
                        sequence,
                        host_session_id: "h".into(),
                        clients: ClientConnectionsSnapshot {
                            revision: sequence,
                            connections: if online {
                                vec![serde_json::from_value(
                                    json!({"clientId":"phone","connectionId":"socket"}),
                                )
                                .unwrap()]
                            } else {
                                vec![]
                            },
                        },
                        instances: vec![],
                    })
                    .unwrap();
                release_idle(&provider, &activity, WINDOW).await;
                assert_eq!(
                    provider.stops.load(Ordering::SeqCst),
                    usize::from(sequence == 5)
                );
            }
            advance(&clock, 3600);
            release_idle(&provider, &activity, WINDOW).await;
            assert_eq!(
                provider.stops.load(Ordering::SeqCst),
                1,
                "stopped generation is not repeatedly released"
            );
        }
    }
    #[tokio::test]
    async fn every_turn_must_finish_then_a_full_idle_window_elapses() {
        let (clock, activity, provider) = fixture();
        turn(&activity, "a", TurnStatus::Running);
        turn(&activity, "b", TurnStatus::Queued);
        advance(&clock, 3600);
        release_idle(&provider, &activity, WINDOW).await;
        turn(&activity, "a", TurnStatus::Completed);
        advance(&clock, 3600);
        release_idle(&provider, &activity, WINDOW).await;
        assert_eq!(provider.stops.load(Ordering::SeqCst), 0);
        turn(&activity, "b", TurnStatus::Completed);
        advance(&clock, 299);
        // A replayed terminal or metadata refresh must not postpone the deadline.
        turn(&activity, "a", TurnStatus::Completed);
        status(&activity, "i", InstanceStatus::Ready);
        release_idle(&provider, &activity, WINDOW).await;
        assert_eq!(provider.stops.load(Ordering::SeqCst), 0);
        advance(&clock, 1);
        release_idle(&provider, &activity, WINDOW).await;
        assert_eq!(provider.stops.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn approval_and_waiting_input_are_activity_even_without_a_running_turn() {
        let (clock, activity, provider) = fixture();
        let mut approval: Approval = serde_json::from_value(json!({
            "resource":{"providerId":"i","nativeResourceId":"approval"},
            "conversation":{"providerId":"i","nativeResourceId":"c"},
            "turn":{"providerId":"i","nativeResourceId":"t"},
            "kind":"command","title":"Approve","status":"pending","decisions":[]
        }))
        .unwrap();
        activity.event(&ProtocolEvent::EventApprovalRequested {
            jsonrpc: "2.0".into(),
            params: ApprovalRequestedEvent {
                approval: approval.clone(),
            },
        });
        advance(&clock, 3600);
        release_idle(&provider, &activity, WINDOW).await;
        assert_eq!(provider.stops.load(Ordering::SeqCst), 0);
        approval.status = ApprovalStatus::Approved;
        activity.event(&ProtocolEvent::EventApprovalResolved {
            jsonrpc: "2.0".into(),
            params: ApprovalResolvedEvent { approval },
        });
        let mut conversation: Conversation = serde_json::from_value(json!({
            "resource":{"providerId":"i","nativeResourceId":"input"},"project":null,
            "title":"Input","status":"waiting-user-input"
        }))
        .unwrap();
        activity.event(&ProtocolEvent::EventConversationUpserted {
            jsonrpc: "2.0".into(),
            params: ConversationUpsertedEvent {
                conversation: conversation.clone(),
            },
        });
        advance(&clock, 3600);
        release_idle(&provider, &activity, WINDOW).await;
        assert_eq!(provider.stops.load(Ordering::SeqCst), 0);
        conversation.status = ConversationStatus::Idle;
        activity.event(&ProtocolEvent::EventConversationUpserted {
            jsonrpc: "2.0".into(),
            params: ConversationUpsertedEvent { conversation },
        });
        advance(&clock, 300);
        release_idle(&provider, &activity, WINDOW).await;
        assert_eq!(provider.stops.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn admitted_request_blocks_release_and_business_completion_starts_a_new_window() {
        let (clock, activity, provider) = fixture();
        advance(&clock, 300);
        let request = activity.gate.read().await;
        release_idle(&provider, &activity, WINDOW).await;
        assert_eq!(provider.stops.load(Ordering::SeqCst), 0);
        activity.touch("i");
        drop(request);
        release_idle(&provider, &activity, WINDOW).await;
        assert_eq!(provider.stops.load(Ordering::SeqCst), 0);
        advance(&clock, 300);
        // A read-only request blocks release during its execution without renewing idle.
        let read = activity.gate.read().await;
        release_idle(&provider, &activity, WINDOW).await;
        drop(read);
        release_idle(&provider, &activity, WINDOW).await;
        assert_eq!(provider.stops.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn unknown_send_requires_explicit_cleanup_and_cannot_revive_a_new_generation() {
        let request = ProtocolRequest::from_method_params(ProtocolMethod::TurnStart, "start".into(), json!({
            "conversation":{"deviceId":"d","providerPluginId":"p","providerInstanceId":"i","nativeResourceId":"c-a"},
            "clientRequestId":"message","capabilityRevision":"1","input":{"kind":"text","text":"hello"},
            "selection":{"accessModeId":"workspace-write","model":{"kind":"flat","modelId":"fixture"}}
        })).unwrap();
        let error: JsonRpcResponse = serde_json::from_value(json!({"jsonrpc":"2.0","id":"start", "error":{
            "code":-32000,"message":"timeout","data":{"code":"provider_error","details":{"outcome":"unknown"}}
        }})).unwrap();
        let (clock, activity, provider) = fixture();
        let observation = activity.mutation(&request);
        activity.mutation_response(observation.clone(), &error);
        // An unrelated or replayed terminal cannot prove this unknown send finished.
        turn(&activity, "a", TurnStatus::Completed);
        advance(&clock, 3600);
        release_idle(&provider, &activity, WINDOW).await;
        assert_eq!(provider.stops.load(Ordering::SeqCst), 0);
        status(&activity, "i", InstanceStatus::Stopped);
        status(&activity, "i", InstanceStatus::Ready);
        activity.mutation_response(observation, &error);
        advance(&clock, 300);
        release_idle(&provider, &activity, WINDOW).await;
        assert_eq!(provider.stops.load(Ordering::SeqCst), 1);
    }

    struct BlockingStop {
        base: Provider,
        entered: tokio::sync::Notify,
        finish: tokio::sync::Semaphore,
    }
    impl ProtocolServer for BlockingStop {
        fn instance_release_control<'a>(
            &'a self,
            request: InstanceReleaseControlRequest,
        ) -> ProtocolFuture<'a, InstanceReleaseControlResponse> {
            Box::pin(async move {
                self.entered.notify_one();
                let _permit = self.finish.acquire().await.unwrap();
                self.base.instance_release_control(request).await
            })
        }
    }
    #[tokio::test]
    async fn new_requests_cannot_enter_a_generation_being_released() {
        let (clock, activity, base) = fixture();
        advance(&clock, 300);
        let provider = Arc::new(BlockingStop {
            base,
            entered: tokio::sync::Notify::new(),
            finish: tokio::sync::Semaphore::new(0),
        });
        let p = provider.clone();
        let a = activity.clone();
        let release = tokio::spawn(async move { release_idle(p.as_ref(), &a, WINDOW).await });
        provider.entered.notified().await;
        assert!(activity.gate.try_read().is_err());
        let a = activity.clone();
        let request = tokio::spawn(async move {
            let _admitted = a.gate.read().await;
            a.idle_routes(WINDOW)
        });
        tokio::task::yield_now().await;
        assert!(!request.is_finished());
        provider.finish.add_permits(1);
        release.await.unwrap();
        assert!(
            request.await.unwrap().is_empty(),
            "new request observes the stopped generation"
        );
    }
}
