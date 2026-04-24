//! Shared helper for the orchestrator's tracing side-channel.
//!
//! Both `handlers/runs.rs` and `handlers/github.rs` wrap the base SSE
//! emitter in a `TracingEmitter` that appends provider-call telemetry
//! events and inserts an LLM call trace row on every DECIDE callback.
//! Those two implementations used to live as near-verbatim copies; to
//! keep the FK-ordering invariant (RouteDecisionMade MUST be appended
//! before ProviderCallCompleted in the same `append` call) in one
//! place, the body lives here and the emitters delegate to
//! [`record_decide_trace`].
//!
//! The helper MUST NOT be called outside `on_decide_completed`
//! wrappers — it synthesises telemetry identifiers (`call_id`,
//! `route_decision_id`, `route_attempt_id`) from the
//! `OrchestrationContext::run_id` and the wall clock, which are only
//! meaningful for a single decide-phase trace record.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use cairn_domain::{
    events::{ProviderCallCompleted, RouteDecisionMade},
    providers::{OperationKind, ProviderCallStatus, RouteDecisionStatus},
    EventEnvelope, EventId, EventSource, LlmCallTrace, ProviderBindingId, ProviderCallId,
    ProviderConnectionId, ProviderModelId, RouteAttemptId, RouteDecisionId, RuntimeEvent, TaskId,
};
use cairn_orchestrator::{DecideOutput, OrchestrationContext};
use cairn_runtime::telemetry::OtlpExporter;
use cairn_store::{projections::LlmCallTraceReadModel, EventLog, InMemoryStore};

/// Append RouteDecisionMade + ProviderCallCompleted telemetry, export
/// the provider span via OTLP, and insert an `LlmCallTrace` row.
///
/// # FK ordering invariant
///
/// `RouteDecisionMade` is appended BEFORE `ProviderCallCompleted` in
/// the same `EventLog::append(&[...])` call. `provider_calls.route_decision_id`
/// has a FK to `route_decisions(route_decision_id)`; both projections
/// run inside a single `PgEventLog::append` transaction and are applied
/// in slice order. Appending them out of order, or splitting into two
/// calls, produces an FK violation on the Postgres secondary — and a
/// silent divergence from the InMemory primary (no FK there).
///
/// # Fail-loud contract
///
/// On append failure, the detailed error is logged at ERROR and a
/// class-level message ("dual-write divergence on run={run_id}") is
/// written into `fatal_error_slot`. The orchestrator loop consults
/// this slot via `OrchestratorEventEmitter::take_fatal_error` after
/// the DECIDE callback and aborts with `OrchestratorError::Store`.
///
/// SEC-007: the slot message is class-level only — the raw driver
/// text is NOT folded into the public error surface.
pub(crate) async fn record_decide_trace(
    ctx: &OrchestrationContext,
    d: &DecideOutput,
    store: &Arc<InMemoryStore>,
    exporter: &Arc<OtlpExporter>,
    fatal_error_slot: &Mutex<Option<String>>,
) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    // Include `ctx.iteration` AND a monotonic nonce so two DECIDE
    // completions in the same millisecond for the same run produce
    // distinct event_ids. `event_log.event_id` is UNIQUE on the
    // durable backends; a collision there would cascade into a
    // spurious "dual-write divergence" fatal under the new
    // fail-loud contract. The nonce is a process-wide counter —
    // cheap, wait-free, and sufficient since event_id uniqueness
    // is only enforced per-instance (each cairn-app has its own
    // sequence space).
    static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let call_id = format!(
        "orch_{}_i{}_{}_{}",
        ctx.run_id.as_str(),
        ctx.iteration,
        now,
        nonce
    );
    let route_decision_id = RouteDecisionId::new(format!("rd_{call_id}"));
    let route_attempt_id = RouteAttemptId::new(format!("ra_{call_id}"));
    let provider_binding_id = ProviderBindingId::new("brain");

    let route_event = EventEnvelope::for_runtime_event(
        EventId::new(format!("evt_route_{call_id}")),
        EventSource::Runtime,
        RuntimeEvent::RouteDecisionMade(RouteDecisionMade {
            project: ctx.project.clone(),
            route_decision_id: route_decision_id.clone(),
            operation_kind: OperationKind::Generate,
            selected_provider_binding_id: Some(provider_binding_id.clone()),
            final_status: RouteDecisionStatus::Selected,
            attempt_count: 1,
            fallback_used: false,
            decided_at: now,
        }),
    );

    // Approximate cost: $0.50/M input, $1.50/M output (generic
    // estimate; multiply first to avoid integer truncation on small
    // token counts).
    let input_tokens_u = d.input_tokens.unwrap_or(0);
    let output_tokens_u = d.output_tokens.unwrap_or(0);
    let cost_micros = ((input_tokens_u as u64).saturating_mul(500)
        + (output_tokens_u as u64).saturating_mul(1500))
        / 1_000;

    let provider_event = EventEnvelope::for_runtime_event(
        EventId::new(format!("evt_trace_{call_id}")),
        EventSource::Runtime,
        RuntimeEvent::ProviderCallCompleted(ProviderCallCompleted {
            project: ctx.project.clone(),
            provider_call_id: ProviderCallId::new(&call_id),
            route_decision_id,
            route_attempt_id,
            provider_binding_id,
            provider_connection_id: ProviderConnectionId::new("brain"),
            provider_model_id: ProviderModelId::new(&d.model_id),
            operation_kind: OperationKind::Generate,
            status: ProviderCallStatus::Succeeded,
            latency_ms: Some(d.latency_ms),
            input_tokens: d.input_tokens,
            output_tokens: d.output_tokens,
            cost_micros: Some(cost_micros),
            completed_at: now,
            session_id: Some(ctx.session_id.clone()),
            run_id: Some(ctx.run_id.clone()),
            error_class: None,
            raw_error_message: None,
            retry_count: 0,
            task_id: ctx.task_id.as_ref().map(|t| TaskId::new(t.as_str())),
            prompt_release_id: None,
            fallback_position: 0,
            started_at: now.saturating_sub(d.latency_ms),
            finished_at: now,
        }),
    );
    let provider_payload = provider_event.payload.clone();

    if let Err(e) = store.append(&[route_event, provider_event]).await {
        tracing::error!(
            run_id = %ctx.run_id,
            error = %e,
            "event store append failed — in-memory/secondary logs have diverged, aborting run"
        );
        // SEC-007: keep the latched public message class-level only.
        // The `{e}` detail (raw driver text, constraint names, schema
        // fragments) is logged at ERROR above for operators; folding
        // it into the `OrchestratorError::Store` message would leak
        // internals through the API surface.
        let mut slot = fatal_error_slot.lock().unwrap_or_else(|p| p.into_inner());
        *slot = Some(format!(
            "dual-write divergence on run={}",
            ctx.run_id.as_str()
        ));
        // Skip OTLP export and LlmCallTrace insert: the loop will
        // abort on the latched fatal on the next `take_fatal_error`
        // consult, and neither downstream side-effect is safe to
        // perform while the primary/secondary have diverged — the
        // trace would reference a provider_call row that never
        // landed on the durable backend.
        return;
    }

    let _ = exporter.export_event(&provider_payload).await;

    let trace = LlmCallTrace {
        trace_id: call_id,
        model_id: d.model_id.clone(),
        prompt_tokens: input_tokens_u,
        completion_tokens: output_tokens_u,
        latency_ms: d.latency_ms,
        cost_micros,
        session_id: Some(ctx.session_id.clone()),
        run_id: Some(ctx.run_id.clone()),
        created_at_ms: now,
        is_error: false,
    };
    let _ = store.insert_trace(trace).await;
}
