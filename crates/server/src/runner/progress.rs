//! Progress events a `Runner` reports while executing a job, and the
//! publisher it reports them through. Mirrors the SSE payloads
//! `docs/api-contract.md` documents (`ProgressEvent`/`LogEvent`/
//! `FindingEvent`/`StatusEvent`) — `worker::progress::ProgressHub` is what
//! turns these into an actual `GET /runs/{id}/stream` (build order phase
//! 6); for now, publishing with nobody subscribed is a harmless no-op.

use tokio::sync::broadcast;

use crate::store::RunStatus;

// Nothing reads these fields/variants outside `worker::progress`'s own
// tests yet — the real reader is the SSE handler (build order phase 6),
// which will `match` on every one of them to build `docs/api-contract.md`'s
// `progress`/`log`/`finding`/`status` SSE payloads.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    Iterations { count: u64, elapsed_secs: u64 },
    Log { line: String, level: LogLevel },
    Finding { invariant: String, step_index: u32 },
    Status(RunStatus),
}

/// A `Runner`'s handle for reporting progress on one run. Cheap to clone
/// (wraps a `broadcast::Sender`).
#[derive(Clone)]
pub struct ProgressPublisher {
    sender: broadcast::Sender<ProgressEvent>,
}

impl ProgressPublisher {
    pub(crate) fn new(sender: broadcast::Sender<ProgressEvent>) -> Self {
        Self { sender }
    }

    /// Never fails from the caller's point of view: an `Err` here only
    /// means no one is subscribed right now, which is the common case
    /// whenever no SSE client is connected to this run.
    pub fn emit(&self, event: ProgressEvent) {
        let _ = self.sender.send(event);
    }
}
