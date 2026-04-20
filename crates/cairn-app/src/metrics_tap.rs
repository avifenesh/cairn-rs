//! Event-log subscriber that derives lifecycle metrics from cairn's
//! canonical RuntimeEvent stream.
//!
//! Why a tap instead of direct instrumentation: the services that emit
//! `RunCreated` / `TaskStateChanged` / `ToolInvocationCompleted` live
//! in `cairn-runtime`, `cairn-fabric`, and `cairn-tools` — crates that
//! don't know about `AppMetrics` and shouldn't grow that dependency.
//! The event log is the single bottleneck every mutation funnels
//! through, so tapping it gives us lossless coverage with zero
//! cross-crate wiring.
//!
//! Cadence: driven by the `broadcast::Sender` that cairn-store fans
//! out on every append. Latency from FCALL → metric bump is the
//! event-log transaction + one channel hop (sub-millisecond
//! in-process; the subscriber lives in the same process as the
//! store).
//!
//! Lag: when the broadcast channel overflows (slow consumer), tokio
//! returns `RecvError::Lagged(n)`. We log-and-continue — missing
//! counter bumps are worse than crashing, and the lag count is
//! itself a useful operational signal.

#![cfg(feature = "metrics-core")]

use std::sync::Arc;

use cairn_domain::lifecycle::{RunState, TaskState};
use cairn_domain::RuntimeEvent;
use cairn_store::InMemoryStore;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::metrics::AppMetrics;

/// Handle for the tap task. Clone-safe: the JoinHandle and cancel
/// token are shared behind an `Arc` so `AppState` can stay
/// `#[derive(Clone)]`. Cancel the token and `await` via
/// [`Self::shutdown`] for a synchronous stop; otherwise the task is
/// torn down by the runtime when the process exits.
#[derive(Clone)]
pub struct MetricsTap {
    inner: Arc<MetricsTapInner>,
}

struct MetricsTapInner {
    handle: tokio::sync::Mutex<Option<JoinHandle<()>>>,
    cancel: CancellationToken,
}

impl MetricsTap {
    pub fn spawn(store: Arc<InMemoryStore>, metrics: Arc<AppMetrics>) -> Self {
        let cancel = CancellationToken::new();
        let worker_cancel = cancel.clone();
        // Subscribe on the caller's task, before spawning — if we
        // subscribed inside the spawned task, the caller could append
        // an event between `spawn` returning and the task actually
        // running, and that event would miss the broadcast.
        let mut rx = store.subscribe();
        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = worker_cancel.cancelled() => break,
                    msg = rx.recv() => match msg {
                        Ok(ev) => process_event(&metrics, &ev.envelope.payload),
                        Err(RecvError::Lagged(n)) => {
                            tracing::warn!(
                                dropped = n,
                                "metrics tap: broadcast lag, {n} events missed"
                            );
                        }
                        Err(RecvError::Closed) => break,
                    }
                }
            }
            tracing::info!("metrics tap stopped");
        });
        tracing::info!("metrics tap started");
        Self {
            inner: Arc::new(MetricsTapInner {
                handle: tokio::sync::Mutex::new(Some(handle)),
                cancel,
            }),
        }
    }

    /// Request a graceful stop and await the worker. Idempotent:
    /// subsequent calls are no-ops.
    pub async fn shutdown(&self) {
        self.inner.cancel.cancel();
        let handle = self.inner.handle.lock().await.take();
        if let Some(h) = handle {
            if let Err(e) = h.await {
                tracing::warn!(error = %e, "metrics tap task panicked");
            }
        }
    }
}

fn process_event(metrics: &AppMetrics, event: &RuntimeEvent) {
    match event {
        RuntimeEvent::RunCreated(e) => {
            metrics.record_run_created(
                e.project.tenant_id.as_str(),
                e.project.workspace_id.as_str(),
            );
        }
        RuntimeEvent::RunStateChanged(e) => {
            let (outcome, failure_class) = match e.transition.to {
                RunState::Completed => (Some("completed"), None),
                RunState::Failed => (Some("failed"), e.failure_class.map(failure_label)),
                RunState::Canceled => (Some("canceled"), None),
                _ => (None, None),
            };
            if let Some(outcome) = outcome {
                metrics.record_run_terminal(
                    e.project.tenant_id.as_str(),
                    e.project.workspace_id.as_str(),
                    outcome,
                    failure_class,
                );
            }
            // Symmetric to the task path: `ExecutionFailed { failure_class:
            // LeaseExpired }` surfaces as a Failed transition on a run. The
            // `cairn_lease_expiries_total{entity="run"}` series would never
            // bump otherwise even though the subscriber emits it.
            if matches!(e.transition.to, RunState::Failed)
                && matches!(
                    e.failure_class,
                    Some(cairn_domain::lifecycle::FailureClass::LeaseExpired)
                )
            {
                metrics.record_lease_expiry("run");
            }
        }
        RuntimeEvent::TaskCreated(e) => {
            metrics.record_task_created(
                e.project.tenant_id.as_str(),
                e.project.workspace_id.as_str(),
            );
        }
        RuntimeEvent::TaskStateChanged(e) => {
            let (outcome, failure_class) = match e.transition.to {
                TaskState::Completed => (Some("completed"), None),
                TaskState::Failed => (Some("failed"), e.failure_class.map(failure_label)),
                TaskState::RetryableFailed => {
                    (Some("retryable_failed"), e.failure_class.map(failure_label))
                }
                TaskState::Canceled => (Some("canceled"), None),
                TaskState::DeadLettered => (Some("dead_lettered"), None),
                _ => (None, None),
            };
            if let Some(outcome) = outcome {
                metrics.record_task_terminal(
                    e.project.tenant_id.as_str(),
                    e.project.workspace_id.as_str(),
                    outcome,
                    failure_class,
                );
            }
            // A RetryableFailed with failure_class=LeaseExpired is exactly the
            // signal the lease-history subscriber emits after FF reclaims a
            // dead worker's lease. Counted separately for dashboard visibility
            // without requiring operators to filter on the terminal counter.
            if matches!(e.transition.to, TaskState::RetryableFailed)
                && matches!(
                    e.failure_class,
                    Some(cairn_domain::lifecycle::FailureClass::LeaseExpired)
                )
            {
                metrics.record_lease_expiry("task");
            }
        }
        RuntimeEvent::ToolInvocationCompleted(e) => {
            use cairn_domain::tool_invocation::ToolInvocationOutcomeKind as K;
            let outcome = match e.outcome {
                K::Success => "success",
                K::RetryableFailure => "retryable_failure",
                K::PermanentFailure => "permanent_failure",
                K::Timeout => "timeout",
                K::Canceled => "canceled",
                K::ProtocolViolation => "protocol_violation",
            };
            metrics.record_tool_invocation(&e.tool_name, outcome);
        }
        _ => {}
    }
}

fn failure_label(fc: cairn_domain::lifecycle::FailureClass) -> &'static str {
    use cairn_domain::lifecycle::FailureClass;
    match fc {
        FailureClass::TimedOut => "timed_out",
        FailureClass::DependencyFailed => "dependency_failed",
        FailureClass::ApprovalRejected => "approval_rejected",
        FailureClass::PolicyDenied => "policy_denied",
        FailureClass::ExecutionError => "execution_error",
        FailureClass::LeaseExpired => "lease_expired",
        FailureClass::CanceledByOperator => "canceled_by_operator",
    }
}
