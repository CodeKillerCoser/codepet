//! Host-owned scheduling, independent of phone presence and Provider heartbeats.
use super::*;
use codepet_provider_sdk::{
    InstanceReleaseControlOutcome as Outcome, InstanceReleaseControlRequest, ProviderCapability,
};
use std::sync::Weak;
use tokio::time::Instant;

struct Scheduled {
    due: Instant,
    in_flight: bool,
}

#[derive(Default)]
struct Schedule {
    instances: BTreeMap<String, Scheduled>,
    unsupported: BTreeSet<String>,
}

impl Schedule {
    fn due(
        &mut self,
        ready: Vec<ProviderInstanceRoute>,
        now: Instant,
        window: Duration,
    ) -> Vec<ProviderInstanceRoute> {
        self.instances.retain(|id, state| {
            state.in_flight || ready.iter().any(|route| &route.provider_instance_id == id)
        });
        ready
            .into_iter()
            .filter(|route| {
                if self.unsupported.contains(&route.provider_instance_id) {
                    return false;
                }
                let state = self
                    .instances
                    .entry(route.provider_instance_id.clone())
                    .or_insert(Scheduled {
                        due: now + window,
                        in_flight: false,
                    });
                if state.in_flight || now < state.due {
                    return false;
                }
                state.in_flight = true;
                true
            })
            .collect()
    }

    fn complete(&mut self, id: &str, outcome: Option<Outcome>, now: Instant, window: Duration) {
        if outcome == Some(Outcome::Unsupported) {
            self.unsupported.insert(id.to_string());
        }
        if let Some(state) = self.instances.get_mut(id) {
            state.in_flight = false;
            // Busy/not-idle checks are bounded; failures wait a full window.
            let delay = match outcome {
                Some(Outcome::Busy | Outcome::NotIdle) => window.min(Duration::from_secs(5)),
                _ => window,
            };
            state.due = now + delay;
        }
    }
}

impl PluginManager {
    pub(super) fn spawn_release_control(
        &self,
        plugin_id: String,
        generation: u64,
        process: Weak<PluginProcess>,
    ) {
        let owner = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let mut schedule = Schedule::default();
            let mut requests = tokio::task::JoinSet::<(
                String,
                Result<Outcome, codepet_provider_sdk::ProtocolError>,
                Duration,
            )>::new();
            let mut ticker = tokio::time::interval(Duration::from_secs(1));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                // At most one in-flight request per instance; plugins have independent workers.
                tokio::select! {
                    _ = ticker.tick() => {},
                    result = requests.join_next(), if !requests.is_empty() => {
                        if let Some(Ok((id, result, window))) = result {
                            match &result {
                                Ok(Outcome::Released) => log("INFO", &plugin_id, &id, generation, "released; clean runtime start accepted"),
                                Err(error) => log("WARN", &plugin_id, &id, generation, &format!("release failed code={} message={}", error.code, error.message)),
                                _ => {}, // Normal busy/idle polls must not flood logs.
                            }
                            schedule.complete(&id, result.ok(), Instant::now(), window);
                        }
                    }
                }
                let (Some(inner), Some(process)) = (owner.upgrade(), process.upgrade()) else {
                    break;
                };
                if inner.shutting_down.load(Ordering::SeqCst) {
                    break;
                }
                let routes = {
                    let plugins = inner.plugins.read().await;
                    let Some(entry) = plugins.get(&plugin_id) else {
                        break;
                    };
                    if entry.generation != generation
                        || entry.state != PluginRuntimeState::Ready
                        || !entry
                            .process
                            .as_ref()
                            .is_some_and(|p| Arc::ptr_eq(p, &process))
                    {
                        break;
                    }
                    entry
                        .instances
                        .values()
                        .filter(|i| i.record.enabled)
                        .filter_map(|i| i.instance.as_ref())
                        .filter(|i| {
                            i.status == InstanceStatus::Ready
                                && i.capabilities
                                    .methods
                                    .contains(&ProviderCapability::InstanceReleaseControl)
                        })
                        .map(|i| i.route.clone())
                        .collect()
                };
                let window = inner.config.release_control_idle_timeout;
                for route in schedule.due(routes, Instant::now(), window) {
                    let process = process.clone();
                    requests.spawn(async move {
                        let id = route.provider_instance_id.clone();
                        let result = process
                            .client()
                            .instance_release_control(InstanceReleaseControlRequest {
                                route,
                                idle_timeout_seconds: window.as_secs(),
                            })
                            .await
                            .map(|response| response.outcome);
                        (id, result, window)
                    });
                }
            }
            // Dropping the JoinSet cancels outstanding RPC streams. Provider shutdown owns cleanup.
        });
    }
}

#[track_caller]
fn log(level: &str, plugin: &str, instance: &str, generation: u64, message: &str) {
    eprintln!(
        "{}",
        crate::log_origin::format_text_log(
            &crate::log_origin::LogOrigin::capture(),
            level,
            "provider.releaseControl",
            &format!("plugin={plugin} instance={instance} generation={generation} {message}")
        )
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    fn route(id: &str) -> ProviderInstanceRoute {
        ProviderInstanceRoute {
            device_id: "device".into(),
            provider_plugin_id: "plugin".into(),
            provider_instance_id: id.into(),
        }
    }
    const WINDOW: Duration = Duration::from_secs(300);

    #[test]
    fn scheduling_is_single_flight_per_instance_and_does_not_block_other_instances() {
        let mut schedule = Schedule::default();
        let now = Instant::now();
        let routes = vec![route("a"), route("b")];
        assert!(schedule.due(routes.clone(), now, WINDOW).is_empty());
        assert_eq!(schedule.due(routes.clone(), now + WINDOW, WINDOW).len(), 2);
        assert!(schedule
            .due(routes.clone(), now + WINDOW * 2, WINDOW)
            .is_empty());
        schedule.complete("b", Some(Outcome::Busy), now + WINDOW, WINDOW);
        assert!(schedule
            .due(
                routes.clone(),
                now + WINDOW + Duration::from_secs(4),
                WINDOW
            )
            .is_empty());
        assert_eq!(
            schedule.due(routes, now + WINDOW + Duration::from_secs(5), WINDOW),
            vec![route("b")]
        );
    }

    #[test]
    fn stopped_or_removed_instances_are_never_started_and_ready_gets_a_fresh_window() {
        let mut schedule = Schedule::default();
        let now = Instant::now();
        schedule.due(vec![route("a")], now, WINDOW);
        assert!(schedule.due(vec![], now + WINDOW, WINDOW).is_empty());
        assert!(schedule
            .due(vec![route("a")], now + WINDOW, WINDOW)
            .is_empty());
        assert_eq!(
            schedule.due(vec![route("a")], now + WINDOW * 2, WINDOW),
            vec![route("a")]
        );
    }

    #[test]
    fn unsupported_is_suppressed_and_failed_requests_have_bounded_retries() {
        let mut schedule = Schedule::default();
        let now = Instant::now();
        let routes = vec![route("a"), route("b")];
        schedule.due(routes.clone(), now, WINDOW);
        schedule.due(routes.clone(), now + WINDOW, WINDOW);
        schedule.complete("a", Some(Outcome::Unsupported), now + WINDOW, WINDOW);
        schedule.complete("b", None, now + WINDOW, WINDOW);
        assert!(schedule
            .due(
                routes.clone(),
                now + WINDOW + Duration::from_secs(5),
                WINDOW
            )
            .is_empty());
        assert_eq!(
            schedule.due(routes, now + WINDOW * 2, WINDOW),
            vec![route("b")]
        );
    }
}
