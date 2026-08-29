use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone, Default)]
pub struct CodexThreadScope {
    inner: Arc<Mutex<CodexThreadScopeState>>,
}

#[derive(Default)]
struct CodexThreadScopeState {
    remote_thread_ids: HashSet<String>,
    remote_operations_in_flight: HashMap<String, usize>,
    remote_creations_in_flight: usize,
    remote_creation_ambiguous: bool,
    remote_creation_epoch: u64,
    subscribers: Vec<Sender<String>>,
    creation_settled_subscribers: Vec<Sender<CodexRemoteCreationSettlement>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexCompanionDisposition {
    Local,
    Remote,
    Quarantine(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexRemoteCreationOutcome {
    Known,
    Ambiguous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodexRemoteCreationSettlement {
    pub epoch: u64,
    pub outcome: CodexRemoteCreationOutcome,
}

pub struct CodexRemoteCreationGuard {
    scope: CodexThreadScope,
    epoch: u64,
    active: bool,
}

pub struct CodexRemoteOperationGuard {
    scope: CodexThreadScope,
    thread_id: String,
    active: bool,
}

impl CodexThreadScope {
    pub fn mark_remote(&self, thread_id: impl Into<String>) {
        let thread_id = thread_id.into();
        if thread_id.is_empty() {
            return;
        }
        let mut state = lock(&self.inner);
        state.remote_operations_in_flight.remove(&thread_id);
        if !state.remote_thread_ids.insert(thread_id.clone()) {
            return;
        }
        state
            .subscribers
            .retain(|subscriber| subscriber.send(thread_id.clone()).is_ok());
    }

    pub fn is_remote(&self, thread_id: &str) -> bool {
        lock(&self.inner).remote_thread_ids.contains(thread_id)
    }

    pub fn begin_remote_creation(&self) -> CodexRemoteCreationGuard {
        let mut state = lock(&self.inner);
        if state.remote_creations_in_flight == 0 {
            state.remote_creation_ambiguous = false;
            state.remote_creation_epoch = state.remote_creation_epoch.saturating_add(1);
        }
        state.remote_creations_in_flight += 1;
        let epoch = state.remote_creation_epoch;
        drop(state);
        CodexRemoteCreationGuard {
            scope: self.clone(),
            epoch,
            active: true,
        }
    }

    pub fn remote_creation_in_flight(&self) -> bool {
        lock(&self.inner).remote_creations_in_flight > 0
    }

    pub fn companion_disposition(
        &self,
        thread_id: &str,
        already_known: bool,
    ) -> CodexCompanionDisposition {
        let state = lock(&self.inner);
        if state.remote_thread_ids.contains(thread_id) {
            CodexCompanionDisposition::Remote
        } else if !already_known && state.remote_creations_in_flight > 0 {
            CodexCompanionDisposition::Quarantine(state.remote_creation_epoch)
        } else {
            CodexCompanionDisposition::Local
        }
    }

    pub fn with_local_thread<T>(
        &self,
        thread_id: &str,
        operation: impl FnOnce() -> T,
    ) -> Option<T> {
        let state = lock(&self.inner);
        if state.remote_thread_ids.contains(thread_id)
            || state.remote_operations_in_flight.contains_key(thread_id)
        {
            return None;
        }
        let result = operation();
        drop(state);
        Some(result)
    }

    pub fn begin_remote_operation(
        &self,
        thread_id: impl Into<String>,
    ) -> CodexRemoteOperationGuard {
        let thread_id = thread_id.into();
        let mut state = lock(&self.inner);
        *state
            .remote_operations_in_flight
            .entry(thread_id.clone())
            .or_insert(0) += 1;
        drop(state);
        CodexRemoteOperationGuard {
            scope: self.clone(),
            thread_id,
            active: true,
        }
    }

    pub fn subscribe_remote_threads(&self) -> Receiver<String> {
        let (sender, receiver) = mpsc::channel();
        let mut state = lock(&self.inner);
        for thread_id in &state.remote_thread_ids {
            let _ = sender.send(thread_id.clone());
        }
        state.subscribers.push(sender);
        receiver
    }

    pub fn subscribe_remote_creation_settled(
        &self,
    ) -> Receiver<CodexRemoteCreationSettlement> {
        let (sender, receiver) = mpsc::channel();
        lock(&self.inner)
            .creation_settled_subscribers
            .push(sender);
        receiver
    }
}

impl Drop for CodexRemoteCreationGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        self.finish(CodexRemoteCreationOutcome::Ambiguous);
    }
}

impl CodexRemoteCreationGuard {
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn settle_known(mut self) {
        self.finish(CodexRemoteCreationOutcome::Known);
    }

    fn finish(&mut self, outcome: CodexRemoteCreationOutcome) {
        if !self.active {
            return;
        }
        let mut state = lock(&self.scope.inner);
        if outcome == CodexRemoteCreationOutcome::Ambiguous {
            state.remote_creation_ambiguous = true;
        }
        state.remote_creations_in_flight = state.remote_creations_in_flight.saturating_sub(1);
        if state.remote_creations_in_flight == 0 {
            let outcome = if state.remote_creation_ambiguous {
                CodexRemoteCreationOutcome::Ambiguous
            } else {
                CodexRemoteCreationOutcome::Known
            };
            let settlement = CodexRemoteCreationSettlement {
                epoch: self.epoch,
                outcome,
            };
            state
                .creation_settled_subscribers
                .retain(|subscriber| subscriber.send(settlement).is_ok());
            state.remote_creation_ambiguous = false;
        }
        self.active = false;
    }
}

impl Drop for CodexRemoteOperationGuard {
    fn drop(&mut self) {
        if self.active {
            self.commit_remote();
        }
    }
}

impl CodexRemoteOperationGuard {
    pub fn commit_remote(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        self.scope.mark_remote(self.thread_id.clone());
    }

    pub fn release_local(&mut self) {
        if !self.active {
            return;
        }
        let mut state = lock(&self.scope.inner);
        if let Some(count) = state.remote_operations_in_flight.get_mut(&self.thread_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                state.remote_operations_in_flight.remove(&self.thread_id);
            }
        }
        self.active = false;
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn remote_thread_marks_are_shared_and_announced_once() {
        let scope = CodexThreadScope::default();
        let clone = scope.clone();
        let receiver = scope.subscribe_remote_threads();

        clone.mark_remote("thread-remote");
        clone.mark_remote("thread-remote");

        assert!(scope.is_remote("thread-remote"));
        assert_eq!(receiver.try_iter().collect::<Vec<_>>(), vec!["thread-remote"]);

        let late_receiver = scope.subscribe_remote_threads();
        assert_eq!(
            late_receiver.try_iter().collect::<Vec<_>>(),
            vec!["thread-remote"]
        );
    }

    #[test]
    fn remote_creation_guard_is_shared_and_released() {
        let scope = CodexThreadScope::default();
        let clone = scope.clone();
        let guard = clone.begin_remote_creation();

        assert!(scope.remote_creation_in_flight());
        let settled = scope.subscribe_remote_creation_settled();
        assert_eq!(
            scope.companion_disposition("thread-new", false),
            CodexCompanionDisposition::Quarantine(guard.epoch())
        );
        assert_eq!(
            scope.companion_disposition("thread-known", true),
            CodexCompanionDisposition::Local
        );
        guard.settle_known();
        assert!(!scope.remote_creation_in_flight());
        assert_eq!(
            settled
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .outcome,
            CodexRemoteCreationOutcome::Known
        );
        assert_eq!(
            scope.companion_disposition("thread-new", false),
            CodexCompanionDisposition::Local
        );
    }

    #[test]
    fn dropped_remote_creation_guard_reports_ambiguous_outcome() {
        let scope = CodexThreadScope::default();
        let settled = scope.subscribe_remote_creation_settled();
        let guard = scope.begin_remote_creation();

        drop(guard);

        assert_eq!(
            settled
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .outcome,
            CodexRemoteCreationOutcome::Ambiguous
        );
    }

    #[test]
    fn local_action_permit_linearizes_against_remote_marking() {
        let scope = CodexThreadScope::default();
        let action_scope = scope.clone();
        let mark_scope = scope.clone();
        let (action_started, action_started_rx) = mpsc::channel();
        let (release_action, release_action_rx) = mpsc::channel();
        let (mark_finished, mark_finished_rx) = mpsc::channel();

        let action = std::thread::spawn(move || {
            action_scope.with_local_thread("thread-race", || {
                action_started.send(()).unwrap();
                release_action_rx.recv().unwrap();
            })
        });
        action_started_rx.recv().unwrap();
        let marker = std::thread::spawn(move || {
            mark_scope.mark_remote("thread-race");
            mark_finished.send(()).unwrap();
        });

        assert!(mark_finished_rx
            .recv_timeout(Duration::from_millis(20))
            .is_err());
        release_action.send(()).unwrap();
        action.join().unwrap();
        mark_finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        marker.join().unwrap();
        assert!(scope.is_remote("thread-race"));
    }

    #[test]
    fn remote_operation_guard_blocks_local_actions_until_evidence_settles() {
        let scope = CodexThreadScope::default();
        let mut rejected = scope.begin_remote_operation("thread-rejected");
        assert!(scope
            .with_local_thread("thread-rejected", || ())
            .is_none());
        rejected.release_local();
        assert!(scope
            .with_local_thread("thread-rejected", || ())
            .is_some());
        assert!(!scope.is_remote("thread-rejected"));

        let ambiguous = scope.begin_remote_operation("thread-ambiguous");
        assert!(scope
            .with_local_thread("thread-ambiguous", || ())
            .is_none());
        drop(ambiguous);
        assert!(scope.is_remote("thread-ambiguous"));
        assert!(scope
            .with_local_thread("thread-ambiguous", || ())
            .is_none());
    }
}
