//! `GET /runs/{id}/stream` — SSE, per `docs/api-contract.md`'s "Live
//! stream" section. Bridges `worker::progress::ProgressHub`'s live
//! `ProgressEvent` broadcast (see `runner::progress`) into the documented
//! `progress`/`log`/`finding`/`status` SSE event shapes.
//!
//! This streams the real events the runner emits — for the production
//! `SubprocessRunner`, that's one `log` line before the subprocess starts,
//! then a single `progress` and (if a crash was found) a single `finding`
//! event once the whole `cargo fuzz run` invocation exits, plus `status` at
//! the start and end of the run. It is not a per-iteration progress tick;
//! see `runner::subprocess` — the subprocess is opaque until it exits, so
//! there is nothing more granular to stream today.

use std::convert::Infallible;

use async_stream::stream;
use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use chrono::Utc;
use futures::Stream;
use serde::Serialize;
use tokio::sync::broadcast::error::RecvError;

use super::error::ApiError;
use super::util::parse_uuid;
use super::AppState;
use crate::runner::progress::{LogLevel, ProgressEvent};
use crate::store::RunStatus;

fn is_terminal(status: RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Completed | RunStatus::Cancelled | RunStatus::Failed
    )
}

fn log_level_str(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressPayload {
    iterations: u64,
    elapsed_secs: u64,
    status: RunStatus,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LogPayload {
    line: String,
    level: &'static str,
    ts: chrono::DateTime<Utc>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FindingPayload {
    // Always empty today: the runner emits this event before the finding
    // is ever persisted (persistence happens once, in bulk, in
    // `Worker::process` after the run finishes — see `worker/mod.rs`), so
    // there is no database id yet at the point this event fires. A real
    // fix needs the finding to be persisted (or its id otherwise known)
    // before this event is emitted, which is out of scope for this route —
    // tracked as a known follow-up, not silently faked.
    finding_id: &'static str,
    invariant: String,
    step_index: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusPayload {
    status: RunStatus,
}

fn progress_event(count: u64, elapsed_secs: u64, status: RunStatus) -> Result<Event, axum::Error> {
    Event::default()
        .event("progress")
        .json_data(ProgressPayload {
            iterations: count,
            elapsed_secs,
            status,
        })
}

fn log_event(line: String, level: LogLevel) -> Result<Event, axum::Error> {
    Event::default().event("log").json_data(LogPayload {
        line,
        level: log_level_str(level),
        ts: Utc::now(),
    })
}

fn finding_event(invariant: String, step_index: u32) -> Result<Event, axum::Error> {
    Event::default().event("finding").json_data(FindingPayload {
        finding_id: "",
        invariant,
        step_index,
    })
}

fn status_event(status: RunStatus) -> Result<Event, axum::Error> {
    Event::default()
        .event("status")
        .json_data(StatusPayload { status })
}

pub async fn stream_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let run_id = parse_uuid(&id)?;
    let record =
        state.store.get_run(run_id).await?.ok_or_else(|| {
            ApiError::not_found("run_not_found", format!("no run with id {run_id}"))
        })?;

    let initial_status = record.status;
    let receiver = state.progress.subscribe(run_id);

    let event_stream = stream! {
        let Some(mut receiver) = receiver else {
            // Not currently live: `ProgressHub` only holds a channel for
            // the lifetime of an in-progress run (see its doc comment), so
            // no channel means this run is either still `Pending` (not yet
            // claimed by a worker) or already terminal. Either way there is
            // nothing further to stream — report the last known status
            // once and close, rather than polling the store internally.
            if let Ok(event) = status_event(initial_status) {
                yield Ok(event);
            }
            return;
        };

        // Seeds the `status` field every `progress` payload carries — the
        // underlying `ProgressEvent::Iterations` doesn't carry status
        // itself (see docs/api-contract.md vs. runner::progress::ProgressEvent).
        let mut last_status = initial_status;

        loop {
            match receiver.recv().await {
                Ok(ProgressEvent::Iterations { count, elapsed_secs }) => {
                    if let Ok(event) = progress_event(count, elapsed_secs, last_status) {
                        yield Ok(event);
                    }
                }
                Ok(ProgressEvent::Log { line, level }) => {
                    if let Ok(event) = log_event(line, level) {
                        yield Ok(event);
                    }
                }
                Ok(ProgressEvent::Finding { invariant, step_index }) => {
                    if let Ok(event) = finding_event(invariant, step_index) {
                        yield Ok(event);
                    }
                }
                Ok(ProgressEvent::Status(status)) => {
                    last_status = status;
                    if let Ok(event) = status_event(status) {
                        yield Ok(event);
                    }
                    if is_terminal(status) {
                        break;
                    }
                }
                Err(RecvError::Lagged(_)) => {
                    // The channel is still open, a slow subscriber just
                    // missed some buffered events -- keep going rather than
                    // ending the stream over a gap.
                    continue;
                }
                Err(RecvError::Closed) => {
                    // Normally preceded by the terminal `Status` event
                    // above, which already breaks the loop; this is just a
                    // safe fallback if the channel closes first.
                    break;
                }
            }
        }
    };

    Ok(Sse::new(event_stream).keep_alive(KeepAlive::default()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::api::{router, AppState};
    use crate::store::mock::MockStore;
    use crate::store::NewCampaign;
    use crate::targets::TargetRegistry;
    use crate::worker::progress::ProgressHub;

    fn test_state() -> AppState {
        let targets_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        AppState {
            store: Arc::new(MockStore::default()),
            targets: Arc::new(
                TargetRegistry::load(&targets_dir).expect("real manifest should load"),
            ),
            progress: Arc::new(ProgressHub::default()),
            max_time_budget_secs: 600,
        }
    }

    async fn new_run(state: &AppState) -> Uuid {
        let record = state
            .store
            .create_campaign(NewCampaign {
                target_id: "counter".to_string(),
                name: "test".to_string(),
                invariant_ids: Vec::new(),
                time_budget_secs: 30,
            })
            .await
            .unwrap();
        record.run_id
    }

    async fn body_string(response: axum::response::Response) -> String {
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn missing_run_is_404() {
        let app = router(test_state());
        let response = app
            .oneshot(
                Request::get(format!("/runs/{}/stream", Uuid::new_v4()))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn malformed_id_is_400() {
        let app = router(test_state());
        let response = app
            .oneshot(
                Request::get("/runs/not-a-uuid/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn pending_run_with_no_channel_emits_one_status_event_then_closes() {
        let state = test_state();
        let run_id = new_run(&state).await;
        // No `state.progress.publisher(run_id)` was ever opened for this
        // run -- exactly the "not currently live" case `ProgressHub`
        // documents (not yet claimed by a worker).
        let app = router(state);

        let response = app
            .oneshot(
                Request::get(format!("/runs/{run_id}/stream"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = body_string(response).await;
        assert_eq!(body.matches("event: status").count(), 1, "body: {body}");
        assert!(body.contains("\"status\":\"pending\""), "body: {body}");
    }

    #[tokio::test]
    async fn live_run_streams_events_in_order_and_closes_on_terminal_status() {
        let state = test_state();
        let run_id = new_run(&state).await;
        state.store.start_run(run_id, "test-worker").await.unwrap();

        // Opens the channel before the request, same as `Worker::process`
        // does relative to a real client connecting -- `ProgressHub::subscribe`
        // (called inside `stream_run`, synchronously, as part of building
        // the `Sse` response) only sees this channel if it already exists.
        let progress = state.progress.clone();
        let publisher = progress.publisher(run_id);

        let app = router(state);
        let response = app
            .oneshot(
                Request::get(format!("/runs/{run_id}/stream"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // `stream_run`'s own async body -- including the `subscribe()` call
        // -- has already run to completion by the time `oneshot` returned a
        // response above; only the lazy `stream!` body (the `recv()` loop)
        // is deferred to when the body gets polled below. So the receiver
        // already exists and emitting now, before draining, is safe:
        // `broadcast` delivers anything sent after subscribing regardless
        // of whether `recv()` has been called yet. Emitting before the
        // request (the previous, buggy version of this test) is NOT safe:
        // a `broadcast::Receiver` never sees messages sent before it
        // subscribed, so the handler's `recv().await` would hang forever
        // waiting for events that already came and went.
        publisher.emit(ProgressEvent::Status(RunStatus::Running));
        publisher.emit(ProgressEvent::Log {
            line: "cargo +nightly fuzz run counter_fuzz".to_string(),
            level: LogLevel::Info,
        });
        publisher.emit(ProgressEvent::Iterations {
            count: 42,
            elapsed_secs: 3,
        });
        publisher.emit(ProgressEvent::Finding {
            invariant: "counter-value-matches-model".to_string(),
            step_index: 7,
        });
        publisher.emit(ProgressEvent::Status(RunStatus::Completed));

        let body = body_string(response).await;

        // Two `status` events (running, then the terminal completed) plus
        // one each of progress/log/finding -- and nothing after the
        // terminal status closed the stream.
        assert_eq!(body.matches("event: status").count(), 2, "body: {body}");
        assert_eq!(body.matches("event: progress").count(), 1, "body: {body}");
        assert_eq!(body.matches("event: log").count(), 1, "body: {body}");
        assert_eq!(body.matches("event: finding").count(), 1, "body: {body}");

        assert!(body.contains("\"status\":\"running\""), "body: {body}");
        assert!(body.contains("\"status\":\"completed\""), "body: {body}");
        // The `progress` event's status is seeded from the last-seen
        // `Status`, which by the time `Iterations` arrived was `running`.
        assert!(body.contains("\"iterations\":42"), "body: {body}");
        assert!(
            body.contains("\"elapsedSecs\":3,\"status\":\"running\""),
            "body: {body}"
        );
        assert!(
            body.contains("\"invariant\":\"counter-value-matches-model\""),
            "body: {body}"
        );
        // The known, deliberate placeholder from this module's doc comment.
        assert!(body.contains("\"findingId\":\"\""), "body: {body}");

        // `status_idx` points at the terminal event's `data:` line (its
        // `"status":"completed"` payload), which is already past that
        // event's own `event: status` marker -- so zero further `event:`
        // lines after this point means nothing followed it.
        let status_idx = body.find("\"status\":\"completed\"").unwrap();
        assert_eq!(
            body[status_idx..].matches("event:").count(),
            0,
            "nothing should follow the terminal status event; body: {body}"
        );
    }
}
