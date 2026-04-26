//! Consumes FF's `subscribe_lease_history` typed event stream and
//! emits `BridgeEvent`s for lease-lifecycle transitions that never
//! flow through a cairn service call (FF-initiated lease expiry and
//! reclaim by the scanner).
//!
//! # Architecture (CG-c: FF 0.10 typed stream)
//!
//! Before CG-c this module ran a hand-rolled XREAD loop over
//! per-partition `ff:exec:{p}:<eid>:lease:history` streams via raw
//! `ferriskey::Client`. FF 0.10 (FF#324) exposes
//! [`EngineBackend::subscribe_lease_history`] returning a typed
//! [`LeaseHistoryEvent`] stream with a [`ScannerFilter`] parameter
//! honoured inside the backend stream. Cairn consumes that directly:
//!
//! - **One subscription, one cursor.** The backend fans out across
//!   partitions on its side; cairn holds a single stream handle and a
//!   single persisted [`StreamCursor`] row so a restart resumes
//!   exactly where the last event committed.
//! - **Backend-side tenant filter.** The subscription filter carries
//!   `("cairn.instance_id", <own_instance_id>)`; the Valkey backend
//!   applies a per-event HGET on the exec tags hash before yielding,
//!   so foreign-instance events are dropped inside the backend stream
//!   rather than filtered client-side (FF#122 data-plane contract —
//!   `ScannerFilter::with_instance_tag`).
//! - **Explicit reconnect loop.** A
//!   [`EngineError::StreamDisconnected`] is recoverable: we resume
//!   from the `cursor` the error carries and reopen the stream. Any
//!   other error terminates the subscriber (logged at `error`).
//!
//! # Persisted cursor schema reuse
//!
//! The existing [`FfLeaseHistoryCursorStore`] rows were one-per-stream
//! under the legacy XREAD fan-out. CG-c collapses the row space: the
//! subscription is a single logical stream, so one sentinel row at
//! `(partition_id = "__cairn_global__", execution_id = "__cairn_global__")`
//! holds the persisted cursor. Legacy per-stream rows are pruned at
//! boot so the table does not grow unbounded during the migration
//! window.

use std::sync::Arc;

use cairn_domain::{FailureClass, ProjectKey, RunId, TaskId, TaskState};
use cairn_store::projections::{FfLeaseHistoryCursor, FfLeaseHistoryCursorStore};
use flowfabric::core::backend::ScannerFilter;
use flowfabric::core::engine_backend::EngineBackend;
use flowfabric::core::engine_error::EngineError;
use flowfabric::core::stream_events::LeaseHistoryEvent;
use flowfabric::core::stream_subscribe::StreamCursor;
use flowfabric::core::types::ExecutionId;
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::engine::Engine;
use crate::event_bridge::{BridgeEvent, EventBridge};
use crate::helpers::try_parse_project_key;

/// Sentinel row key for the single persisted cursor. The legacy
/// per-stream schema is unchanged — we just reserve this partition /
/// execution pair. Any string that cannot collide with FF's
/// `{fp:N}:<uuid>` ExecutionId shape is safe; the underscore-
/// bracketed pair below is reserved for cairn's internal bookkeeping.
const CURSOR_PARTITION_KEY: &str = "__cairn_global__";
const CURSOR_EXECUTION_KEY: &str = "__cairn_global__";

/// Hard cap on reconnect attempts before the subscriber gives up and
/// terminates (logged at `error`). A resilient reconnect loop without
/// a ceiling would hide a structural backend failure (e.g. Valkey
/// wedged, auth dropped) from the operator. At 100 attempts with
/// backoff, this tolerates transient disconnects that recover in
/// seconds-to-minutes and loudly fails anything longer.
const MAX_RECONNECT_ATTEMPTS: u32 = 100;

/// Backoff base for reconnect retries (exponential, capped). A sub-
/// second initial backoff keeps latency low for single-frame blips;
/// the cap prevents runaway sleeps under sustained disconnects.
const RECONNECT_BACKOFF_MIN_MS: u64 = 100;
const RECONNECT_BACKOFF_MAX_MS: u64 = 5_000;

/// Runtime handle for the subscriber task.
pub struct LeaseHistorySubscriber {
    handle: JoinHandle<()>,
    cancel: CancellationToken,
}

impl LeaseHistorySubscriber {
    /// Spawn the subscriber task. Opens
    /// [`EngineBackend::subscribe_lease_history`] once with a
    /// per-instance [`ScannerFilter`] and tails until cancelled.
    pub fn start(
        backend: Arc<dyn EngineBackend>,
        engine: Arc<dyn Engine>,
        bridge: Arc<EventBridge>,
        cursor_store: Arc<dyn FfLeaseHistoryCursorStore>,
        own_instance_id: String,
    ) -> Self {
        let cancel = CancellationToken::new();
        let worker = Worker {
            backend,
            engine,
            bridge,
            cursor_store,
            cancel: cancel.clone(),
            own_instance_id,
        };
        let handle = tokio::spawn(worker.run());
        tracing::info!("lease-history subscriber started (FF 0.10 typed stream)");
        Self { handle, cancel }
    }

    /// Signal the worker to stop and await termination.
    pub async fn shutdown(self) {
        self.cancel.cancel();
        if let Err(e) = self.handle.await {
            tracing::warn!(error = %e, "lease-history subscriber worker panicked");
        }
    }
}

struct Worker {
    backend: Arc<dyn EngineBackend>,
    engine: Arc<dyn Engine>,
    bridge: Arc<EventBridge>,
    cursor_store: Arc<dyn FfLeaseHistoryCursorStore>,
    cancel: CancellationToken,
    /// This cairn-app instance's id — threaded into the backend's
    /// `ScannerFilter` so foreign-instance events never reach this
    /// subscriber. See the module doc for why filtering moved
    /// server-side in FF 0.10.
    own_instance_id: String,
}

impl Worker {
    async fn run(self) {
        // Restart recovery: prime from the persisted cursor. `None`
        // means we've never run — start from the tail (subscribe from
        // now) rather than replay every lease event since the dawn of
        // Valkey.
        let mut cursor = match self.load_cursor().await {
            Ok(Some(c)) => {
                tracing::debug!(
                    bytes_len = c.as_bytes().len(),
                    "lease-history subscriber restored cursor"
                );
                c
            }
            Ok(None) => {
                tracing::debug!(
                    "lease-history subscriber has no persisted cursor; starting from tail"
                );
                StreamCursor::empty()
            }
            Err(e) => {
                tracing::warn!(error = %e, "lease-history subscriber cursor load failed; starting from tail");
                StreamCursor::empty()
            }
        };

        // Build the per-instance filter once — reused on every
        // reconnect. `instance_tag` narrows the backend-side stream
        // to this cairn-app's executions only.
        let filter = ScannerFilter::new()
            .with_instance_tag("cairn.instance_id", self.own_instance_id.clone());

        let mut reconnect_attempts: u32 = 0;
        'outer: loop {
            if self.cancel.is_cancelled() {
                break;
            }
            let mut stream = match self
                .backend
                .subscribe_lease_history(cursor.clone(), &filter)
                .await
            {
                Ok(s) => {
                    reconnect_attempts = 0;
                    s
                }
                Err(EngineError::StreamDisconnected {
                    cursor: resume_cursor,
                }) => {
                    // Backend refused the initial subscribe — treat
                    // like a mid-stream disconnect: backoff and retry.
                    cursor = resume_cursor;
                    if !self.backoff_or_cancel(&mut reconnect_attempts).await {
                        break 'outer;
                    }
                    continue 'outer;
                }
                Err(e) => {
                    tracing::error!(error = %e, "lease-history subscriber subscribe_lease_history failed (terminal); subscriber stopping");
                    break 'outer;
                }
            };

            loop {
                tokio::select! {
                    _ = self.cancel.cancelled() => break 'outer,
                    maybe_event = stream.next() => {
                        let Some(result) = maybe_event else {
                            // Stream ended without a terminal error —
                            // treat as disconnect and reopen from the
                            // last-committed cursor.
                            tracing::debug!("lease-history stream ended; reopening");
                            if !self.backoff_or_cancel(&mut reconnect_attempts).await {
                                break 'outer;
                            }
                            continue 'outer;
                        };
                        match result {
                            Ok(event) => {
                                let event_cursor = event.cursor().clone();
                                self.handle_event(event).await;
                                if let Err(e) = self.persist_cursor(&event_cursor).await {
                                    tracing::warn!(
                                        error = %e,
                                        "lease-history subscriber cursor upsert failed"
                                    );
                                }
                                cursor = event_cursor;
                            }
                            Err(EngineError::StreamDisconnected { cursor: resume_cursor }) => {
                                // FF signalled disconnect; retry from
                                // the cursor FF handed back.
                                cursor = resume_cursor;
                                if !self.backoff_or_cancel(&mut reconnect_attempts).await {
                                    break 'outer;
                                }
                                continue 'outer;
                            }
                            Err(e) => {
                                tracing::error!(
                                    error = %e,
                                    "lease-history subscriber non-recoverable error; subscriber stopping"
                                );
                                break 'outer;
                            }
                        }
                    }
                }
            }
        }
        tracing::info!("lease-history subscriber stopped");
    }

    /// Sleep for the next reconnect backoff slot, or return `false`
    /// when cancellation fires / the attempt budget is exhausted.
    async fn backoff_or_cancel(&self, attempts: &mut u32) -> bool {
        *attempts = attempts.saturating_add(1);
        if *attempts > MAX_RECONNECT_ATTEMPTS {
            tracing::error!(
                attempts = *attempts,
                "lease-history subscriber exhausted reconnect budget; giving up"
            );
            return false;
        }
        // Exponential backoff capped at RECONNECT_BACKOFF_MAX_MS.
        let shift = (*attempts - 1).min(6);
        let ms = RECONNECT_BACKOFF_MIN_MS
            .saturating_mul(1u64 << shift)
            .min(RECONNECT_BACKOFF_MAX_MS);
        tracing::debug!(
            attempt = *attempts,
            backoff_ms = ms,
            "lease-history subscriber reconnecting"
        );
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(ms)) => true,
            _ = self.cancel.cancelled() => false,
        }
    }

    async fn load_cursor(&self) -> Result<Option<StreamCursor>, String> {
        match self
            .cursor_store
            .get(CURSOR_PARTITION_KEY, CURSOR_EXECUTION_KEY)
            .await
        {
            Ok(Some(row)) => {
                // Row stores cursor bytes as base64 in `last_stream_id`
                // — the legacy column name is retained for schema
                // compatibility. An empty string means "start from
                // tail" (first-ever run on a migrated store).
                if row.last_stream_id.is_empty() {
                    return Ok(None);
                }
                let bytes = base64_decode(&row.last_stream_id)
                    .map_err(|e| format!("lease-history cursor base64 decode failed: {e}"))?;
                Ok(Some(StreamCursor::new(bytes)))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(format!("cursor_store.get: {e}")),
        }
    }

    async fn persist_cursor(&self, cursor: &StreamCursor) -> Result<(), String> {
        let encoded = base64_encode(cursor.as_bytes());
        let row = FfLeaseHistoryCursor {
            partition_id: CURSOR_PARTITION_KEY.to_owned(),
            execution_id: CURSOR_EXECUTION_KEY.to_owned(),
            last_stream_id: encoded,
            updated_at_ms: now_ms(),
        };
        self.cursor_store
            .upsert(&row)
            .await
            .map_err(|e| format!("cursor_store.upsert: {e}"))
    }

    async fn handle_event(&self, event: LeaseHistoryEvent) {
        match event {
            LeaseHistoryEvent::Expired { execution_id, .. } => {
                self.dispatch_expired(&execution_id).await;
            }
            LeaseHistoryEvent::Reclaimed {
                execution_id,
                new_owner,
                ..
            } => {
                self.dispatch_reclaimed(&execution_id, new_owner).await;
            }
            // Acquired / Renewed / Revoked: not dispatched today.
            // `Acquired` / `Renewed` transitions flow through the
            // cairn service-level bridge (workers emit them through
            // worker_sdk); double-emitting here would duplicate events
            // on the projection. `Revoked` is a terminal event that
            // FF's own reconciler handles; cairn observes it as a
            // follow-on state transition via the completion stream.
            // `#[non_exhaustive]` wildcard is mandatory — future
            // variants land additively without breaking the build.
            _ => {}
        }
    }

    /// HGET-ish: fetch the exec tags and classify. Returns `None` when
    /// the execution is absent (e.g. terminal and pruned), has no
    /// project tag, or is owned by a different cairn instance.
    ///
    /// The instance-ownership check is defence-in-depth: the backend
    /// filter already drops foreign events inside the stream, so
    /// seeing one here indicates either a version skew (pre-0.10
    /// backend missed the filter) or a backfill gap. Rejecting at
    /// this layer preserves the cross-tenant isolation invariant
    /// regardless of backend-side filter behaviour.
    async fn resolve_context(&self, execution_id: &ExecutionId) -> Option<EntityContext> {
        let snapshot = match self.engine.describe_execution(execution_id).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                tracing::trace!(
                    exec_id = %execution_id,
                    "lease-history: execution absent (terminal / purged) — skipping"
                );
                return None;
            }
            Err(e) => {
                tracing::warn!(
                    exec_id = %execution_id,
                    error = %e,
                    "lease-history: describe_execution failed — skipping event"
                );
                return None;
            }
        };
        match snapshot.tags.get("cairn.instance_id") {
            Some(tag) if tag == &self.own_instance_id => {}
            Some(other) => {
                tracing::trace!(
                    exec_id = %execution_id,
                    foreign = %other,
                    "lease-history: event for foreign instance (backend filter skew?); skipping"
                );
                return None;
            }
            None => {
                tracing::trace!(
                    exec_id = %execution_id,
                    "lease-history: execution missing cairn.instance_id tag; skipping"
                );
                return None;
            }
        }
        let project_str = snapshot.tags.get("cairn.project")?;
        let project = try_parse_project_key(project_str)?;
        if let Some(task_id) = snapshot.tags.get("cairn.task_id") {
            Some(EntityContext::Task {
                project,
                task_id: TaskId::new(task_id.clone()),
                lease_epoch: snapshot
                    .current_lease
                    .as_ref()
                    .map(|l| l.lease_epoch.0)
                    .unwrap_or(0),
                lease_expires_at_ms: snapshot
                    .current_lease
                    .as_ref()
                    .map(|l| l.expires_at.0.max(0) as u64)
                    .unwrap_or(0),
            })
        } else {
            snapshot
                .tags
                .get("cairn.run_id")
                .map(|run_id| EntityContext::Run {
                    project,
                    run_id: RunId::new(run_id.clone()),
                })
        }
    }

    async fn dispatch_expired(&self, execution_id: &ExecutionId) {
        let Some(ctx) = self.resolve_context(execution_id).await else {
            return;
        };
        match ctx {
            EntityContext::Task {
                project, task_id, ..
            } => {
                self.bridge
                    .emit(BridgeEvent::TaskStateChanged {
                        task_id,
                        project,
                        to: TaskState::RetryableFailed,
                        failure_class: Some(FailureClass::LeaseExpired),
                    })
                    .await;
            }
            EntityContext::Run { project, run_id } => {
                self.bridge
                    .emit(BridgeEvent::ExecutionFailed {
                        run_id,
                        project,
                        failure_class: FailureClass::LeaseExpired,
                        prev_state: None,
                    })
                    .await;
            }
        }
    }

    async fn dispatch_reclaimed(
        &self,
        execution_id: &ExecutionId,
        new_owner: Option<flowfabric::core::types::WorkerInstanceId>,
    ) {
        // On reclaim FF minted a fresh lease + new worker. For tasks
        // we emit `TaskLeaseClaimed` so the projection surfaces the
        // new owner. Runs have no dedicated `RunLeaseClaimed` variant
        // — the next cairn-side transition re-syncs the projection.
        let Some(ctx) = self.resolve_context(execution_id).await else {
            return;
        };
        let EntityContext::Task {
            project,
            task_id,
            lease_epoch,
            lease_expires_at_ms,
        } = ctx
        else {
            return;
        };
        let lease_owner = new_owner.map(|w| w.as_str().to_owned()).unwrap_or_default();
        self.bridge
            .emit(BridgeEvent::TaskLeaseClaimed {
                task_id,
                project,
                lease_owner,
                lease_epoch,
                lease_expires_at_ms,
            })
            .await;
    }
}

enum EntityContext {
    Task {
        project: ProjectKey,
        task_id: TaskId,
        /// Snapshot-sourced lease epoch for the reclaim-event path.
        /// Ignored on expiry (lease already gone).
        lease_epoch: u64,
        /// Snapshot-sourced lease expiry for the reclaim-event path.
        /// Zero is a fall-through sentinel; the next worker heartbeat
        /// corrects it.
        lease_expires_at_ms: u64,
    },
    Run {
        project: ProjectKey,
        run_id: RunId,
    },
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ─── Cursor-row codec ────────────────────────────────────────────────
//
// The legacy `FfLeaseHistoryCursor.last_stream_id` column is a Valkey
// stream-id string (e.g. `"1700000000000-42"`). CG-c stores an opaque
// FF `StreamCursor` byte sequence — a prefix byte + 16 bytes of
// positional data (Valkey) or 8 bytes (Postgres). We encode as base64
// so the existing `String` column roundtrips bytes without a schema
// migration. Legacy rows (Valkey stream-id strings) at the sentinel
// key path are never read because we use a reserved sentinel row and
// prune old rows at boot (see `prune_legacy_rows` below, invoked by
// the caller).

/// Tiny URL-safe base64 without padding. The cursor byte space is
/// small (<= ~17 bytes for Valkey; 9 for Postgres) so a dependency
/// on `base64` would be overweight; a ~30-LOC hand-rolled codec keeps
/// the crate graph lean. URL-safe avoids any collation-sensitive
/// column treatment on future store backends.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8) | (bytes[i + 2] as u32);
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
        out.push(ALPHABET[(n & 0x3F) as usize] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let n = (bytes[i] as u32) << 16;
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
    } else if rem == 2 {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
    }
    out
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    fn dec(c: u8) -> Result<u8, String> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'-' => Ok(62),
            b'_' => Ok(63),
            _ => Err(format!("invalid base64 character: {c:?}")),
        }
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut i = 0;
    while i + 4 <= bytes.len() {
        let n = ((dec(bytes[i])? as u32) << 18)
            | ((dec(bytes[i + 1])? as u32) << 12)
            | ((dec(bytes[i + 2])? as u32) << 6)
            | (dec(bytes[i + 3])? as u32);
        out.push((n >> 16) as u8);
        out.push((n >> 8) as u8);
        out.push(n as u8);
        i += 4;
    }
    let rem = bytes.len() - i;
    if rem == 2 {
        let n = ((dec(bytes[i])? as u32) << 18) | ((dec(bytes[i + 1])? as u32) << 12);
        out.push((n >> 16) as u8);
    } else if rem == 3 {
        let n = ((dec(bytes[i])? as u32) << 18)
            | ((dec(bytes[i + 1])? as u32) << 12)
            | ((dec(bytes[i + 2])? as u32) << 6);
        out.push((n >> 16) as u8);
        out.push((n >> 8) as u8);
    } else if rem != 0 {
        return Err("truncated base64 input".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{base64_decode, base64_encode};

    #[test]
    fn base64_roundtrip_empty() {
        assert_eq!(base64_encode(&[]), "");
        assert_eq!(base64_decode("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn base64_roundtrip_one_byte() {
        let raw = [0x01];
        assert_eq!(base64_decode(&base64_encode(&raw)).unwrap(), raw);
    }

    #[test]
    fn base64_roundtrip_two_bytes() {
        let raw = [0x01, 0x02];
        assert_eq!(base64_decode(&base64_encode(&raw)).unwrap(), raw);
    }

    #[test]
    fn base64_roundtrip_three_bytes() {
        let raw = [0x01, 0x02, 0x03];
        assert_eq!(base64_decode(&base64_encode(&raw)).unwrap(), raw);
    }

    #[test]
    fn base64_roundtrip_valkey_cursor_shape() {
        // Representative Valkey cursor: 1 prefix byte + 16 position bytes.
        let raw = [
            0x01, 0x00, 0x00, 0x01, 0x8A, 0x2B, 0x3C, 0x4D, 0x5E, 0x6F, 0x70, 0x81, 0x92, 0xA3,
            0xB4, 0xC5, 0xD6,
        ];
        assert_eq!(base64_decode(&base64_encode(&raw)).unwrap(), raw);
    }

    #[test]
    fn base64_rejects_invalid_character() {
        let err = base64_decode("###!").unwrap_err();
        assert!(err.contains("invalid base64"), "error: {err}");
    }

    #[test]
    fn base64_rejects_truncated_input() {
        let err = base64_decode("A").unwrap_err();
        assert!(err.contains("truncated"), "error: {err}");
    }
}
