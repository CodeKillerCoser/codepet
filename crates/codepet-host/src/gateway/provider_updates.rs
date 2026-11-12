//! A fast router plus bounded ordered workers: no business work on the shared inbox.
use std::{collections::BTreeMap, sync::Arc};
use tokio::{sync::mpsc, task::JoinHandle};

struct Worker<T> {
    sender: mpsc::Sender<T>,
    task: JoinHandle<()>,
}
pub(super) struct Workers<T> {
    workers: BTreeMap<String, Worker<T>>,
    capacity: usize,
    handler: Arc<dyn Fn(T) + Send + Sync>,
}
impl<T: Send + 'static> Workers<T> {
    pub(super) fn new(capacity: usize, handler: impl Fn(T) + Send + Sync + 'static) -> Self {
        Self {
            workers: BTreeMap::new(),
            capacity: capacity.max(1),
            handler: Arc::new(handler),
        }
    }
    pub(super) fn dispatch(&mut self, key: &str, value: T) -> Result<(), ()> {
        let worker = self.workers.entry(key.to_owned()).or_insert_with(|| {
            let (sender, mut receiver) = mpsc::channel(self.capacity);
            let handler = self.handler.clone();
            let task = tokio::spawn(async move {
                while let Some(value) = receiver.recv().await {
                    let handler = handler.clone();
                    if tokio::task::spawn_blocking(move || handler(value))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            });
            Worker { sender, task }
        });
        worker.sender.try_send(value).map_err(|_| ())
    }
}
impl<T> Drop for Workers<T> {
    fn drop(&mut self) {
        for worker in self.workers.values() {
            worker.task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Condvar, Mutex};
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn slow_provider_and_its_full_queue_do_not_block_other_provider() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        struct Release(Arc<(Mutex<bool>, Condvar)>);
        impl Drop for Release {
            fn drop(&mut self) {
                *self.0 .0.lock().unwrap() = true;
                self.0 .1.notify_all();
            }
        }
        let _release = Release(gate.clone());
        let (started, mut entering) = mpsc::unbounded_channel();
        let (finished, mut done) = mpsc::unbounded_channel();
        let mut workers = Workers::new(1, move |(key, index): (&'static str, usize)| {
            if key == "a" && index == 0 {
                started.send(()).unwrap();
                let mut open = gate.0.lock().unwrap();
                while !*open {
                    open = gate.1.wait(open).unwrap();
                }
            }
            let _ = finished.send((key, index));
        });
        workers.dispatch("a", ("a", 0)).unwrap();
        entering.recv().await.unwrap();
        workers.dispatch("a", ("a", 1)).unwrap();
        assert!(workers.dispatch("a", ("a", 2)).is_err());
        workers.dispatch("b", ("b", 0)).unwrap();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(2), done.recv())
                .await
                .unwrap(),
            Some(("b", 0))
        );
        drop(_release);
        assert_eq!(done.recv().await, Some(("a", 0)));
        assert_eq!(done.recv().await, Some(("a", 1)));
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::super::*;
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_provider_commits_preserve_replay_and_live_cursor_order() {
        let bus = Arc::new(GatewayEventBus::new(1024));
        let mut subscription = bus.subscribe(Some(&event_cursor(0))).unwrap();
        let mut writers = Vec::new();
        for source in 0..8 {
            let bus = bus.clone();
            writers.push(tokio::task::spawn_blocking(move||{
                for index in 0..100 {
                    let event=serde_json::from_value(serde_json::json!({"jsonrpc":"2.0","method":"turn.outputDelta","params":{"eventCursor":"event-0","payload":{
                        "turn":{"providerId":source.to_string(),"nativeResourceId":"t"},"conversation":{"providerId":source.to_string(),"nativeResourceId":"c"},
                        "itemId":"i","contentId":"text","kind":"text","delta":index.to_string()
                    }}})).unwrap();bus.publish(event).unwrap();
                }
            }));
        }
        for writer in writers {
            writer.await.unwrap();
        }
        for sequence in 1..=800 {
            let event =
                tokio::time::timeout(std::time::Duration::from_secs(1), subscription.next_event())
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(protocol_event_sequence(&event).unwrap(), sequence);
        }
        let replay = bus.replay(Some(&event_cursor(0))).unwrap();
        assert_eq!(replay.len(), 800);
        for (i, event) in replay.iter().enumerate() {
            assert_eq!(protocol_event_sequence(event).unwrap(), i as u64 + 1);
        }
    }
}
