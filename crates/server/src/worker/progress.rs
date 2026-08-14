//! The in-process pub/sub hub connecting the worker (publisher, via
//! `ProgressPublisher`) to the SSE handler (subscriber — build order phase
//! 6). Each running job gets a broadcast channel keyed by run id.

use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::broadcast;
use uuid::Uuid;

use crate::runner::progress::{ProgressEvent, ProgressPublisher};

const CHANNEL_CAPACITY: usize = 256;

#[derive(Default)]
pub struct ProgressHub {
    channels: Mutex<HashMap<Uuid, broadcast::Sender<ProgressEvent>>>,
}

impl ProgressHub {
    /// Opens (or reopens) the channel for `run_id` and returns a publisher
    /// for it. Called once per job by `Worker::process`, before invoking
    /// the `Runner`.
    pub fn publisher(&self, run_id: Uuid) -> ProgressPublisher {
        let mut channels = self.channels.lock().unwrap();
        let sender = channels
            .entry(run_id)
            .or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0)
            .clone();
        ProgressPublisher::new(sender)
    }

    /// Subscribes to a run's live progress, if it's currently being worked
    /// on. Returns `None` once the run has finished (or hasn't started) —
    /// the SSE handler (phase 6) falls back to polling `GET /runs/{id}` in
    /// that case, per `docs/api-contract.md`.
    #[allow(dead_code)] // consumed by the SSE handler, build order phase 6
    pub fn subscribe(&self, run_id: Uuid) -> Option<broadcast::Receiver<ProgressEvent>> {
        self.channels
            .lock()
            .unwrap()
            .get(&run_id)
            .map(|s| s.subscribe())
    }

    /// Drops the channel once a job is done — without this, `channels`
    /// would grow by one entry per run for the life of the process.
    pub fn close(&self, run_id: Uuid) {
        self.channels.lock().unwrap().remove(&run_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::progress::LogLevel;
    use crate::store::RunStatus;

    #[test]
    fn subscribe_returns_none_before_any_publisher_exists() {
        let hub = ProgressHub::default();
        assert!(hub.subscribe(Uuid::new_v4()).is_none());
    }

    #[test]
    fn close_drops_the_channel() {
        let hub = ProgressHub::default();
        let run_id = Uuid::new_v4();
        let _publisher = hub.publisher(run_id);
        assert!(hub.subscribe(run_id).is_some());

        hub.close(run_id);
        assert!(hub.subscribe(run_id).is_none());
    }

    #[tokio::test]
    async fn subscriber_receives_everything_the_publisher_emits() {
        let hub = ProgressHub::default();
        let run_id = Uuid::new_v4();

        let publisher = hub.publisher(run_id);
        let mut receiver = hub
            .subscribe(run_id)
            .expect("channel should exist once a publisher was made");

        publisher.emit(ProgressEvent::Status(RunStatus::Running));
        publisher.emit(ProgressEvent::Iterations {
            count: 42,
            elapsed_secs: 3,
        });
        publisher.emit(ProgressEvent::Log {
            line: "hello".to_string(),
            level: LogLevel::Warn,
        });
        publisher.emit(ProgressEvent::Finding {
            invariant: "no-negative-balance".to_string(),
            step_index: 7,
        });

        assert!(matches!(
            receiver.recv().await.unwrap(),
            ProgressEvent::Status(RunStatus::Running)
        ));
        assert!(matches!(
            receiver.recv().await.unwrap(),
            ProgressEvent::Iterations {
                count: 42,
                elapsed_secs: 3
            }
        ));
        assert!(matches!(
            receiver.recv().await.unwrap(),
            ProgressEvent::Log {
                level: LogLevel::Warn,
                ..
            }
        ));
        assert!(matches!(
            receiver.recv().await.unwrap(),
            ProgressEvent::Finding { step_index: 7, .. }
        ));
    }

    #[test]
    fn emit_with_no_subscribers_does_not_panic() {
        let hub = ProgressHub::default();
        let publisher = hub.publisher(Uuid::new_v4());
        publisher.emit(ProgressEvent::Status(RunStatus::Completed));
    }
}
