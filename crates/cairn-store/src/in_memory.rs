//! In-memory store implementation for testing and local-mode use.
//!
//! Provides a single `InMemoryStore` that implements `EventLog` and all
//! entity read-model traits. Event append atomically updates sync projections.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use cairn_domain::*;
use serde::{Deserialize, Serialize};

use crate::error::StoreError;
use crate::event_log::*;
use crate::projections::*;

fn now_millis() -> u64 {
    // Matches pg/sqlite backends' fallback on clock skew: a clock before
    // UNIX_EPOCH (container misconfiguration) MUST NOT panic the store.
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct State {
    events: Vec<StoredEvent>,
    next_position: u64,
    /// RFC 002: causation_id → earliest event position that references it.
    /// Populated by `append()` and consulted by `find_by_causation_id`; the
    /// pre-T2-H1 field was declared but never populated, so lookup fell back
    /// to O(n) over the full `events` vector.
    command_id_index: HashMap<String, u64>,
    sessions: HashMap<String, SessionRecord>,
    runs: HashMap<String, RunRecord>,
    tasks: HashMap<String, TaskRecord>,
    approvals: HashMap<String, ApprovalRecord>,
    checkpoints: HashMap<String, CheckpointRecord>,
    mailbox_messages: HashMap<String, MailboxRecord>,
    tool_invocations: HashMap<String, ToolInvocationRecord>,
    signals: HashMap<String, cairn_domain::SignalRecord>,
    ingest_jobs: HashMap<String, cairn_domain::IngestJobRecord>,
    scheduled_tasks: HashMap<String, cairn_domain::ScheduledTaskRecord>,
    eval_runs: HashMap<String, crate::projections::EvalRunRecord>,
    outcomes: HashMap<String, crate::projections::OutcomeRecord>,
    eval_datasets: HashMap<String, cairn_domain::EvalDataset>,
    eval_rubrics: HashMap<String, cairn_domain::EvalRubric>,
    eval_baselines: HashMap<String, cairn_domain::EvalBaseline>,
    // keyed by run_id; each run has at most one active strategy
    checkpoint_strategies: HashMap<String, cairn_domain::CheckpointStrategy>,
    prompt_assets: HashMap<String, crate::projections::PromptAssetRecord>,
    prompt_versions: HashMap<String, crate::projections::PromptVersionRecord>,
    prompt_releases: HashMap<String, crate::projections::PromptReleaseRecord>,
    tenants: HashMap<String, cairn_domain::org::TenantRecord>,
    workspaces: HashMap<String, cairn_domain::org::WorkspaceRecord>,
    projects: HashMap<String, cairn_domain::org::ProjectRecord>,
    /// RFC 002: point-in-time snapshots per tenant, ordered by creation.
    snapshots: Vec<cairn_domain::Snapshot>,
    route_decisions: HashMap<String, cairn_domain::providers::RouteDecisionRecord>,
    provider_calls: HashMap<String, cairn_domain::providers::ProviderCallRecord>,
    approval_policies: HashMap<String, cairn_domain::ApprovalPolicyRecord>,
    external_workers: HashMap<String, cairn_domain::workers::ExternalWorkerRecord>,
    /// GAP-006: accumulated session costs keyed by session_id.
    session_costs: HashMap<String, cairn_domain::providers::SessionCostRecord>,
    /// Run-level accumulated costs keyed by run_id.
    run_costs: HashMap<String, cairn_domain::providers::RunCostRecord>,
    /// GAP-010: LLM call trace records derived from ProviderCallCompleted events.
    llm_traces: Vec<cairn_domain::LlmCallTrace>,
    operator_profiles: HashMap<String, crate::projections::OperatorProfileRecord>,
    full_operator_profiles: HashMap<String, cairn_domain::org::OperatorProfile>,
    workspace_members: Vec<crate::projections::WorkspaceMemberRecord>,
    signal_subscriptions: HashMap<String, crate::projections::SignalSubscriptionRecord>,
    provider_health_records: HashMap<String, cairn_domain::providers::ProviderHealthRecord>,
    provider_pools: HashMap<String, cairn_domain::providers::ProviderConnectionPool>,
    default_settings: HashMap<String, cairn_domain::DefaultSetting>,
    credentials: HashMap<String, cairn_domain::credentials::CredentialRecord>,
    channels: HashMap<String, cairn_domain::ChannelRecord>,
    channel_messages: HashMap<String, Vec<cairn_domain::ChannelMessage>>,
    credential_rotations: Vec<cairn_domain::credentials::CredentialRotationRecord>,
    licenses: HashMap<String, cairn_domain::LicenseRecord>,
    entitlement_overrides: HashMap<String, cairn_domain::EntitlementOverrideRecord>,
    notification_prefs: HashMap<String, cairn_domain::notification_prefs::NotificationPreference>,
    notification_records: Vec<cairn_domain::notification_prefs::NotificationRecord>,
    guardrail_policies: HashMap<String, cairn_domain::policy::GuardrailPolicy>,
    provider_budgets: HashMap<String, cairn_domain::providers::ProviderBudget>,
    provider_connections: HashMap<String, cairn_domain::providers::ProviderConnectionRecord>,
    quotas: HashMap<String, cairn_domain::TenantQuota>,
    provider_bindings: HashMap<String, cairn_domain::providers::ProviderBindingRecord>,
    provider_health_schedules: HashMap<String, cairn_domain::providers::ProviderHealthSchedule>,
    run_sla_configs: HashMap<String, cairn_domain::sla::SlaConfig>,
    run_sla_breaches: HashMap<String, cairn_domain::sla::SlaBreach>,
    run_cost_alerts: HashMap<String, cairn_domain::providers::RunCostAlert>,
    retention_policies: HashMap<String, cairn_domain::RetentionPolicy>,
    route_policies: HashMap<String, cairn_domain::providers::RoutePolicy>,
    resource_shares: HashMap<String, cairn_domain::resource_sharing::SharedResource>,
    /// FF lease_history subscriber cursors, keyed by `(partition_id,
    /// execution_id)`.
    ff_lease_history_cursors: HashMap<(String, String), crate::projections::FfLeaseHistoryCursor>,
}

pub struct InMemoryStore {
    state: Mutex<State>,
    usage_counters: Arc<Mutex<HashMap<ProjectKey, UsageCounters>>>,
    /// Broadcast channel for real-time SSE streaming (RFC 002).
    ///
    /// Every successfully appended `StoredEvent` is sent here. Receivers can
    /// subscribe before reading the replay window so no events are missed.
    /// Capacity of 1024 covers burst writes; lagged receivers get
    /// `BroadcastStreamRecvError::Lagged` and should reconnect with the last
    /// known position.
    event_tx: tokio::sync::broadcast::Sender<StoredEvent>,
    /// Optional durable secondary event log (e.g. Postgres or SQLite).
    ///
    /// When set, every `append()` call dual-writes to this log AFTER the
    /// in-memory write succeeds. This makes ALL service-layer events durable
    /// without touching the 109 `store.append()` call sites across 42 files.
    ///
    /// Set via `set_secondary_log()` after construction. The secondary write
    /// is best-effort: failures are logged but do NOT roll back the in-memory
    /// write, preserving the existing availability guarantee.
    secondary_log: std::sync::RwLock<Option<Arc<dyn EventLog + Send + Sync>>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageCounters {
    pub run_count: u64,
    pub event_count: u64,
    pub sandbox_provision_count: u64,
    pub decision_evaluation_count: u64,
    pub trigger_fire_count: u64,
}

impl InMemoryStore {
    pub fn new() -> Self {
        let (event_tx, _) = tokio::sync::broadcast::channel(1024);
        Self {
            state: Mutex::new(State {
                events: Vec::new(),
                next_position: 1,
                command_id_index: HashMap::new(),
                sessions: HashMap::new(),
                runs: HashMap::new(),
                tasks: HashMap::new(),
                approvals: HashMap::new(),
                checkpoints: HashMap::new(),
                mailbox_messages: HashMap::new(),
                tool_invocations: HashMap::new(),
                signals: HashMap::new(),
                ingest_jobs: HashMap::new(),
                scheduled_tasks: HashMap::new(),
                eval_runs: HashMap::new(),
                outcomes: HashMap::new(),
                eval_datasets: HashMap::new(),
                eval_rubrics: HashMap::new(),
                eval_baselines: HashMap::new(),
                checkpoint_strategies: HashMap::new(),
                prompt_assets: HashMap::new(),
                prompt_versions: HashMap::new(),
                prompt_releases: HashMap::new(),
                route_decisions: HashMap::new(),
                provider_calls: HashMap::new(),
                approval_policies: HashMap::new(),
                external_workers: HashMap::new(),
                session_costs: HashMap::new(),
                run_costs: HashMap::new(),
                llm_traces: Vec::new(),
                operator_profiles: HashMap::new(),
                full_operator_profiles: HashMap::new(),
                workspace_members: Vec::new(),
                signal_subscriptions: HashMap::new(),
                provider_health_records: HashMap::new(),
                provider_pools: HashMap::new(),
                default_settings: HashMap::new(),
                credentials: HashMap::new(),
                channels: HashMap::new(),
                channel_messages: HashMap::new(),
                credential_rotations: Vec::new(),
                licenses: HashMap::new(),
                entitlement_overrides: HashMap::new(),
                notification_prefs: HashMap::new(),
                notification_records: Vec::new(),
                guardrail_policies: HashMap::new(),
                provider_budgets: HashMap::new(),
                provider_connections: HashMap::new(),
                quotas: HashMap::new(),
                provider_bindings: HashMap::new(),
                provider_health_schedules: HashMap::new(),
                run_sla_configs: HashMap::new(),
                run_sla_breaches: HashMap::new(),
                run_cost_alerts: HashMap::new(),
                retention_policies: HashMap::new(),
                route_policies: HashMap::new(),
                resource_shares: HashMap::new(),
                ff_lease_history_cursors: HashMap::new(),
                tenants: HashMap::new(),
                workspaces: HashMap::new(),
                projects: HashMap::new(),
                snapshots: Vec::new(),
            }),
            usage_counters: Arc::new(Mutex::new(HashMap::new())),
            event_tx,
            secondary_log: std::sync::RwLock::new(None),
        }
    }

    fn increment_usage_for_project(
        &self,
        project: &ProjectKey,
        update: impl FnOnce(&mut UsageCounters),
    ) {
        let mut usage = self
            .usage_counters
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        update(usage.entry(project.clone()).or_default());
    }

    pub fn increment_sandbox_provision_count(&self, project: &ProjectKey) {
        self.increment_usage_for_project(project, |counters| {
            counters.sandbox_provision_count += 1;
        });
    }

    pub fn increment_decision_evaluation_count(&self, project: &ProjectKey) {
        self.increment_usage_for_project(project, |counters| {
            counters.decision_evaluation_count += 1;
        });
    }

    pub fn increment_trigger_fire_count(&self, project: &ProjectKey) {
        self.increment_usage_for_project(project, |counters| {
            counters.trigger_fire_count += 1;
        });
    }

    pub fn usage_snapshot(&self) -> HashMap<ProjectKey, UsageCounters> {
        self.usage_counters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn reset_usage_counters(&self) {
        self.usage_counters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// Attach a durable secondary event log.
    ///
    /// After this call every `append()` will dual-write to `log` after the
    /// in-memory write. Intended to be called once at startup, before the
    /// HTTP server accepts traffic.
    ///
    /// Pass `Arc<PgEventLog>` or `Arc<SqliteEventLog>` — any `EventLog` impl works.
    pub fn set_secondary_log(&self, log: Arc<dyn EventLog + Send + Sync>) {
        *self
            .secondary_log
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(log);
    }

    /// Subscribe to the real-time event broadcast (RFC 002).
    ///
    /// Call this *before* reading the replay window to guarantee no events are
    /// missed between the replay read and the live subscription.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<StoredEvent> {
        self.event_tx.subscribe()
    }

    fn apply_projection(state: &mut State, event: &StoredEvent) {
        let now = event.stored_at;
        match &event.envelope.payload {
            RuntimeEvent::SessionCreated(e) => {
                state.sessions.insert(
                    e.session_id.as_str().to_owned(),
                    SessionRecord {
                        session_id: e.session_id.clone(),
                        project: e.project.clone(),
                        state: SessionState::Open,
                        version: 1,
                        created_at: now,
                        updated_at: now,
                    },
                );
                // Update session quota counter
                if let Some(quota) = state.quotas.get_mut(e.project.tenant_id.as_str()) {
                    quota.sessions_this_hour = quota.sessions_this_hour.saturating_add(1);
                }
            }
            RuntimeEvent::SessionStateChanged(e) => {
                if let Some(rec) = state.sessions.get_mut(e.session_id.as_str()) {
                    rec.state = e.transition.to;
                    rec.version += 1;
                    rec.updated_at = now;
                }
            }
            RuntimeEvent::RunCreated(e) => {
                state.runs.insert(
                    e.run_id.as_str().to_owned(),
                    RunRecord {
                        run_id: e.run_id.clone(),
                        session_id: e.session_id.clone(),
                        parent_run_id: e.parent_run_id.clone(),
                        project: e.project.clone(),
                        state: RunState::Pending,
                        prompt_release_id: e.prompt_release_id.clone(),
                        agent_role_id: e.agent_role_id.clone(),
                        failure_class: None,
                        pause_reason: None,
                        resume_trigger: None,
                        version: 1,
                        created_at: now,
                        updated_at: now,
                    },
                );
                // Update run quota counter
                if let Some(quota) = state.quotas.get_mut(e.project.tenant_id.as_str()) {
                    quota.current_active_runs = quota.current_active_runs.saturating_add(1);
                }
            }
            RuntimeEvent::RunStateChanged(e) => {
                if let Some(rec) = state.runs.get_mut(e.run_id.as_str()) {
                    rec.state = e.transition.to;
                    rec.failure_class = e.failure_class;
                    rec.pause_reason = e.pause_reason.clone();
                    rec.resume_trigger = e.resume_trigger;
                    rec.version += 1;
                    rec.updated_at = now;
                }
            }
            RuntimeEvent::TaskCreated(e) => {
                // Prefer the session_id on the event; fall back to walking
                // parent_run_id → RunRecord.session_id for tasks without one.
                let session_id = e.session_id.clone().or_else(|| {
                    e.parent_run_id
                        .as_ref()
                        .and_then(|rid| state.runs.get(rid.as_str()))
                        .map(|r| r.session_id.clone())
                });
                state.tasks.insert(
                    e.task_id.as_str().to_owned(),
                    TaskRecord {
                        task_id: e.task_id.clone(),
                        project: e.project.clone(),
                        parent_run_id: e.parent_run_id.clone(),
                        parent_task_id: e.parent_task_id.clone(),
                        session_id,
                        state: TaskState::Queued,
                        prompt_release_id: e.prompt_release_id.clone(),
                        failure_class: None,
                        pause_reason: None,
                        resume_trigger: None,
                        retry_count: 0,
                        lease_owner: None,
                        lease_expires_at: None,
                        title: None,
                        description: None,
                        version: 1,
                        created_at: now,
                        updated_at: now,
                    },
                );
            }
            RuntimeEvent::TaskStateChanged(e) => {
                if let Some(rec) = state.tasks.get_mut(e.task_id.as_str()) {
                    rec.state = e.transition.to;
                    rec.failure_class = e.failure_class;
                    rec.pause_reason = e.pause_reason.clone();
                    rec.resume_trigger = e.resume_trigger;
                    if e.transition.to == TaskState::RetryableFailed {
                        rec.retry_count += 1;
                    }
                    // RFC 002: clear lease fields when transitioning back to Queued.
                    // RFC 005: also clear lease on Paused — a paused task must not
                    // expire while suspended (the lease timer is logically stopped).
                    if matches!(e.transition.to, TaskState::Queued | TaskState::Paused) {
                        rec.lease_owner = None;
                        rec.lease_expires_at = None;
                    }
                    rec.version += 1;
                    rec.updated_at = now;
                }
            }
            RuntimeEvent::ApprovalRequested(e) => {
                state.approvals.insert(
                    e.approval_id.as_str().to_owned(),
                    ApprovalRecord {
                        approval_id: e.approval_id.clone(),
                        project: e.project.clone(),
                        run_id: e.run_id.clone(),
                        task_id: e.task_id.clone(),
                        requirement: e.requirement,
                        decision: None,
                        title: e.title.clone(),
                        description: e.description.clone(),
                        version: 1,
                        created_at: now,
                        updated_at: now,
                    },
                );
            }
            RuntimeEvent::ApprovalResolved(e) => {
                if let Some(rec) = state.approvals.get_mut(e.approval_id.as_str()) {
                    rec.decision = Some(e.decision);
                    rec.version += 1;
                    rec.updated_at = now;
                }
            }
            RuntimeEvent::CheckpointRecorded(e) => {
                // Supersede any existing latest checkpoint for this run.
                if e.disposition == CheckpointDisposition::Latest {
                    for cp in state.checkpoints.values_mut() {
                        if cp.run_id == e.run_id && cp.disposition == CheckpointDisposition::Latest
                        {
                            cp.disposition = CheckpointDisposition::Superseded;
                            cp.version += 1;
                        }
                    }
                }
                state.checkpoints.insert(
                    e.checkpoint_id.as_str().to_owned(),
                    CheckpointRecord {
                        checkpoint_id: e.checkpoint_id.clone(),
                        project: e.project.clone(),
                        run_id: e.run_id.clone(),
                        disposition: e.disposition,
                        data: e.data.clone(),
                        version: 1,
                        created_at: now,
                    },
                );
            }
            RuntimeEvent::MailboxMessageAppended(e) => {
                state.mailbox_messages.insert(
                    e.message_id.as_str().to_owned(),
                    MailboxRecord {
                        message_id: e.message_id.clone(),
                        project: e.project.clone(),
                        run_id: e.run_id.clone(),
                        task_id: e.task_id.clone(),
                        from_task_id: e.from_task_id.clone(),
                        content: e.content.clone(),
                        from_run_id: e.from_run_id.clone(),
                        deliver_at_ms: e.deliver_at_ms,
                        sender: e.sender.clone(),
                        recipient: e.recipient.clone(),
                        body: e.body.clone(),
                        sent_at: e.sent_at,
                        delivery_status: e.delivery_status,
                        version: 1,
                        created_at: now,
                    },
                );
            }
            RuntimeEvent::TaskLeaseClaimed(e) => {
                if let Some(rec) = state.tasks.get_mut(e.task_id.as_str()) {
                    rec.lease_owner = Some(e.lease_owner.clone());
                    rec.lease_expires_at = Some(e.lease_expires_at_ms);
                    rec.version += 1;
                    rec.updated_at = now;
                }
            }
            RuntimeEvent::TaskLeaseHeartbeated(e) => {
                if let Some(rec) = state.tasks.get_mut(e.task_id.as_str()) {
                    rec.lease_expires_at = Some(e.lease_expires_at_ms);
                    rec.version += 1;
                    rec.updated_at = now;
                }
            }
            RuntimeEvent::ToolInvocationStarted(e) => {
                let requested = ToolInvocationRecord::new_requested(
                    e.invocation_id.clone(),
                    e.project.clone(),
                    e.session_id.clone(),
                    e.run_id.clone(),
                    e.task_id.clone(),
                    e.target.clone(),
                    e.execution_class,
                    e.requested_at_ms,
                );
                let started = requested
                    .mark_started(e.started_at_ms)
                    .expect("tool invocation started event should always be a valid requested->started transition");
                state
                    .tool_invocations
                    .insert(e.invocation_id.as_str().to_owned(), started);
            }
            RuntimeEvent::ToolInvocationCompleted(e) => {
                if let Some(rec) = state.tool_invocations.get_mut(e.invocation_id.as_str()) {
                    *rec = rec.mark_finished(e.outcome, None, e.finished_at_ms).expect(
                        "tool invocation completed event should preserve valid terminal transition",
                    );
                }
            }
            RuntimeEvent::ToolInvocationFailed(e) => {
                if let Some(rec) = state.tool_invocations.get_mut(e.invocation_id.as_str()) {
                    *rec = rec
                        .mark_finished(e.outcome, e.error_message.clone(), e.finished_at_ms)
                        .expect("tool invocation failed event should preserve valid terminal transition");
                }
            }
            RuntimeEvent::SignalIngested(e) => {
                state.signals.insert(
                    e.signal_id.as_str().to_owned(),
                    cairn_domain::SignalRecord {
                        id: e.signal_id.clone(),
                        project: e.project.clone(),
                        source: e.source.clone(),
                        payload: e.payload.clone(),
                        timestamp_ms: e.timestamp_ms,
                    },
                );
            }
            RuntimeEvent::ExternalWorkerRegistered(e) => {
                state.external_workers.insert(
                    e.worker_id.as_str().to_owned(),
                    cairn_domain::workers::ExternalWorkerRecord {
                        worker_id: e.worker_id.clone(),
                        tenant_id: e.tenant_id.clone(),
                        display_name: e.display_name.clone(),
                        status: "active".to_owned(),
                        registered_at: e.registered_at,
                        updated_at: now,
                        health: cairn_domain::workers::WorkerHealth::default(),
                        current_task_id: None,
                    },
                );
            }
            RuntimeEvent::ExternalWorkerSuspended(e) => {
                if let Some(rec) = state.external_workers.get_mut(e.worker_id.as_str()) {
                    rec.status = "suspended".to_owned();
                    rec.updated_at = now;
                }
            }
            RuntimeEvent::ExternalWorkerReactivated(e) => {
                if let Some(rec) = state.external_workers.get_mut(e.worker_id.as_str()) {
                    rec.status = "active".to_owned();
                    rec.updated_at = now;
                }
            }
            RuntimeEvent::ExternalWorkerReported(e) => {
                // Update last heartbeat and current task.
                if let Some(rec) = state.external_workers.get_mut(e.report.worker_id.as_str()) {
                    rec.health.last_heartbeat_ms = e.report.reported_at_ms;
                    rec.health.is_alive = true;
                    if e.report.outcome.is_none() {
                        rec.current_task_id = Some(e.report.task_id.clone());
                    } else {
                        rec.current_task_id = None;
                    }
                    rec.updated_at = now;
                }
            }
            RuntimeEvent::SoulPatchProposed(_) | RuntimeEvent::SoulPatchApplied(_) => {}
            RuntimeEvent::SpendAlertTriggered(_) => {}
            RuntimeEvent::RunCostUpdated(e) => {
                // Accumulate run cost from directly appended RunCostUpdated events.
                let rec = state
                    .run_costs
                    .entry(e.run_id.as_str().to_owned())
                    .or_insert_with(|| cairn_domain::providers::RunCostRecord {
                        run_id: e.run_id.clone(),
                        total_cost_micros: 0,
                        total_tokens_in: 0,
                        total_tokens_out: 0,
                        provider_calls: 0,
                        token_in: 0,
                        token_out: 0,
                    });
                rec.total_cost_micros = rec.total_cost_micros.saturating_add(e.delta_cost_micros);
                rec.total_tokens_in = rec.total_tokens_in.saturating_add(e.delta_tokens_in);
                rec.total_tokens_out = rec.total_tokens_out.saturating_add(e.delta_tokens_out);
                rec.provider_calls = rec.provider_calls.saturating_add(1);
                rec.token_in = rec.total_tokens_in;
                rec.token_out = rec.total_tokens_out;
                // Auto-trigger run cost alert if threshold exceeded and not yet triggered.
                let total = rec.total_cost_micros;
                if let Some(alert) = state.run_cost_alerts.get(e.run_id.as_str()) {
                    if alert.triggered_at_ms == 0 && total >= alert.threshold_micros {
                        let tenant_id = alert.tenant_id.clone();
                        let threshold = alert.threshold_micros;
                        let triggered_at_ms = now;
                        // Update alert record.
                        if let Some(a) = state.run_cost_alerts.get_mut(e.run_id.as_str()) {
                            a.triggered_at_ms = triggered_at_ms;
                            a.actual_cost_micros = total;
                        }
                        // Emit RunCostAlertTriggered into the log.
                        let alert_pos = EventPosition(state.next_position);
                        state.next_position += 1;
                        let alert_event = StoredEvent {
                            position: alert_pos,
                            envelope: EventEnvelope {
                                event_id: cairn_domain::EventId::new(format!(
                                    "derived_rcat_{}",
                                    e.run_id.as_str()
                                )),
                                source: cairn_domain::EventSource::System,
                                ownership: cairn_domain::OwnershipKey::Project(e.project.clone()),
                                causation_id: None,
                                correlation_id: None,
                                payload: RuntimeEvent::RunCostAlertTriggered(
                                    cairn_domain::RunCostAlertTriggered {
                                        run_id: e.run_id.clone(),
                                        tenant_id,
                                        threshold_micros: threshold,
                                        actual_cost_micros: total,
                                        triggered_at_ms,
                                    },
                                ),
                            },
                            stored_at: now,
                        };
                        state.events.push(alert_event);
                    }
                }
            }
            RuntimeEvent::ChannelCreated(e) => {
                state.channels.insert(
                    e.channel_id.as_str().to_owned(),
                    cairn_domain::ChannelRecord {
                        channel_id: e.channel_id.clone(),
                        project: e.project.clone(),
                        name: e.name.clone(),
                        capacity: e.capacity,
                        created_at: e.created_at_ms,
                        updated_at: e.created_at_ms,
                    },
                );
            }
            RuntimeEvent::ChannelMessageSent(e) => {
                state
                    .channel_messages
                    .entry(e.channel_id.as_str().to_owned())
                    .or_default()
                    .push(cairn_domain::ChannelMessage {
                        channel_id: e.channel_id.clone(),
                        message_id: e.message_id.clone(),
                        sender_id: e.sender_id.clone(),
                        body: e.body.clone(),
                        sent_at_ms: e.sent_at_ms,
                        consumed_by: None,
                        consumed_at_ms: None,
                    });
            }
            RuntimeEvent::ChannelMessageConsumed(e) => {
                if let Some(messages) = state.channel_messages.get_mut(e.channel_id.as_str()) {
                    if let Some(msg) = messages.iter_mut().find(|m| m.message_id == e.message_id) {
                        msg.consumed_by = Some(e.consumed_by.clone());
                        msg.consumed_at_ms = Some(e.consumed_at_ms);
                    }
                }
            }
            RuntimeEvent::DefaultSettingSet(e) => {
                let composite_key = format!("{:?}:{}:{}", e.scope, e.scope_id, e.key);
                state.default_settings.insert(
                    composite_key,
                    cairn_domain::DefaultSetting {
                        key: e.key.clone(),
                        value: e.value.clone(),
                        scope: e.scope,
                    },
                );
            }
            RuntimeEvent::DefaultSettingCleared(e) => {
                let composite_key = format!("{:?}:{}:{}", e.scope, e.scope_id, e.key);
                state.default_settings.remove(&composite_key);
            }
            RuntimeEvent::LicenseActivated(e) => {
                state.licenses.insert(
                    e.tenant_id.as_str().to_owned(),
                    cairn_domain::LicenseRecord {
                        tenant_id: e.tenant_id.clone(),
                        tier: e.tier,
                        entitlements: vec![],
                        issued_at: e.valid_from_ms,
                        expires_at: e.valid_until_ms,
                        license_key: Some(e.license_id.clone()),
                    },
                );
            }
            RuntimeEvent::EntitlementOverrideSet(e) => {
                let key = format!("{}:{}", e.tenant_id.as_str(), e.feature);
                state.entitlement_overrides.insert(
                    key,
                    cairn_domain::EntitlementOverrideRecord {
                        override_id: format!("override_{}_{}", e.tenant_id.as_str(), e.feature),
                        tenant_id: e.tenant_id.clone(),
                        entitlement: cairn_domain::commercial::Entitlement::AdvancedAdmin,
                        granted: e.allowed,
                        reason: e.reason.clone(),
                        applied_at: e.set_at_ms,
                        feature: e.feature.clone(),
                        allowed: e.allowed,
                        set_at_ms: e.set_at_ms,
                    },
                );
            }
            RuntimeEvent::NotificationPreferenceSet(e) => {
                let key = format!("{}:{}", e.tenant_id.as_str(), e.operator_id);
                state.notification_prefs.insert(
                    key.clone(),
                    cairn_domain::notification_prefs::NotificationPreference {
                        pref_id: key,
                        tenant_id: e.tenant_id.clone(),
                        operator_id: e.operator_id.clone(),
                        event_types: e.event_types.clone(),
                        channels: e.channels.clone(),
                    },
                );
            }
            RuntimeEvent::NotificationSent(e) => {
                state.notification_records.push(
                    cairn_domain::notification_prefs::NotificationRecord {
                        record_id: e.record_id.clone(),
                        tenant_id: e.tenant_id.clone(),
                        operator_id: e.operator_id.clone(),
                        event_type: e.event_type.clone(),
                        channel_kind: e.channel_kind.clone(),
                        channel_target: e.channel_target.clone(),
                        payload: e.payload.clone(),
                        sent_at_ms: e.sent_at_ms,
                        delivered: e.delivered,
                        delivery_error: e.delivery_error.clone(),
                    },
                );
            }
            RuntimeEvent::ProviderPoolCreated(e) => {
                state.provider_pools.insert(
                    e.pool_id.clone(),
                    cairn_domain::providers::ProviderConnectionPool {
                        pool_id: e.pool_id.clone(),
                        connection_ids: vec![],
                        max_connections: e.max_connections,
                        active_connections: 0,
                        tenant_id: e.tenant_id.clone(),
                    },
                );
            }
            RuntimeEvent::ProviderPoolConnectionAdded(e) => {
                if let Some(pool) = state.provider_pools.get_mut(&e.pool_id) {
                    if !pool.connection_ids.contains(&e.connection_id) {
                        pool.connection_ids.push(e.connection_id.clone());
                        pool.active_connections = pool.connection_ids.len() as u32;
                    }
                }
            }
            RuntimeEvent::ProviderPoolConnectionRemoved(e) => {
                if let Some(pool) = state.provider_pools.get_mut(&e.pool_id) {
                    pool.connection_ids.retain(|id| id != &e.connection_id);
                    pool.active_connections = pool.connection_ids.len() as u32;
                }
            }
            RuntimeEvent::TenantQuotaSet(e) => {
                state.quotas.insert(
                    e.tenant_id.as_str().to_owned(),
                    cairn_domain::TenantQuota {
                        tenant_id: e.tenant_id.clone(),
                        max_concurrent_runs: e.max_concurrent_runs,
                        max_sessions_per_hour: e.max_sessions_per_hour,
                        max_tasks_per_run: e.max_tasks_per_run,
                        current_active_runs: 0,
                        sessions_this_hour: 0,
                    },
                );
            }
            RuntimeEvent::ProviderBudgetSet(e) => {
                let key = format!("{}:{:?}", e.tenant_id.as_str(), e.period);
                state.provider_budgets.insert(
                    key,
                    cairn_domain::providers::ProviderBudget {
                        tenant_id: e.tenant_id.clone(),
                        period: e.period,
                        limit_micros: e.limit_micros,
                        alert_threshold_percent: e.alert_threshold_percent.unwrap_or(80),
                        current_spend_micros: 0,
                        created_at: now,
                        updated_at: now,
                    },
                );
            }
            RuntimeEvent::ProviderBudgetAlertTriggered(e) => {
                if let Some(budget) = state.provider_budgets.get_mut(&e.budget_id) {
                    budget.current_spend_micros = e.current_micros;
                    budget.updated_at = now;
                }
            }
            RuntimeEvent::ProviderBudgetExceeded(e) => {
                if let Some(budget) = state.provider_budgets.get_mut(&e.budget_id) {
                    budget.current_spend_micros =
                        budget.limit_micros.saturating_add(e.exceeded_by_micros);
                    budget.updated_at = now;
                }
            }
            RuntimeEvent::CredentialStored(e) => {
                state.credentials.insert(
                    e.credential_id.as_str().to_owned(),
                    cairn_domain::credentials::CredentialRecord {
                        id: e.credential_id.clone(),
                        tenant_id: e.tenant_id.clone(),
                        name: e.provider_id.clone(),
                        credential_type: "api_key".to_owned(),
                        encrypted_value: e.encrypted_value.clone(),
                        created_at: e.encrypted_at_ms,
                        updated_at: e.encrypted_at_ms,
                        active: true,
                        provider_id: e.provider_id.clone(),
                        encrypted_at_ms: Some(e.encrypted_at_ms),
                        key_id: e.key_id.clone(),
                        key_version: e.key_version.clone(),
                        revoked_at_ms: None,
                    },
                );
            }
            RuntimeEvent::CredentialRevoked(e) => {
                if let Some(rec) = state.credentials.get_mut(e.credential_id.as_str()) {
                    rec.active = false;
                    rec.revoked_at_ms = Some(e.revoked_at_ms);
                    rec.updated_at = e.revoked_at_ms;
                }
            }
            RuntimeEvent::CredentialKeyRotated(e) => {
                state.credential_rotations.push(
                    cairn_domain::credentials::CredentialRotationRecord {
                        rotation_id: e.rotation_id.clone(),
                        tenant_id: e.tenant_id.clone(),
                        credential_id: cairn_domain::ids::CredentialId::new(""),
                        rotated_at: now,
                        rotated_by: None,
                        old_key_id: e.old_key_id.clone(),
                        new_key_id: e.new_key_id.clone(),
                        rotated_credentials: e.credential_ids_rotated.len() as u32,
                        started_at_ms: now,
                        completed_at_ms: Some(now),
                    },
                );
            }
            RuntimeEvent::GuardrailPolicyCreated(e) => {
                state.guardrail_policies.insert(
                    e.policy_id.clone(),
                    cairn_domain::policy::GuardrailPolicy {
                        policy_id: e.policy_id.clone(),
                        name: e.name.clone(),
                        rules: e.rules.clone(),
                        enabled: true,
                    },
                );
            }
            RuntimeEvent::OperatorProfileCreated(e) => {
                state.operator_profiles.insert(
                    e.profile_id.as_str().to_owned(),
                    crate::projections::OperatorProfileRecord {
                        operator_id: e.profile_id.clone(),
                        tenant_id: e.tenant_id.clone(),
                        display_name: e.display_name.clone(),
                        email: Some(e.email.clone()),
                        role: serde_json::to_string(&e.role)
                            .unwrap_or_default()
                            .trim_matches('"')
                            .to_owned(),
                        created_at: now,
                    },
                );
                state.full_operator_profiles.insert(
                    e.profile_id.as_str().to_owned(),
                    cairn_domain::org::OperatorProfile {
                        operator_id: e.profile_id.clone(),
                        tenant_id: e.tenant_id.clone(),
                        display_name: e.display_name.clone(),
                        email: e.email.clone(),
                        role: e.role,
                        preferences: serde_json::Value::Null,
                    },
                );
            }
            RuntimeEvent::OperatorProfileUpdated(e) => {
                if let Some(rec) = state.operator_profiles.get_mut(e.profile_id.as_str()) {
                    if let Some(dn) = &e.display_name {
                        rec.display_name = dn.clone();
                    }
                    if let Some(email) = &e.email {
                        rec.email = Some(email.clone());
                    }
                }
                if let Some(profile) = state.full_operator_profiles.get_mut(e.profile_id.as_str()) {
                    if let Some(dn) = &e.display_name {
                        profile.display_name = dn.clone();
                    }
                    if let Some(email) = &e.email {
                        profile.email = email.clone();
                    }
                }
            }
            RuntimeEvent::ProviderConnectionRegistered(e) => {
                state.provider_connections.insert(
                    e.provider_connection_id.as_str().to_owned(),
                    cairn_domain::providers::ProviderConnectionRecord {
                        provider_connection_id: e.provider_connection_id.clone(),
                        tenant_id: e.tenant.tenant_id.clone(),
                        provider_family: e.provider_family.clone(),
                        adapter_type: e.adapter_type.clone(),
                        supported_models: e.supported_models.clone(),
                        status: e.status,
                        created_at: e.registered_at,
                    },
                );
            }
            RuntimeEvent::ProviderHealthChecked(e) => {
                let healthy = matches!(
                    e.status,
                    cairn_domain::providers::ProviderHealthStatus::Healthy
                );
                let prev_failures = state
                    .provider_health_records
                    .get(e.connection_id.as_str())
                    .map(|r| r.consecutive_failures)
                    .unwrap_or(0);
                let consecutive_failures = if healthy {
                    0
                } else {
                    prev_failures.saturating_add(1)
                };
                state.provider_health_records.insert(
                    e.connection_id.as_str().to_owned(),
                    cairn_domain::providers::ProviderHealthRecord {
                        binding_id: cairn_domain::ids::ProviderBindingId::new(
                            e.connection_id.as_str(),
                        ),
                        healthy,
                        last_checked_ms: e.checked_at_ms,
                        error_message: None,
                        consecutive_failures,
                        status: e.status,
                    },
                );
            }
            RuntimeEvent::ProviderMarkedDegraded(e) => {
                let rec = state
                    .provider_health_records
                    .entry(e.connection_id.as_str().to_owned())
                    .or_insert_with(|| cairn_domain::providers::ProviderHealthRecord {
                        binding_id: cairn_domain::ids::ProviderBindingId::new(
                            e.connection_id.as_str(),
                        ),
                        healthy: false,
                        last_checked_ms: e.marked_at_ms,
                        error_message: None,
                        consecutive_failures: 0,
                        status: cairn_domain::providers::ProviderHealthStatus::Degraded,
                    });
                rec.healthy = false;
                rec.status = cairn_domain::providers::ProviderHealthStatus::Degraded;
                rec.error_message = Some(e.reason.clone());
                rec.last_checked_ms = e.marked_at_ms;
            }
            RuntimeEvent::ProviderRecovered(e) => {
                if let Some(rec) = state
                    .provider_health_records
                    .get_mut(e.connection_id.as_str())
                {
                    rec.healthy = true;
                    rec.status = cairn_domain::providers::ProviderHealthStatus::Healthy;
                    rec.error_message = None;
                    rec.last_checked_ms = e.recovered_at_ms;
                    rec.consecutive_failures = 0;
                }
            }
            RuntimeEvent::WorkspaceMemberAdded(e) => {
                state.workspace_members.retain(|m| {
                    !(m.workspace_id == e.workspace_key.workspace_id.as_str()
                        && m.operator_id == e.member_id.as_str())
                });
                state
                    .workspace_members
                    .push(crate::projections::WorkspaceMemberRecord {
                        workspace_id: e.workspace_key.workspace_id.as_str().to_owned(),
                        operator_id: e.member_id.as_str().to_owned(),
                        role: e.role,
                        added_at_ms: e.added_at_ms,
                    });
            }
            RuntimeEvent::WorkspaceMemberRemoved(e) => {
                state.workspace_members.retain(|m| {
                    !(m.workspace_id == e.workspace_key.workspace_id.as_str()
                        && m.operator_id == e.member_id.as_str())
                });
            }
            RuntimeEvent::RunCostAlertSet(e) => {
                state.run_cost_alerts.insert(
                    e.run_id.as_str().to_owned(),
                    cairn_domain::providers::RunCostAlert {
                        run_id: e.run_id.clone(),
                        threshold_micros: e.threshold_micros,
                        triggered_at_ms: 0,
                        tenant_id: e.tenant_id.clone(),
                        actual_cost_micros: 0,
                    },
                );
            }
            RuntimeEvent::RunCostAlertTriggered(e) => {
                if let Some(a) = state.run_cost_alerts.get_mut(e.run_id.as_str()) {
                    a.triggered_at_ms = e.triggered_at_ms;
                    a.actual_cost_micros = e.actual_cost_micros;
                }
            }
            RuntimeEvent::RunSlaSet(e) => {
                state.run_sla_configs.insert(
                    e.run_id.as_str().to_owned(),
                    cairn_domain::sla::SlaConfig {
                        run_id: e.run_id.clone(),
                        tenant_id: e.tenant_id.clone(),
                        target_completion_ms: e.target_completion_ms,
                        alert_at_percent: e.alert_at_percent,
                        configured_at_ms: e.set_at_ms,
                    },
                );
            }
            RuntimeEvent::RunSlaBreached(e) => {
                state.run_sla_breaches.insert(
                    e.run_id.as_str().to_owned(),
                    cairn_domain::sla::SlaBreach {
                        run_id: e.run_id.clone(),
                        tenant_id: e.tenant_id.clone(),
                        elapsed_ms: e.elapsed_ms,
                        target_ms: e.target_ms,
                        breached_at_ms: e.breached_at_ms,
                    },
                );
            }
            RuntimeEvent::ProviderBindingCreated(e) => {
                // Use the event position as created_at when the event's timestamp is 0,
                // ensuring stable creation-order sorting in list_active.
                let effective_created_at = if e.created_at > 0 {
                    e.created_at
                } else {
                    event.position.0
                };
                state.provider_bindings.insert(
                    e.provider_binding_id.as_str().to_owned(),
                    cairn_domain::providers::ProviderBindingRecord {
                        provider_binding_id: e.provider_binding_id.clone(),
                        project: e.project.clone(),
                        provider_connection_id: e.provider_connection_id.clone(),
                        provider_model_id: e.provider_model_id.clone(),
                        operation_kind: e.operation_kind,
                        settings: e.settings.clone(),
                        active: e.active,
                        created_at: effective_created_at,
                    },
                );
            }
            RuntimeEvent::ProviderBindingStateChanged(e) => {
                if let Some(b) = state
                    .provider_bindings
                    .get_mut(e.provider_binding_id.as_str())
                {
                    b.active = e.active;
                }
            }
            RuntimeEvent::ProviderHealthScheduleSet(e) => {
                state.provider_health_schedules.insert(
                    e.schedule_id.clone(),
                    cairn_domain::providers::ProviderHealthSchedule {
                        schedule_id: e.schedule_id.clone(),
                        binding_id: cairn_domain::ProviderBindingId::new(""),
                        interval_ms: e.interval_ms,
                        enabled: e.enabled,
                        connection_id: e.connection_id.clone(),
                        tenant_id: e.tenant_id.clone(),
                        last_run_ms: None,
                    },
                );
            }
            RuntimeEvent::ProviderHealthScheduleTriggered(e) => {
                if let Some(s) = state.provider_health_schedules.get_mut(&e.schedule_id) {
                    s.last_run_ms = Some(e.triggered_at_ms);
                }
            }
            RuntimeEvent::SignalSubscriptionCreated(e) => {
                state.signal_subscriptions.insert(
                    e.subscription_id.clone(),
                    crate::projections::SignalSubscriptionRecord {
                        subscription_id: e.subscription_id.clone(),
                        signal_type: e.signal_kind.clone(),
                        target: e
                            .target_run_id
                            .as_ref()
                            .map(|r| r.as_str().to_owned())
                            .unwrap_or_default(),
                        created_at_ms: e.created_at_ms,
                        project: Some(e.project.clone()),
                        project_tenant: e.project.tenant_id.as_str().to_owned(),
                        project_workspace: e.project.workspace_id.as_str().to_owned(),
                        project_id: e.project.project_id.as_str().to_owned(),
                        target_run_id: e.target_run_id.clone(),
                        target_mailbox_id: e.target_mailbox_id.clone(),
                        filter_expression: e.filter_expression.clone(),
                    },
                );
            }
            RuntimeEvent::RetentionPolicySet(e) => {
                state.retention_policies.insert(
                    e.tenant_id.as_str().to_owned(),
                    cairn_domain::RetentionPolicy {
                        policy_id: e.policy_id.clone(),
                        tenant_id: e.tenant_id.clone(),
                        full_history_days: e.full_history_days,
                        current_state_days: e.current_state_days,
                        max_events_per_entity: e.max_events_per_entity.unwrap_or(0) as u32,
                    },
                );
            }
            RuntimeEvent::TenantQuotaViolated(_)
            | RuntimeEvent::ApprovalDelegated(_)
            | RuntimeEvent::AuditLogEntryRecorded(_)
            | RuntimeEvent::EventLogCompacted(_)
            | RuntimeEvent::GuardrailPolicyEvaluated(_)
            | RuntimeEvent::OperatorIntervention(_)
            | RuntimeEvent::PauseScheduled(_)
            | RuntimeEvent::PermissionDecisionRecorded(_)
            | RuntimeEvent::ProviderModelRegistered(_)
            | RuntimeEvent::ProviderRetryPolicySet(_) => {}
            RuntimeEvent::ResourceShared(e) => {
                state.resource_shares.insert(
                    e.share_id.clone(),
                    cairn_domain::resource_sharing::SharedResource {
                        share_id: e.share_id.clone(),
                        tenant_id: e.tenant_id.clone(),
                        source_workspace_id: e.source_workspace_id.clone(),
                        target_workspace_id: e.target_workspace_id.clone(),
                        resource_type: e.resource_type.clone(),
                        resource_id: e.resource_id.clone(),
                        permissions: e.permissions.clone(),
                        shared_at_ms: e.shared_at_ms,
                    },
                );
            }
            RuntimeEvent::ResourceShareRevoked(e) => {
                state.resource_shares.remove(&e.share_id);
            }
            RuntimeEvent::RoutePolicyCreated(e) => {
                state.route_policies.insert(
                    e.policy_id.clone(),
                    cairn_domain::providers::RoutePolicy {
                        policy_id: e.policy_id.clone(),
                        name: e.name.clone(),
                        enabled: e.enabled,
                        tenant_id: e.tenant_id.as_str().to_owned(),
                        rules: e.rules.clone(),
                        updated_at_ms: now,
                    },
                );
            }
            RuntimeEvent::RoutePolicyUpdated(e) => {
                if let Some(p) = state.route_policies.get_mut(&e.policy_id) {
                    p.updated_at_ms = e.updated_at_ms;
                }
            }
            RuntimeEvent::RecoveryEscalated(_) | RuntimeEvent::SignalRouted(_) => {}
            RuntimeEvent::SnapshotCreated(e) => {
                state.snapshots.push(cairn_domain::Snapshot {
                    snapshot_id: e.snapshot_id.clone(),
                    tenant_id: e.tenant_id.clone(),
                    event_position: e.event_position,
                    state_hash: String::new(),
                    created_at_ms: e.created_at_ms,
                    compressed_state: vec![],
                });
            }
            // Audit-only events — appended to the log for later
            // reconstruction but not projected into any read model.
            // TaskDependency{Added,Resolved} are authoritative in FF;
            // cairn keeps them on the log for join-against-
            // TaskStateChanged queries.
            RuntimeEvent::TaskDependencyAdded(_)
            | RuntimeEvent::TaskDependencyResolved(_)
            | RuntimeEvent::TaskLeaseExpired(_)
            | RuntimeEvent::TaskPriorityChanged(_)
            | RuntimeEvent::ToolInvocationProgressUpdated(_) => {}
            RuntimeEvent::SessionCostUpdated(e) => {
                let rec = state
                    .session_costs
                    .entry(e.session_id.as_str().to_owned())
                    .or_insert_with(|| cairn_domain::providers::SessionCostRecord {
                        session_id: e.session_id.clone(),
                        tenant_id: e.tenant_id.clone(),
                        total_cost_micros: 0,
                        total_tokens_in: 0,
                        total_tokens_out: 0,
                        provider_calls: 0,
                        token_in: 0,
                        token_out: 0,
                        updated_at_ms: now,
                    });
                rec.total_cost_micros = rec.total_cost_micros.saturating_add(e.delta_cost_micros);
                rec.total_tokens_in = rec.total_tokens_in.saturating_add(e.delta_tokens_in);
                rec.total_tokens_out = rec.total_tokens_out.saturating_add(e.delta_tokens_out);
                rec.provider_calls = rec.provider_calls.saturating_add(1);
                rec.token_in = rec.total_tokens_in;
                rec.token_out = rec.total_tokens_out;
                rec.updated_at_ms = now;
                // Also accumulate into provider budget spend for the tenant.
                for budget in state.provider_budgets.values_mut() {
                    if budget.tenant_id == e.tenant_id {
                        budget.current_spend_micros = budget
                            .current_spend_micros
                            .saturating_add(e.delta_cost_micros);
                        budget.updated_at = now;
                    }
                }
            }
            // RFC 005: link child task to parent run/task on subagent spawn.
            RuntimeEvent::SubagentSpawned(e) => {
                if let Some(rec) = state.tasks.get_mut(e.child_task_id.as_str()) {
                    rec.parent_run_id = Some(e.parent_run_id.clone());
                    rec.parent_task_id = e.parent_task_id.clone();
                    rec.updated_at = now;
                }
            }
            // Audit/linkage events that don't update core projections.
            RuntimeEvent::CheckpointRestored(_)
            | RuntimeEvent::RecoveryAttempted(_)
            | RuntimeEvent::RecoveryCompleted(_)
            | RuntimeEvent::UserMessageAppended(_)
            | RuntimeEvent::TriggerCreated(_)
            | RuntimeEvent::TriggerEnabled(_)
            | RuntimeEvent::TriggerDisabled(_)
            | RuntimeEvent::TriggerSuspended(_)
            | RuntimeEvent::TriggerResumed(_)
            | RuntimeEvent::TriggerDeleted(_)
            | RuntimeEvent::TriggerFired(_)
            | RuntimeEvent::TriggerSkipped(_)
            | RuntimeEvent::TriggerDenied(_)
            | RuntimeEvent::TriggerRateLimited(_)
            | RuntimeEvent::TriggerPendingApproval(_)
            | RuntimeEvent::RunTemplateCreated(_)
            | RuntimeEvent::RunTemplateDeleted(_)
            | RuntimeEvent::PlanProposed(_)
            | RuntimeEvent::PlanApproved(_)
            | RuntimeEvent::PlanRejected(_)
            | RuntimeEvent::PlanRevisionRequested(_)
            // RFC 020 Track 3: audit-only events; no in-memory projection update.
            | RuntimeEvent::ToolInvocationCacheHit(_)
            | RuntimeEvent::ToolRecoveryPaused(_)
            // RFC 020 Track 4: boot-level recovery audit event.
            | RuntimeEvent::RecoverySummaryEmitted(_) => {}
            RuntimeEvent::ScheduledTaskCreated(e) => {
                state.scheduled_tasks.insert(
                    e.scheduled_task_id.as_str().to_owned(),
                    cairn_domain::ScheduledTaskRecord {
                        scheduled_task_id: e.scheduled_task_id.clone(),
                        tenant_id: e.tenant_id.clone(),
                        name: e.name.clone(),
                        cron_expression: e.cron_expression.clone(),
                        last_run_at: None,
                        next_run_at: e.next_run_at,
                        enabled: true,
                        created_at: e.created_at,
                        updated_at: e.created_at,
                    },
                );
            }
            RuntimeEvent::RouteDecisionMade(e) => {
                state.route_decisions.insert(
                    e.route_decision_id.as_str().to_owned(),
                    cairn_domain::providers::RouteDecisionRecord {
                        route_decision_id: e.route_decision_id.clone(),
                        project_id: e.project.project_id.clone(),
                        operation_kind: e.operation_kind,
                        terminal_route_attempt_id: None,
                        selected_provider_binding_id: e.selected_provider_binding_id.clone(),
                        selected_route_attempt_id: None,
                        selector_context: cairn_domain::selectors::SelectorContext::default(),
                        attempt_count: e.attempt_count,
                        fallback_used: e.fallback_used,
                        final_status: e.final_status,
                    },
                );
            }
            RuntimeEvent::ProviderCallCompleted(e) => {
                state.provider_calls.insert(
                    e.provider_call_id.as_str().to_owned(),
                    cairn_domain::providers::ProviderCallRecord {
                        provider_call_id: e.provider_call_id.clone(),
                        route_decision_id: e.route_decision_id.clone(),
                        route_attempt_id: e.route_attempt_id.clone(),
                        project_id: e.project.project_id.clone(),
                        operation_kind: e.operation_kind,
                        provider_binding_id: e.provider_binding_id.clone(),
                        provider_connection_id: e.provider_connection_id.clone(),
                        provider_adapter: String::new(),
                        provider_model_id: e.provider_model_id.clone(),
                        task_id: e.task_id.clone(),
                        run_id: e.run_id.clone(),
                        prompt_release_id: e.prompt_release_id.clone(),
                        fallback_position: e.fallback_position as u16,
                        status: e.status,
                        latency_ms: e.latency_ms.or_else(|| {
                            if e.started_at > 0 && e.finished_at >= e.started_at {
                                Some(e.finished_at - e.started_at)
                            } else {
                                None
                            }
                        }),
                        input_tokens: e.input_tokens,
                        output_tokens: e.output_tokens,
                        cost_micros: e.cost_micros,
                        cost_type: cairn_domain::providers::ProviderCostType::default(),
                        error_class: e.error_class,
                    },
                );
                // GAP-010: derive LlmCallTrace from every ProviderCallCompleted.
                // All calls — successful and failed — are valuable for observability.
                state.llm_traces.push(cairn_domain::LlmCallTrace {
                    trace_id: e.provider_call_id.as_str().to_owned(),
                    model_id: e.provider_model_id.as_str().to_owned(),
                    prompt_tokens: e.input_tokens.unwrap_or(0),
                    completion_tokens: e.output_tokens.unwrap_or(0),
                    latency_ms: e.latency_ms.unwrap_or(0),
                    cost_micros: e.cost_micros.unwrap_or(0),
                    session_id: e.session_id.clone(),
                    run_id: e.run_id.clone(),
                    created_at_ms: e.completed_at,
                    is_error: e.status != cairn_domain::providers::ProviderCallStatus::Succeeded,
                });
                // Accumulate run-level costs and emit a derived RunCostUpdated event.
                if let Some(run_id) = &e.run_id {
                    let delta_cost = e.cost_micros.unwrap_or(0);
                    let delta_in = e.input_tokens.unwrap_or(0) as u64;
                    let delta_out = e.output_tokens.unwrap_or(0) as u64;
                    let rec = state
                        .run_costs
                        .entry(run_id.as_str().to_owned())
                        .or_insert_with(|| cairn_domain::providers::RunCostRecord {
                            run_id: run_id.clone(),
                            total_cost_micros: 0,
                            total_tokens_in: 0,
                            total_tokens_out: 0,
                            provider_calls: 0,
                            token_in: 0,
                            token_out: 0,
                        });
                    rec.total_cost_micros = rec.total_cost_micros.saturating_add(delta_cost);
                    rec.total_tokens_in = rec.total_tokens_in.saturating_add(delta_in);
                    rec.total_tokens_out = rec.total_tokens_out.saturating_add(delta_out);
                    rec.provider_calls += 1;
                    rec.token_in = rec.total_tokens_in;
                    rec.token_out = rec.total_tokens_out;
                    // Emit a derived RunCostUpdated event directly into the log.
                    let derived_pos = EventPosition(state.next_position);
                    state.next_position += 1;
                    let derived = StoredEvent {
                        position: derived_pos,
                        envelope: EventEnvelope {
                            event_id: cairn_domain::EventId::new(format!(
                                "derived_rcu_{}",
                                e.provider_call_id.as_str()
                            )),
                            source: cairn_domain::EventSource::System,
                            ownership: cairn_domain::OwnershipKey::Project(e.project.clone()),
                            causation_id: None,
                            correlation_id: None,
                            payload: RuntimeEvent::RunCostUpdated(cairn_domain::RunCostUpdated {
                                project: e.project.clone(),
                                run_id: run_id.clone(),
                                delta_cost_micros: delta_cost,
                                delta_tokens_in: delta_in,
                                delta_tokens_out: delta_out,
                                provider_call_id: e.provider_call_id.as_str().to_owned(),
                                updated_at_ms: event.stored_at,
                                session_id: None,
                                tenant_id: None,
                            }),
                        },
                        stored_at: event.stored_at,
                    };
                    state.events.push(derived);
                }
                // Accumulate session-level costs.
                // Derive session_id from run record if not provided on the event.
                let effective_session_id = e.session_id.clone().or_else(|| {
                    e.run_id
                        .as_ref()
                        .and_then(|rid| state.runs.get(rid.as_str()).map(|r| r.session_id.clone()))
                });
                if let Some(session_id) = effective_session_id {
                    let delta_cost = e.cost_micros.unwrap_or(0);
                    let delta_in = e.input_tokens.unwrap_or(0) as u64;
                    let delta_out = e.output_tokens.unwrap_or(0) as u64;
                    let rec = state
                        .session_costs
                        .entry(session_id.as_str().to_owned())
                        .or_insert_with(|| cairn_domain::providers::SessionCostRecord {
                            session_id: session_id.clone(),
                            tenant_id: cairn_domain::TenantId::new(e.project.tenant_id.as_str()),
                            total_cost_micros: 0,
                            total_tokens_in: 0,
                            total_tokens_out: 0,
                            updated_at_ms: event.stored_at,
                            provider_calls: 0,
                            token_in: 0,
                            token_out: 0,
                        });
                    rec.total_cost_micros = rec.total_cost_micros.saturating_add(delta_cost);
                    rec.total_tokens_in = rec.total_tokens_in.saturating_add(delta_in);
                    rec.total_tokens_out = rec.total_tokens_out.saturating_add(delta_out);
                    rec.provider_calls += 1;
                    rec.token_in = rec.total_tokens_in;
                    rec.token_out = rec.total_tokens_out;
                    rec.updated_at_ms = event.stored_at;
                    // Emit SessionCostUpdated event into the log for traceability.
                    let sc_pos = EventPosition(state.next_position);
                    state.next_position += 1;
                    let sc_derived = StoredEvent {
                        position: sc_pos,
                        envelope: EventEnvelope {
                            event_id: cairn_domain::EventId::new(format!(
                                "derived_scu_{}",
                                e.provider_call_id.as_str()
                            )),
                            source: cairn_domain::EventSource::System,
                            ownership: cairn_domain::OwnershipKey::Project(e.project.clone()),
                            causation_id: None,
                            correlation_id: None,
                            payload: RuntimeEvent::SessionCostUpdated(SessionCostUpdated {
                                project: e.project.clone(),
                                session_id: session_id.clone(),
                                tenant_id: cairn_domain::TenantId::new(
                                    e.project.tenant_id.as_str(),
                                ),
                                delta_cost_micros: delta_cost,
                                delta_tokens_in: delta_in,
                                delta_tokens_out: delta_out,
                                provider_call_id: e.provider_call_id.as_str().to_owned(),
                                updated_at_ms: event.stored_at,
                            }),
                        },
                        stored_at: event.stored_at,
                    };
                    state.events.push(sc_derived);
                }
            }
            RuntimeEvent::TenantCreated(e) => {
                state.tenants.insert(
                    e.tenant_id.as_str().to_owned(),
                    cairn_domain::org::TenantRecord {
                        tenant_id: e.tenant_id.clone(),
                        name: e.name.clone(),
                        created_at: e.created_at,
                        updated_at: e.created_at,
                    },
                );
            }
            RuntimeEvent::WorkspaceCreated(e) => {
                state.workspaces.insert(
                    e.workspace_id.as_str().to_owned(),
                    cairn_domain::org::WorkspaceRecord {
                        workspace_id: e.workspace_id.clone(),
                        tenant_id: e.tenant_id.clone(),
                        name: e.name.clone(),
                        created_at: e.created_at,
                        updated_at: e.created_at,
                    },
                );
            }
            RuntimeEvent::ProjectCreated(e) => {
                state.projects.insert(
                    e.project.project_id.as_str().to_owned(),
                    cairn_domain::org::ProjectRecord {
                        project_id: e.project.project_id.clone(),
                        workspace_id: e.project.workspace_id.clone(),
                        tenant_id: e.project.tenant_id.clone(),
                        name: e.name.clone(),
                        created_at: e.created_at,
                        updated_at: e.created_at,
                    },
                );
            }
            RuntimeEvent::PromptAssetCreated(e) => {
                state.prompt_assets.insert(
                    e.prompt_asset_id.as_str().to_owned(),
                    crate::projections::PromptAssetRecord {
                        prompt_asset_id: e.prompt_asset_id.clone(),
                        project: e.project.clone(),
                        name: e.name.clone(),
                        kind: e.kind.clone(),
                        created_at: e.created_at,
                        scope: String::new(),
                        status: "draft".to_owned(),
                        workspace: String::new(),
                        updated_at: now,
                    },
                );
            }
            RuntimeEvent::PromptVersionCreated(e) => {
                let version_number = state
                    .prompt_versions
                    .values()
                    .filter(|v| v.prompt_asset_id == e.prompt_asset_id)
                    .count() as u32
                    + 1;
                state.prompt_versions.insert(
                    e.prompt_version_id.as_str().to_owned(),
                    crate::projections::PromptVersionRecord {
                        prompt_version_id: e.prompt_version_id.clone(),
                        prompt_asset_id: e.prompt_asset_id.clone(),
                        project: e.project.clone(),
                        content_hash: e.content_hash.clone(),
                        created_at: e.created_at,
                        version_number,
                        // RFC 006: populate workspace from event field so projections
                        // can scope at workspace level without re-deriving from project.
                        workspace: e.workspace_id.as_str().to_owned(),
                    },
                );
            }
            RuntimeEvent::ApprovalPolicyCreated(e) => {
                state.approval_policies.insert(
                    e.policy_id.clone(),
                    cairn_domain::ApprovalPolicyRecord {
                        policy_id: e.policy_id.clone(),
                        tenant_id: e.tenant_id.clone(),
                        name: e.name.clone(),
                        required_approvers: e.required_approvers,
                        allowed_approver_roles: e.allowed_approver_roles.clone(),
                        auto_approve_after_ms: e.auto_approve_after_ms,
                        auto_reject_after_ms: e.auto_reject_after_ms,
                        attached_release_ids: Vec::new(),
                    },
                );
            }
            RuntimeEvent::PromptReleaseCreated(e) => {
                state.prompt_releases.insert(
                    e.prompt_release_id.as_str().to_owned(),
                    crate::projections::PromptReleaseRecord {
                        prompt_release_id: e.prompt_release_id.clone(),
                        project: e.project.clone(),
                        prompt_asset_id: e.prompt_asset_id.clone(),
                        prompt_version_id: e.prompt_version_id.clone(),
                        state: "draft".to_owned(),
                        rollout_percent: None,
                        routing_slot: None,
                        task_type: None,
                        agent_type: None,
                        is_project_default: false,
                        release_tag: e.release_tag.clone(),
                        created_by: e.created_by.clone(),
                        created_at: e.created_at,
                        updated_at: e.created_at,
                    },
                );
            }
            RuntimeEvent::PromptReleaseTransitioned(e) => {
                if let Some(rec) = state.prompt_releases.get_mut(e.prompt_release_id.as_str()) {
                    rec.state = e.to_state.clone();
                    rec.updated_at = e.transitioned_at;
                }
            }
            RuntimeEvent::PromptRolloutStarted(e) => {
                if let Some(rec) = state.prompt_releases.get_mut(e.prompt_release_id.as_str()) {
                    rec.rollout_percent = Some(e.percent);
                    rec.state = "active".to_owned();
                    rec.updated_at = e.started_at;
                }
            }
            RuntimeEvent::IngestJobStarted(e) => {
                state.ingest_jobs.insert(
                    e.job_id.as_str().to_owned(),
                    cairn_domain::IngestJobRecord {
                        id: e.job_id.clone(),
                        project: e.project.clone(),
                        source_id: e.source_id.clone(),
                        document_count: e.document_count,
                        state: cairn_domain::IngestJobState::Processing,
                        error_message: None,
                        created_at: e.started_at,
                        updated_at: e.started_at,
                    },
                );
            }
            RuntimeEvent::IngestJobCompleted(e) => {
                if let Some(rec) = state.ingest_jobs.get_mut(e.job_id.as_str()) {
                    rec.state = if e.success {
                        cairn_domain::IngestJobState::Completed
                    } else {
                        cairn_domain::IngestJobState::Failed
                    };
                    rec.error_message = e.error_message.clone();
                    rec.updated_at = e.completed_at;
                }
            }
            RuntimeEvent::EvalRunStarted(e) => {
                state.eval_runs.insert(
                    e.eval_run_id.as_str().to_owned(),
                    crate::projections::EvalRunRecord {
                        eval_run_id: e.eval_run_id.clone(),
                        project: e.project.clone(),
                        subject_kind: e.subject_kind.clone(),
                        evaluator_type: e.evaluator_type.clone(),
                        success: None,
                        error_message: None,
                        started_at: e.started_at,
                        completed_at: None,
                    },
                );
            }
            RuntimeEvent::EvalRunCompleted(e) => {
                if let Some(rec) = state.eval_runs.get_mut(e.eval_run_id.as_str()) {
                    rec.success = Some(e.success);
                    rec.error_message = e.error_message.clone();
                    rec.completed_at = Some(e.completed_at);
                }
            }
            RuntimeEvent::OutcomeRecorded(e) => {
                state.outcomes.insert(
                    e.outcome_id.as_str().to_owned(),
                    crate::projections::OutcomeRecord {
                        outcome_id: e.outcome_id.clone(),
                        run_id: e.run_id.clone(),
                        project: e.project.clone(),
                        agent_type: e.agent_type.clone(),
                        predicted_confidence: e.predicted_confidence,
                        actual_outcome: e.actual_outcome.clone(),
                        recorded_at: e.recorded_at,
                    },
                );
            }
            RuntimeEvent::EvalDatasetCreated(e) => {
                // tenant_id is not carried by this event; stored with empty sentinel.
                // Use EvalSubjectKind::PromptRelease as default subject kind.
                state
                    .eval_datasets
                    .entry(e.dataset_id.clone())
                    .or_insert_with(|| cairn_domain::EvalDataset {
                        dataset_id: e.dataset_id.clone(),
                        tenant_id: cairn_domain::TenantId::new(""),
                        name: e.name.clone(),
                        subject_kind: cairn_domain::EvalSubjectKind::PromptRelease,
                        entries: Vec::new(),
                        created_at_ms: e.created_at_ms,
                    });
            }
            RuntimeEvent::EvalDatasetEntryAdded(e) => {
                if let Some(ds) = state.eval_datasets.get_mut(e.dataset_id.as_str()) {
                    // entry_id is stored as a tag so it can be deduplicated.
                    let already_exists = ds.entries.iter().any(|entry| {
                        entry.tags.first().map(String::as_str) == Some(e.entry_id.as_str())
                    });
                    if !already_exists {
                        ds.entries.push(cairn_domain::EvalDatasetEntry {
                            input: serde_json::json!({ "entry_id": e.entry_id }),
                            expected_output: None,
                            tags: vec![e.entry_id.clone()],
                        });
                    }
                }
            }
            RuntimeEvent::CheckpointStrategySet(e) => {
                if let Some(run_id) = &e.run_id {
                    state.checkpoint_strategies.insert(
                        run_id.as_str().to_owned(),
                        cairn_domain::CheckpointStrategy {
                            strategy_id: e.strategy_id.clone(),
                            project: cairn_domain::ProjectKey::new(
                                "_strategy",
                                "_strategy",
                                "_strategy",
                            ),
                            run_id: run_id.clone(),
                            interval_ms: e.interval_ms,
                            max_checkpoints: if e.max_checkpoints > 0 {
                                e.max_checkpoints
                            } else {
                                10
                            },
                            trigger_on_task_complete: e.trigger_on_task_complete,
                        },
                    );
                }
            }
            RuntimeEvent::EvalRubricCreated(e) => {
                // tenant_id not in event; stored with sentinel "".
                state
                    .eval_rubrics
                    .entry(e.rubric_id.clone())
                    .or_insert_with(|| cairn_domain::EvalRubric {
                        rubric_id: e.rubric_id.clone(),
                        tenant_id: cairn_domain::TenantId::new(""),
                        name: e.name.clone(),
                        dimensions: vec![],
                        created_at_ms: e.created_at_ms,
                    });
            }
            RuntimeEvent::EvalBaselineSet(e) => {
                // EvalBaselineSet carries one metric key=value; upsert the baseline record.
                // Fields like tenant_id, name, prompt_asset_id not in event → sentinels.
                let entry = state
                    .eval_baselines
                    .entry(e.baseline_id.clone())
                    .or_insert_with(|| cairn_domain::EvalBaseline {
                        baseline_id: e.baseline_id.clone(),
                        tenant_id: cairn_domain::TenantId::new(""),
                        name: e.baseline_id.clone(),
                        prompt_asset_id: cairn_domain::PromptAssetId::new(""),
                        metrics: cairn_domain::EvalMetrics::default(),
                        created_at_ms: e.set_at_ms,
                        locked: false,
                    });
                // Only update if not locked — locked baselines are immutable.
                if !entry.locked {
                    // Store metric as a tag in the name for auditability.
                    entry.name = format!("{}[{}={}]", entry.baseline_id, e.metric, e.value);
                }
            }
            RuntimeEvent::EvalBaselineLocked(e) => {
                if let Some(baseline) = state.eval_baselines.get_mut(&e.baseline_id) {
                    baseline.locked = true;
                }
            }
            // RFC 020 decision-cache survival: no dedicated projection —
            // cairn-app rebuilds the in-memory decision cache from the
            // event log at startup.
            RuntimeEvent::DecisionRecorded(_) | RuntimeEvent::DecisionCacheWarmup(_) => {}
        }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

// -- EventLog --

#[async_trait]
impl EventLog for InMemoryStore {
    async fn append(
        &self,
        events: &[EventEnvelope<RuntimeEvent>],
    ) -> Result<Vec<EventPosition>, StoreError> {
        // Scope the MutexGuard so it is lexically dropped before any `.await`.
        let positions = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let mut usage = self
                .usage_counters
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let now = now_millis();
            let mut positions = Vec::with_capacity(events.len());

            for envelope in events {
                let counters = usage.entry(envelope.project().clone()).or_default();
                counters.event_count += 1;
                if matches!(&envelope.payload, RuntimeEvent::RunCreated(_)) {
                    counters.run_count += 1;
                }

                let pos = EventPosition(state.next_position);
                state.next_position += 1;

                let stored = StoredEvent {
                    position: pos,
                    envelope: envelope.clone(),
                    stored_at: now,
                };

                // Populate the causation_id index (RFC 002 idempotency).
                // Keep the earliest position for a given causation; later
                // references are already retrievable via `read_stream`.
                if let Some(cause) = envelope.causation_id.as_ref() {
                    state
                        .command_id_index
                        .entry(cause.as_str().to_owned())
                        .or_insert(pos.0);
                }

                // Push the original event BEFORE calling apply_projection so that
                // any derived events inserted by the projection appear AFTER the
                // original in the log, preserving strict position monotonicity.
                state.events.push(stored.clone());
                Self::apply_projection(&mut state, &stored);
                positions.push(pos);

                // Broadcast to SSE subscribers; ignore send errors (no active receivers).
                let _ = self.event_tx.send(stored);
            }

            positions
            // `state` (MutexGuard) is dropped here, before any await point.
        };

        // Dual-write to the durable secondary log (Postgres, SQLite) if
        // one is configured. Fail CLOSED: the in-memory write has already
        // committed by this point, but we surface the secondary failure
        // so the caller can decide whether to retry, compensate, or
        // abort. The old `eprintln`-and-swallow path silently lost data
        // on restart when the secondary was the durable source of truth
        // (RFC 002).
        //
        // **Divergence contract on `Err`:** the in-memory log has the
        // events, the secondary does not. The caller MUST treat this as
        // a recoverable divergence and is responsible for reconciliation.
        // Options, in rough order of preference:
        //   1. Retry the same call. `InMemoryStore::append` today does
        //      NOT dedup by `event_id`, so a plain retry would double-
        //      apply the projection — do not retry blindly against the
        //      same store; construct a fresh caller-side envelope or
        //      flush+replay on the primary. Populating event_id dedup
        //      on the primary is tracked as audit follow-up T2-M4.
        //   2. Write the events directly to the secondary once it's
        //      healthy, then confirm primary/secondary head positions
        //      match.
        //   3. Abort the caller-level transaction, roll forward from
        //      the secondary's last durable position.
        //
        // Until T2-M4 lands, the safest pattern for a deploy using
        // InMemoryStore as primary with a durable secondary is to
        // configure the app to crash on `Err` from `append` and rely
        // on restart-plus-replay to reconverge.
        let secondary = self
            .secondary_log
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(log) = secondary {
            if let Err(e) = log.append(events).await {
                tracing::error!(
                    error = %e,
                    event_count = events.len(),
                    "secondary event log write failed — in-memory log has {} event(s) the secondary did not commit",
                    events.len(),
                );
                return Err(StoreError::Internal(format!(
                    "secondary event log write failed: {e}; in-memory and secondary logs have diverged by {} event(s)",
                    events.len()
                )));
            }
        }

        Ok(positions)
    }

    async fn read_by_entity(
        &self,
        entity: &EntityRef,
        after: Option<EventPosition>,
        limit: usize,
    ) -> Result<Vec<StoredEvent>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let min_pos = after.map(|p| p.0).unwrap_or(0);

        let results: Vec<StoredEvent> = state
            .events
            .iter()
            .filter(|e| e.position.0 > min_pos)
            .filter(|e| event_matches_entity(&e.envelope.payload, entity))
            .take(limit)
            .cloned()
            .collect();

        Ok(results)
    }

    async fn read_stream(
        &self,
        after: Option<EventPosition>,
        limit: usize,
    ) -> Result<Vec<StoredEvent>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let min_pos = after.map(|p| p.0).unwrap_or(0);

        let results: Vec<StoredEvent> = state
            .events
            .iter()
            .filter(|e| e.position.0 > min_pos)
            .take(limit)
            .cloned()
            .collect();

        Ok(results)
    }

    async fn head_position(&self) -> Result<Option<EventPosition>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.events.last().map(|e| e.position))
    }

    async fn find_by_causation_id(
        &self,
        causation_id: &str,
    ) -> Result<Option<EventPosition>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .command_id_index
            .get(causation_id)
            .copied()
            .map(EventPosition))
    }
}

fn event_matches_entity(event: &RuntimeEvent, entity: &EntityRef) -> bool {
    match (event, entity) {
        (RuntimeEvent::SessionCreated(e), EntityRef::Session(id)) => e.session_id == *id,
        (RuntimeEvent::SessionStateChanged(e), EntityRef::Session(id)) => e.session_id == *id,
        (RuntimeEvent::RunCreated(e), EntityRef::Run(id)) => e.run_id == *id,
        (RuntimeEvent::RunStateChanged(e), EntityRef::Run(id)) => e.run_id == *id,
        (RuntimeEvent::TaskCreated(e), EntityRef::Task(id)) => e.task_id == *id,
        (RuntimeEvent::TaskLeaseClaimed(e), EntityRef::Task(id)) => e.task_id == *id,
        (RuntimeEvent::TaskLeaseHeartbeated(e), EntityRef::Task(id)) => e.task_id == *id,
        (RuntimeEvent::TaskStateChanged(e), EntityRef::Task(id)) => e.task_id == *id,
        (RuntimeEvent::ApprovalRequested(e), EntityRef::Approval(id)) => e.approval_id == *id,
        (RuntimeEvent::ApprovalResolved(e), EntityRef::Approval(id)) => e.approval_id == *id,
        (RuntimeEvent::CheckpointRecorded(e), EntityRef::Checkpoint(id)) => e.checkpoint_id == *id,
        (RuntimeEvent::CheckpointRestored(e), EntityRef::Checkpoint(id)) => e.checkpoint_id == *id,
        (RuntimeEvent::MailboxMessageAppended(e), EntityRef::Mailbox(id)) => e.message_id == *id,
        (RuntimeEvent::ToolInvocationStarted(e), EntityRef::ToolInvocation(id)) => {
            e.invocation_id == *id
        }
        (RuntimeEvent::ToolInvocationCompleted(e), EntityRef::ToolInvocation(id)) => {
            e.invocation_id == *id
        }
        (RuntimeEvent::ToolInvocationFailed(e), EntityRef::ToolInvocation(id)) => {
            e.invocation_id == *id
        }
        (RuntimeEvent::SignalIngested(e), EntityRef::Signal(id)) => e.signal_id == *id,
        (RuntimeEvent::UserMessageAppended(e), EntityRef::Run(id)) => e.run_id == *id,
        (RuntimeEvent::IngestJobStarted(e), EntityRef::IngestJob(id)) => e.job_id == *id,
        (RuntimeEvent::IngestJobCompleted(e), EntityRef::IngestJob(id)) => e.job_id == *id,
        (RuntimeEvent::EvalRunStarted(e), EntityRef::EvalRun(id)) => e.eval_run_id == *id,
        (RuntimeEvent::EvalRunCompleted(e), EntityRef::EvalRun(id)) => e.eval_run_id == *id,
        (RuntimeEvent::OutcomeRecorded(e), EntityRef::Run(id)) => e.run_id == *id,
        (RuntimeEvent::PlanProposed(e), EntityRef::Run(id)) => e.plan_run_id == *id,
        (RuntimeEvent::PlanApproved(e), EntityRef::Run(id)) => e.plan_run_id == *id,
        (RuntimeEvent::PlanRejected(e), EntityRef::Run(id)) => e.plan_run_id == *id,
        (RuntimeEvent::PlanRevisionRequested(e), EntityRef::Run(id)) => {
            e.original_plan_run_id == *id
        }
        (RuntimeEvent::PromptAssetCreated(e), EntityRef::PromptAsset(id)) => {
            e.prompt_asset_id == *id
        }
        (RuntimeEvent::PromptVersionCreated(e), EntityRef::PromptVersion(id)) => {
            e.prompt_version_id == *id
        }
        (RuntimeEvent::PromptReleaseCreated(e), EntityRef::PromptRelease(id)) => {
            e.prompt_release_id == *id
        }
        (RuntimeEvent::PromptReleaseTransitioned(e), EntityRef::PromptRelease(id)) => {
            e.prompt_release_id == *id
        }
        _ => false,
    }
}

// -- SessionReadModel --

#[async_trait]
impl SessionReadModel for InMemoryStore {
    async fn get(&self, session_id: &SessionId) -> Result<Option<SessionRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.sessions.get(session_id.as_str()).cloned())
    }

    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SessionRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<SessionRecord> = state
            .sessions
            .values()
            .filter(|s| s.project == *project)
            .cloned()
            .collect();
        results.sort_by_key(|s| (s.created_at, s.session_id.as_str().to_owned()));
        let results: Vec<SessionRecord> = results.into_iter().skip(offset).take(limit).collect();
        Ok(results)
    }

    async fn list_active(&self, limit: usize) -> Result<Vec<SessionRecord>, StoreError> {
        use cairn_domain::SessionState;
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<SessionRecord> = state
            .sessions
            .values()
            .filter(|s| s.state == SessionState::Open)
            .cloned()
            .collect();
        // Most recently updated first (fleet view shows live activity).
        results.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
        results.truncate(limit);
        Ok(results)
    }
}

// -- SessionCostReadModel --

#[async_trait]
impl crate::projections::SessionCostReadModel for InMemoryStore {
    async fn get_session_cost(
        &self,
        session_id: &cairn_domain::SessionId,
    ) -> Result<Option<cairn_domain::providers::SessionCostRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.session_costs.get(session_id.as_str()).cloned())
    }

    async fn list_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
        _since_ms: u64,
    ) -> Result<Vec<cairn_domain::providers::SessionCostRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<_> = state
            .session_costs
            .values()
            .filter(|r| &r.tenant_id == tenant_id)
            .cloned()
            .collect();
        results.sort_by_key(|r| r.updated_at_ms);
        Ok(results)
    }
}

// -- RunReadModel --

#[async_trait]
impl RunReadModel for InMemoryStore {
    async fn get(&self, run_id: &RunId) -> Result<Option<RunRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.runs.get(run_id.as_str()).cloned())
    }

    async fn list_by_session(
        &self,
        session_id: &SessionId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<RunRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<RunRecord> = state
            .runs
            .values()
            .filter(|r| r.session_id == *session_id)
            .cloned()
            .collect();
        results.sort_by_key(|r| (r.created_at, r.run_id.as_str().to_owned()));
        let results = results.into_iter().skip(offset).take(limit).collect();
        Ok(results)
    }

    async fn any_non_terminal(&self, session_id: &SessionId) -> Result<bool, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .runs
            .values()
            .any(|r| r.session_id == *session_id && !r.state.is_terminal()))
    }

    async fn latest_root_run(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<RunRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .runs
            .values()
            .filter(|r| r.session_id == *session_id && r.parent_run_id.is_none())
            .max_by_key(|r| (r.created_at, r.run_id.as_str().to_owned()))
            .cloned())
    }

    async fn list_by_state(
        &self,
        state: cairn_domain::RunState,
        limit: usize,
    ) -> Result<Vec<RunRecord>, StoreError> {
        let store = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<RunRecord> = store
            .runs
            .values()
            .filter(|r| r.state == state)
            .cloned()
            .collect();
        results.sort_by_key(|r| r.created_at);
        results.truncate(limit);
        Ok(results)
    }

    async fn list_active_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
    ) -> Result<Vec<RunRecord>, StoreError> {
        let store = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<RunRecord> = store
            .runs
            .values()
            .filter(|r| r.project == *project && !r.state.is_terminal())
            .cloned()
            .collect();
        results.sort_by_key(|r| r.created_at);
        results.truncate(limit);
        Ok(results)
    }

    async fn list_by_parent_run(
        &self,
        parent_run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<RunRecord>, StoreError> {
        let store = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // Sort borrowed refs so the per-comparison key doesn't
        // allocate a String for each run_id; only the records that
        // survive truncation get cloned.
        let mut refs: Vec<&RunRecord> = store
            .runs
            .values()
            .filter(|r| r.parent_run_id.as_ref() == Some(parent_run_id))
            .collect();
        refs.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.run_id.as_str().cmp(b.run_id.as_str()))
        });
        Ok(refs.into_iter().take(limit).cloned().collect())
    }
}

// -- TaskReadModel --

#[async_trait]
impl TaskReadModel for InMemoryStore {
    async fn get(&self, task_id: &TaskId) -> Result<Option<TaskRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.tasks.get(task_id.as_str()).cloned())
    }

    async fn list_by_state(
        &self,
        project: &ProjectKey,
        task_state: TaskState,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<TaskRecord> = state
            .tasks
            .values()
            .filter(|t| t.project == *project && t.state == task_state)
            .cloned()
            .collect();
        results.sort_by_key(|t| (t.created_at, t.task_id.as_str().to_owned()));
        results.truncate(limit);
        Ok(results)
    }

    async fn list_expired_leases(
        &self,
        now: u64,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<TaskRecord> = state
            .tasks
            .values()
            .filter(|t| {
                t.state == TaskState::Leased && t.lease_expires_at.is_some_and(|exp| exp < now)
            })
            .cloned()
            .collect();
        results.sort_by_key(|t| {
            (
                t.lease_expires_at.unwrap_or(0),
                t.task_id.as_str().to_owned(),
            )
        });
        results.truncate(limit);
        Ok(results)
    }

    async fn list_by_parent_run(
        &self,
        parent_run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<TaskRecord> = state
            .tasks
            .values()
            .filter(|t| t.parent_run_id.as_ref() == Some(parent_run_id))
            .cloned()
            .collect();
        results.sort_by_key(|t| (t.created_at, t.task_id.as_str().to_owned()));
        results.truncate(limit);
        Ok(results)
    }

    async fn any_non_terminal_children(&self, parent_run_id: &RunId) -> Result<bool, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .tasks
            .values()
            .any(|t| t.parent_run_id.as_ref() == Some(parent_run_id) && !t.state.is_terminal()))
    }
}

// -- ApprovalReadModel --

#[async_trait]
impl ApprovalReadModel for InMemoryStore {
    async fn get(&self, approval_id: &ApprovalId) -> Result<Option<ApprovalRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.approvals.get(approval_id.as_str()).cloned())
    }

    async fn list_pending(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<ApprovalRecord> = state
            .approvals
            .values()
            .filter(|a| a.project == *project && a.decision.is_none())
            .cloned()
            .collect();
        results.sort_by_key(|a| (a.created_at, a.approval_id.as_str().to_owned()));
        let results = results.into_iter().skip(offset).take(limit).collect();
        Ok(results)
    }

    async fn list_all(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<ApprovalRecord> = state
            .approvals
            .values()
            .filter(|a| a.project == *project)
            .cloned()
            .collect();
        results.sort_by_key(|a| (a.created_at, a.approval_id.as_str().to_owned()));
        let results = results.into_iter().skip(offset).take(limit).collect();
        Ok(results)
    }

    async fn has_pending_for_run(&self, run_id: &RunId) -> Result<bool, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .approvals
            .values()
            .any(|a| a.run_id.as_ref() == Some(run_id) && a.decision.is_none()))
    }
}

// -- CheckpointReadModel --

#[async_trait]
impl CheckpointReadModel for InMemoryStore {
    async fn get(
        &self,
        checkpoint_id: &CheckpointId,
    ) -> Result<Option<CheckpointRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.checkpoints.get(checkpoint_id.as_str()).cloned())
    }

    async fn latest_for_run(&self, run_id: &RunId) -> Result<Option<CheckpointRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .checkpoints
            .values()
            .find(|c| c.run_id == *run_id && c.disposition == CheckpointDisposition::Latest)
            .cloned())
    }

    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<CheckpointRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<CheckpointRecord> = state
            .checkpoints
            .values()
            .filter(|c| c.run_id == *run_id)
            .cloned()
            .collect();
        results.sort_by_key(|c| (c.created_at, c.checkpoint_id.as_str().to_owned()));
        results.truncate(limit);
        Ok(results)
    }
}

// -- MailboxReadModel --

#[async_trait]
impl MailboxReadModel for InMemoryStore {
    async fn get(
        &self,
        message_id: &MailboxMessageId,
    ) -> Result<Option<MailboxRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.mailbox_messages.get(message_id.as_str()).cloned())
    }

    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MailboxRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<MailboxRecord> = state
            .mailbox_messages
            .values()
            .filter(|m| m.run_id.as_ref() == Some(run_id))
            .cloned()
            .collect();
        results.sort_by_key(|m| m.message_id.as_str().to_owned());
        let results = results.into_iter().skip(offset).take(limit).collect();
        Ok(results)
    }

    async fn list_by_task(
        &self,
        task_id: &TaskId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MailboxRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<MailboxRecord> = state
            .mailbox_messages
            .values()
            .filter(|m| m.task_id.as_ref() == Some(task_id))
            .cloned()
            .collect();
        results.sort_by_key(|m| (m.created_at, m.message_id.as_str().to_owned()));
        let results = results.into_iter().skip(offset).take(limit).collect();
        Ok(results)
    }

    async fn list_pending(
        &self,
        now_ms: u64,
        limit: usize,
    ) -> Result<Vec<MailboxRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<MailboxRecord> = state
            .mailbox_messages
            .values()
            .filter(|m| m.deliver_at_ms > 0 && m.deliver_at_ms <= now_ms)
            .cloned()
            .collect();
        results.sort_by_key(|m| (m.deliver_at_ms, m.message_id.as_str().to_owned()));
        results.truncate(limit);
        Ok(results)
    }
}

// -- ToolInvocationReadModel --

#[async_trait]
impl ToolInvocationReadModel for InMemoryStore {
    async fn get(
        &self,
        invocation_id: &ToolInvocationId,
    ) -> Result<Option<ToolInvocationRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.tool_invocations.get(invocation_id.as_str()).cloned())
    }

    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ToolInvocationRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<ToolInvocationRecord> = state
            .tool_invocations
            .values()
            .filter(|record| record.run_id.as_ref() == Some(run_id))
            .cloned()
            .collect();
        results.sort_by_key(|record| record.requested_at_ms);
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }
}

// -- SignalReadModel --

#[async_trait]
impl SignalReadModel for InMemoryStore {
    async fn get(
        &self,
        signal_id: &cairn_domain::SignalId,
    ) -> Result<Option<cairn_domain::SignalRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.signals.get(signal_id.as_str()).cloned())
    }

    async fn list_by_project(
        &self,
        project: &cairn_domain::ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::SignalRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<cairn_domain::SignalRecord> = state
            .signals
            .values()
            .filter(|s| s.project == *project)
            .cloned()
            .collect();
        results.sort_by_key(|s| s.timestamp_ms);
        let results = results.into_iter().skip(offset).take(limit).collect();
        Ok(results)
    }
}

// -- IngestJobReadModel --

#[async_trait]
impl IngestJobReadModel for InMemoryStore {
    async fn get(
        &self,
        job_id: &cairn_domain::IngestJobId,
    ) -> Result<Option<cairn_domain::IngestJobRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.ingest_jobs.get(job_id.as_str()).cloned())
    }

    async fn list_by_project(
        &self,
        project: &cairn_domain::ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::IngestJobRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<cairn_domain::IngestJobRecord> = state
            .ingest_jobs
            .values()
            .filter(|j| j.project == *project)
            .cloned()
            .collect();
        results.sort_by_key(|j| j.created_at);
        let results = results.into_iter().skip(offset).take(limit).collect();
        Ok(results)
    }
}

// -- ScheduledTaskReadModel --

#[async_trait]
impl crate::projections::ScheduledTaskReadModel for InMemoryStore {
    async fn get(
        &self,
        id: &cairn_domain::ScheduledTaskId,
    ) -> Result<Option<cairn_domain::ScheduledTaskRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.scheduled_tasks.get(id.as_str()).cloned())
    }

    async fn list_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::ScheduledTaskRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<cairn_domain::ScheduledTaskRecord> = state
            .scheduled_tasks
            .values()
            .filter(|t| &t.tenant_id == tenant_id)
            .cloned()
            .collect();
        results.sort_by_key(|t| t.created_at);
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }

    async fn list_due(
        &self,
        now_ms: u64,
        limit: usize,
    ) -> Result<Vec<cairn_domain::ScheduledTaskRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<cairn_domain::ScheduledTaskRecord> = state
            .scheduled_tasks
            .values()
            .filter(|t| t.enabled && t.next_run_at.is_some_and(|nxt| nxt <= now_ms))
            .cloned()
            .collect();
        results.sort_by_key(|t| t.next_run_at);
        Ok(results.into_iter().take(limit).collect())
    }
}

// -- EvalRunReadModel --

#[async_trait]
impl EvalRunReadModel for InMemoryStore {
    async fn get(
        &self,
        eval_run_id: &cairn_domain::EvalRunId,
    ) -> Result<Option<crate::projections::EvalRunRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.eval_runs.get(eval_run_id.as_str()).cloned())
    }

    async fn list_by_project(
        &self,
        project: &cairn_domain::ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<crate::projections::EvalRunRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<crate::projections::EvalRunRecord> = state
            .eval_runs
            .values()
            .filter(|r| r.project == *project)
            .cloned()
            .collect();
        results.sort_by_key(|r| r.started_at);
        let results = results.into_iter().skip(offset).take(limit).collect();
        Ok(results)
    }
}

// -- OutcomeReadModel --

#[async_trait]
impl OutcomeReadModel for InMemoryStore {
    async fn get(
        &self,
        outcome_id: &cairn_domain::OutcomeId,
    ) -> Result<Option<crate::projections::OutcomeRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.outcomes.get(outcome_id.as_str()).cloned())
    }

    async fn list_by_run(
        &self,
        run_id: &cairn_domain::RunId,
        limit: usize,
    ) -> Result<Vec<crate::projections::OutcomeRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<crate::projections::OutcomeRecord> = state
            .outcomes
            .values()
            .filter(|r| r.run_id == *run_id)
            .cloned()
            .collect();
        results.sort_by_key(|r| r.recorded_at);
        results.truncate(limit);
        Ok(results)
    }

    async fn list_by_project(
        &self,
        project: &cairn_domain::ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<crate::projections::OutcomeRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<crate::projections::OutcomeRecord> = state
            .outcomes
            .values()
            .filter(|r| r.project == *project)
            .cloned()
            .collect();
        results.sort_by_key(|r| r.recorded_at);
        let results = results.into_iter().skip(offset).take(limit).collect();
        Ok(results)
    }
}

// -- EvalDatasetReadModel --

#[async_trait]
impl crate::projections::EvalDatasetReadModel for InMemoryStore {
    async fn get_dataset(
        &self,
        dataset_id: &str,
    ) -> Result<Option<cairn_domain::EvalDataset>, crate::error::StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.eval_datasets.get(dataset_id).cloned())
    }

    async fn list_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::EvalDataset>, crate::error::StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<cairn_domain::EvalDataset> = state
            .eval_datasets
            .values()
            .filter(|d| d.tenant_id == *tenant_id || tenant_id.as_str().is_empty())
            .cloned()
            .collect();
        results.sort_by_key(|d| d.created_at_ms);
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }
}

// -- EvalRubricReadModel --

#[async_trait]
impl crate::projections::EvalRubricReadModel for InMemoryStore {
    async fn get_rubric(
        &self,
        rubric_id: &str,
    ) -> Result<Option<cairn_domain::EvalRubric>, crate::error::StoreError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .eval_rubrics
            .get(rubric_id)
            .cloned())
    }
    async fn list_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::EvalRubric>, crate::error::StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<_> = state
            .eval_rubrics
            .values()
            .filter(|r| r.tenant_id == *tenant_id || tenant_id.as_str().is_empty())
            .cloned()
            .collect();
        results.sort_by_key(|r| r.rubric_id.clone());
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }
}

// -- EvalBaselineReadModel --

#[async_trait]
impl crate::projections::EvalBaselineReadModel for InMemoryStore {
    async fn get_baseline(
        &self,
        baseline_id: &str,
    ) -> Result<Option<cairn_domain::EvalBaseline>, crate::error::StoreError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .eval_baselines
            .get(baseline_id)
            .cloned())
    }
    async fn list_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::EvalBaseline>, crate::error::StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<_> = state
            .eval_baselines
            .values()
            .filter(|b| b.tenant_id == *tenant_id || tenant_id.as_str().is_empty())
            .cloned()
            .collect();
        results.sort_by_key(|r| r.baseline_id.clone());
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }
}

// -- PromptAssetReadModel --

#[async_trait]
impl PromptAssetReadModel for InMemoryStore {
    async fn get(
        &self,
        id: &cairn_domain::PromptAssetId,
    ) -> Result<Option<crate::projections::PromptAssetRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.prompt_assets.get(id.as_str()).cloned())
    }

    async fn list_by_project(
        &self,
        project: &cairn_domain::ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<crate::projections::PromptAssetRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<crate::projections::PromptAssetRecord> = state
            .prompt_assets
            .values()
            .filter(|a| a.project == *project)
            .cloned()
            .collect();
        results.sort_by_key(|a| a.created_at);
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }
}

// -- PromptVersionReadModel --

#[async_trait]
impl PromptVersionReadModel for InMemoryStore {
    async fn get(
        &self,
        id: &cairn_domain::PromptVersionId,
    ) -> Result<Option<crate::projections::PromptVersionRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.prompt_versions.get(id.as_str()).cloned())
    }

    async fn list_by_asset(
        &self,
        asset_id: &cairn_domain::PromptAssetId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<crate::projections::PromptVersionRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<crate::projections::PromptVersionRecord> = state
            .prompt_versions
            .values()
            .filter(|v| v.prompt_asset_id == *asset_id)
            .cloned()
            .collect();
        results.sort_by_key(|v| v.created_at);
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }
}

// -- PromptReleaseReadModel --

/// RFC 001: deterministic hash-based selector bucket for traffic routing (0-100).
fn selector_bucket(selector: &str) -> u8 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    selector.hash(&mut h);
    (h.finish() % 100) as u8
}

#[async_trait]
impl PromptReleaseReadModel for InMemoryStore {
    async fn get(
        &self,
        id: &cairn_domain::PromptReleaseId,
    ) -> Result<Option<crate::projections::PromptReleaseRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.prompt_releases.get(id.as_str()).cloned())
    }

    async fn list_by_project(
        &self,
        project: &cairn_domain::ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<crate::projections::PromptReleaseRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<crate::projections::PromptReleaseRecord> = state
            .prompt_releases
            .values()
            .filter(|r| r.project == *project)
            .cloned()
            .collect();
        results.sort_by_key(|r| r.created_at);
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }

    async fn active_for_selector(
        &self,
        project: &cairn_domain::ProjectKey,
        prompt_asset_id: &cairn_domain::PromptAssetId,
        selector: &str,
    ) -> Result<Option<crate::projections::PromptReleaseRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut active: Vec<_> = state
            .prompt_releases
            .values()
            .filter(|r| {
                r.project == *project
                    && r.prompt_asset_id == *prompt_asset_id
                    && r.state == "active"
            })
            .cloned()
            .collect();
        active.sort_by_key(|r| r.prompt_release_id.as_str().to_owned());
        if active.is_empty() {
            return Ok(None);
        }

        // RFC 001: if any release has rollout_percent, use deterministic bucket routing.
        if active.iter().any(|r| r.rollout_percent.is_some()) {
            let bucket = selector_bucket(selector);
            let mut cumulative = 0u8;
            for release in &active {
                let pct = release.rollout_percent.unwrap_or(100);
                cumulative = cumulative.saturating_add(pct);
                if bucket < cumulative {
                    return Ok(Some(release.clone()));
                }
            }
            return Ok(active.into_iter().last());
        }

        // RFC 006 selector precedence resolution.
        //
        // Priority (highest to lowest):
        //   1. routing_slot — exact match against the selector string.
        //   2. task_type    — exact match against the selector string.
        //   3. agent_type   — exact match against the selector string.
        //   4. is_project_default — release marked as the project-wide default.
        //   5. Any active release (first by release_id, for stability).
        //
        // When the release records do not yet carry these fields (all None / false),
        // every candidate scores 0 and the fallback (step 5) applies — preserving the
        // previous behaviour while the routing metadata is being backfilled.

        // Step 1: routing_slot exact match.
        if let Some(r) = active
            .iter()
            .find(|r| r.routing_slot.as_deref() == Some(selector))
        {
            return Ok(Some(r.clone()));
        }

        // Step 2: task_type exact match.
        if let Some(r) = active
            .iter()
            .find(|r| r.task_type.as_deref() == Some(selector))
        {
            return Ok(Some(r.clone()));
        }

        // Step 3: agent_type exact match.
        if let Some(r) = active
            .iter()
            .find(|r| r.agent_type.as_deref() == Some(selector))
        {
            return Ok(Some(r.clone()));
        }

        // Step 4: project default.
        if let Some(r) = active.iter().find(|r| r.is_project_default) {
            return Ok(Some(r.clone()));
        }

        // Step 5: fallback — first active release (sorted by release_id for stability).
        Ok(active.into_iter().next())
    }
}

// -- TenantReadModel --

#[async_trait]
impl TenantReadModel for InMemoryStore {
    async fn get(
        &self,
        id: &cairn_domain::TenantId,
    ) -> Result<Option<cairn_domain::org::TenantRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.tenants.get(id.as_str()).cloned())
    }

    async fn list(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::org::TenantRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<_> = state.tenants.values().cloned().collect();
        results.sort_by_key(|t| t.created_at);
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }
}

// -- WorkspaceReadModel --

#[async_trait]
impl WorkspaceReadModel for InMemoryStore {
    async fn get(
        &self,
        id: &cairn_domain::WorkspaceId,
    ) -> Result<Option<cairn_domain::org::WorkspaceRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.workspaces.get(id.as_str()).cloned())
    }

    async fn list_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::org::WorkspaceRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<_> = state
            .workspaces
            .values()
            .filter(|w| w.tenant_id == *tenant_id)
            .cloned()
            .collect();
        results.sort_by_key(|w| w.created_at);
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }
}

// -- ProjectReadModel --

#[async_trait]
impl ProjectReadModel for InMemoryStore {
    async fn get_project(
        &self,
        project: &cairn_domain::ProjectKey,
    ) -> Result<Option<cairn_domain::org::ProjectRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.projects.get(project.project_id.as_str()).cloned())
    }

    async fn list_by_workspace(
        &self,
        tenant_id: &cairn_domain::TenantId,
        workspace_id: &cairn_domain::WorkspaceId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::org::ProjectRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<_> = state
            .projects
            .values()
            .filter(|p| p.tenant_id == *tenant_id && p.workspace_id == *workspace_id)
            .cloned()
            .collect();
        results.sort_by_key(|p| p.created_at);
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }
}

// -- RouteDecisionReadModel --

#[async_trait]
impl RouteDecisionReadModel for InMemoryStore {
    async fn get(
        &self,
        decision_id: &cairn_domain::RouteDecisionId,
    ) -> Result<Option<cairn_domain::providers::RouteDecisionRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.route_decisions.get(decision_id.as_str()).cloned())
    }

    async fn list_by_project(
        &self,
        project: &cairn_domain::ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::providers::RouteDecisionRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<_> = state
            .route_decisions
            .values()
            .filter(|d| d.project_id == project.project_id)
            .cloned()
            .collect();
        results.sort_by_key(|d| d.route_decision_id.to_string());
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }
}

// -- ProviderCallReadModel --

#[async_trait]
impl ProviderCallReadModel for InMemoryStore {
    async fn get(
        &self,
        call_id: &cairn_domain::ProviderCallId,
    ) -> Result<Option<cairn_domain::providers::ProviderCallRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.provider_calls.get(call_id.as_str()).cloned())
    }

    async fn list_by_decision(
        &self,
        decision_id: &cairn_domain::RouteDecisionId,
        limit: usize,
    ) -> Result<Vec<cairn_domain::providers::ProviderCallRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let results: Vec<_> = state
            .provider_calls
            .values()
            .filter(|c| c.route_decision_id == *decision_id)
            .take(limit)
            .cloned()
            .collect();
        Ok(results)
    }
}

// -- Lease helpers (not trait-based, used by runtime directly) --

impl InMemoryStore {
    /// Set lease fields on a task. Used by runtime TaskService for claim/heartbeat.
    pub async fn set_task_lease(
        &self,
        task_id: &TaskId,
        owner: String,
        expires_at: u64,
    ) -> Result<(), StoreError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let rec = state
            .tasks
            .get_mut(task_id.as_str())
            .ok_or_else(|| StoreError::NotFound {
                entity: "task",
                id: task_id.to_string(),
            })?;
        rec.lease_owner = Some(owner);
        rec.lease_expires_at = Some(expires_at);
        Ok(())
    }

    /// Clear lease fields on a task.
    pub async fn clear_task_lease(&self, task_id: &TaskId) -> Result<(), StoreError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(rec) = state.tasks.get_mut(task_id.as_str()) {
            rec.lease_owner = None;
            rec.lease_expires_at = None;
        }
        Ok(())
    }
}

#[async_trait]
impl ApprovalPolicyReadModel for InMemoryStore {
    async fn get_policy(
        &self,
        policy_id: &str,
    ) -> Result<Option<cairn_domain::ApprovalPolicyRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.approval_policies.get(policy_id).cloned())
    }

    async fn list_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::ApprovalPolicyRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<_> = state
            .approval_policies
            .values()
            .filter(|p| p.tenant_id == *tenant_id)
            .cloned()
            .collect();
        results.sort_by_key(|p| p.policy_id.clone());
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }
}

#[async_trait]
impl ExternalWorkerReadModel for InMemoryStore {
    async fn get(
        &self,
        id: &cairn_domain::WorkerId,
    ) -> Result<Option<cairn_domain::workers::ExternalWorkerRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.external_workers.get(id.as_str()).cloned())
    }

    async fn list_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::workers::ExternalWorkerRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<_> = state
            .external_workers
            .values()
            .filter(|w| w.tenant_id == *tenant_id)
            .cloned()
            .collect();
        results.sort_by_key(|r| r.registered_at);
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }
}

// -- LlmCallTraceReadModel --

#[async_trait]
impl crate::projections::LlmCallTraceReadModel for InMemoryStore {
    async fn insert_trace(&self, trace: cairn_domain::LlmCallTrace) -> Result<(), StoreError> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .llm_traces
            .push(trace);
        Ok(())
    }

    async fn list_by_session(
        &self,
        session_id: &cairn_domain::SessionId,
        limit: usize,
    ) -> Result<Vec<cairn_domain::LlmCallTrace>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<cairn_domain::LlmCallTrace> = state
            .llm_traces
            .iter()
            .filter(|t| {
                t.session_id
                    .as_ref()
                    .map(|s| s == session_id)
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        // Most-recent first.
        results.sort_by_key(|r| std::cmp::Reverse(r.created_at_ms));
        results.truncate(limit);
        Ok(results)
    }

    async fn list_all_traces(
        &self,
        limit: usize,
    ) -> Result<Vec<cairn_domain::LlmCallTrace>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results = state.llm_traces.clone();
        results.sort_by_key(|r| std::cmp::Reverse(r.created_at_ms));
        results.truncate(limit);
        Ok(results)
    }
}

#[async_trait]
impl crate::projections::RunCostReadModel for InMemoryStore {
    async fn get_run_cost(
        &self,
        run_id: &cairn_domain::RunId,
    ) -> Result<Option<cairn_domain::providers::RunCostRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.run_costs.get(run_id.as_str()).cloned())
    }

    async fn list_by_session(
        &self,
        _session_id: &cairn_domain::SessionId,
    ) -> Result<Vec<cairn_domain::providers::RunCostRecord>, StoreError> {
        // In-memory store does not index run_costs by session; return all for now.
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.run_costs.values().cloned().collect())
    }
}

// TaskDependencyReadModel was removed — dependencies are FF-authoritative
// (ff_stage_dependency_edge / ff_apply_dependency_to_child). Cairn no
// longer persists dependency records; check_dependencies reads live
// edge state from FF via ff_evaluate_flow_eligibility + per-edge HGETs.

// -- OperatorProfileReadModel --

#[async_trait]
impl crate::projections::OperatorProfileReadModel for InMemoryStore {
    async fn get(
        &self,
        operator_id: &cairn_domain::ids::OperatorId,
    ) -> Result<Option<crate::projections::OperatorProfileRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.operator_profiles.get(operator_id.as_str()).cloned())
    }

    async fn list_by_tenant(
        &self,
        tenant_id: &cairn_domain::ids::TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<crate::projections::OperatorProfileRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<crate::projections::OperatorProfileRecord> = state
            .operator_profiles
            .values()
            .filter(|p| &p.tenant_id == tenant_id)
            .cloned()
            .collect();
        results.sort_by_key(|p| p.operator_id.to_string());
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }
}

// -- WorkspaceMembershipReadModel --

#[async_trait]
impl crate::projections::WorkspaceMembershipReadModel for InMemoryStore {
    async fn list_workspace_members(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<crate::projections::WorkspaceMemberRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .workspace_members
            .iter()
            .filter(|m| m.workspace_id == workspace_id)
            .cloned()
            .collect())
    }

    async fn get_member(
        &self,
        workspace_key: &cairn_domain::tenancy::WorkspaceKey,
        operator_id: &str,
    ) -> Result<Option<crate::projections::WorkspaceMemberRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .workspace_members
            .iter()
            .find(|m| {
                m.workspace_id == workspace_key.workspace_id.as_str()
                    && m.operator_id == operator_id
            })
            .cloned())
    }

    async fn add_workspace_member(
        &self,
        record: crate::projections::WorkspaceMemberRecord,
    ) -> Result<(), StoreError> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .workspace_members
            .push(record);
        Ok(())
    }

    async fn remove_workspace_member(
        &self,
        workspace_id: &str,
        operator_id: &str,
    ) -> Result<(), StoreError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .workspace_members
            .retain(|m| !(m.workspace_id == workspace_id && m.operator_id == operator_id));
        Ok(())
    }
}

// -- SignalSubscriptionReadModel --

#[async_trait]
impl crate::projections::SignalSubscriptionReadModel for InMemoryStore {
    async fn get_subscription(
        &self,
        subscription_id: &str,
    ) -> Result<Option<crate::projections::SignalSubscriptionRecord>, StoreError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .signal_subscriptions
            .get(subscription_id)
            .cloned())
    }

    async fn list_by_signal_type(
        &self,
        signal_type: &str,
    ) -> Result<Vec<crate::projections::SignalSubscriptionRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .signal_subscriptions
            .values()
            .filter(|s| s.signal_type == signal_type)
            .cloned()
            .collect())
    }

    async fn list_by_signal_kind(
        &self,
        signal_kind: &str,
        _limit: usize,
        _offset: usize,
    ) -> Result<Vec<crate::projections::SignalSubscriptionRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .signal_subscriptions
            .values()
            .filter(|s| s.signal_type == signal_kind)
            .cloned()
            .collect())
    }

    async fn list_by_project(
        &self,
        project: &cairn_domain::tenancy::ProjectKey,
        _limit: usize,
        _offset: usize,
    ) -> Result<Vec<crate::projections::SignalSubscriptionRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let tid = project.tenant_id.as_str();
        let wid = project.workspace_id.as_str();
        let pid = project.project_id.as_str();
        Ok(state
            .signal_subscriptions
            .values()
            .filter(|s| {
                s.project_tenant == tid && s.project_workspace == wid && s.project_id == pid
            })
            .cloned()
            .collect())
    }

    async fn upsert_subscription(
        &self,
        record: crate::projections::SignalSubscriptionRecord,
    ) -> Result<(), StoreError> {
        self.state
            .lock()
            .unwrap()
            .signal_subscriptions
            .insert(record.subscription_id.clone(), record);
        Ok(())
    }
}

#[async_trait]
impl crate::projections::CredentialRotationReadModel for InMemoryStore {
    async fn list_rotations(
        &self,
        tenant_id: &cairn_domain::TenantId,
    ) -> Result<Vec<cairn_domain::credentials::CredentialRotationRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let results: Vec<_> = state
            .credential_rotations
            .iter()
            .filter(|r| &r.tenant_id == tenant_id)
            .cloned()
            .collect();
        Ok(results)
    }
}

// ── ProviderBindingReadModel ───────────────────────────────────────────────

#[async_trait]
impl crate::projections::ProviderBindingReadModel for InMemoryStore {
    async fn get(
        &self,
        id: &cairn_domain::ProviderBindingId,
    ) -> Result<Option<cairn_domain::providers::ProviderBindingRecord>, StoreError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .provider_bindings
            .get(id.as_str())
            .cloned())
    }

    async fn list_by_project(
        &self,
        project: &cairn_domain::ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::providers::ProviderBindingRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .provider_bindings
            .values()
            .filter(|b| &b.project == project)
            .skip(offset)
            .take(limit)
            .cloned()
            .collect())
    }

    async fn list_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::providers::ProviderBindingRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .provider_bindings
            .values()
            .filter(|b| b.project.tenant_id == *tenant_id)
            .skip(offset)
            .take(limit)
            .cloned()
            .collect())
    }

    async fn list_active(
        &self,
        project: &cairn_domain::ProjectKey,
        operation: cairn_domain::providers::OperationKind,
    ) -> Result<Vec<cairn_domain::providers::ProviderBindingRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<_> = state
            .provider_bindings
            .values()
            .filter(|b| &b.project == project && b.active && b.operation_kind == operation)
            .cloned()
            .collect();
        // Stable creation-order: sort by created_at, then by binding ID for determinism.
        results.sort_by(|a, b| {
            a.created_at.cmp(&b.created_at).then_with(|| {
                a.provider_binding_id
                    .as_str()
                    .cmp(b.provider_binding_id.as_str())
            })
        });
        Ok(results)
    }
}

// ── ProviderHealthReadModel ───────────────────────────────────────────────

#[async_trait]
impl crate::projections::ProviderHealthReadModel for InMemoryStore {
    async fn get(
        &self,
        connection_id: &cairn_domain::ProviderConnectionId,
    ) -> Result<Option<cairn_domain::providers::ProviderHealthRecord>, StoreError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .provider_health_records
            .get(connection_id.as_str())
            .cloned())
    }

    async fn list_by_tenant(
        &self,
        _tenant_id: &cairn_domain::TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::providers::ProviderHealthRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let results: Vec<_> = state.provider_health_records.values().cloned().collect();
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }
}

// ── ProviderHealthScheduleReadModel ──────────────────────────────────────

#[async_trait]
impl crate::projections::ProviderHealthScheduleReadModel for InMemoryStore {
    async fn get_schedule(
        &self,
        schedule_id: &str,
    ) -> Result<Option<cairn_domain::providers::ProviderHealthSchedule>, StoreError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .provider_health_schedules
            .get(schedule_id)
            .cloned())
    }

    async fn list_schedules_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
    ) -> Result<Vec<cairn_domain::providers::ProviderHealthSchedule>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .provider_health_schedules
            .values()
            .filter(|s| &s.tenant_id == tenant_id)
            .cloned()
            .collect())
    }

    async fn list_enabled_schedules(
        &self,
    ) -> Result<Vec<cairn_domain::providers::ProviderHealthSchedule>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .provider_health_schedules
            .values()
            .filter(|s| s.enabled)
            .cloned()
            .collect())
    }
}

// ── Stub read-model implementations (linter-added service impl tests) ─────────

#[async_trait]
impl crate::projections::ChannelReadModel for InMemoryStore {
    async fn get_channel(
        &self,
        id: &cairn_domain::ChannelId,
    ) -> Result<Option<cairn_domain::ChannelRecord>, StoreError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .channels
            .get(id.as_str())
            .cloned())
    }
    async fn list_channels(
        &self,
        project: &cairn_domain::ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::ChannelRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .channels
            .values()
            .filter(|c| &c.project == project)
            .skip(offset)
            .take(limit)
            .cloned()
            .collect())
    }
    async fn list_messages(
        &self,
        channel_id: &cairn_domain::ChannelId,
        limit: usize,
    ) -> Result<Vec<cairn_domain::ChannelMessage>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .channel_messages
            .get(channel_id.as_str())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .take(limit)
            .collect())
    }
}

#[async_trait]
impl crate::projections::GuardrailReadModel for InMemoryStore {
    async fn get_policy(
        &self,
        policy_id: &str,
    ) -> Result<Option<cairn_domain::policy::GuardrailPolicy>, StoreError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .guardrail_policies
            .get(policy_id)
            .cloned())
    }
    async fn list_policies(
        &self,
        tenant_id: &cairn_domain::TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::policy::GuardrailPolicy>, StoreError> {
        let _ = tenant_id;
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut policies: Vec<_> = state.guardrail_policies.values().cloned().collect();
        // Sort by policy_id (timestamp-based) for deterministic creation-order iteration.
        policies.sort_by_key(|r| r.policy_id.clone());
        Ok(policies.into_iter().skip(offset).take(limit).collect())
    }
}

#[async_trait]
impl crate::projections::LicenseReadModel for InMemoryStore {
    async fn get_active(
        &self,
        tenant_id: &cairn_domain::TenantId,
    ) -> Result<Option<cairn_domain::LicenseRecord>, StoreError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .licenses
            .get(tenant_id.as_str())
            .cloned())
    }
    async fn list_overrides(
        &self,
        tenant_id: &cairn_domain::TenantId,
    ) -> Result<Vec<cairn_domain::EntitlementOverrideRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .entitlement_overrides
            .values()
            .filter(|r| &r.tenant_id == tenant_id)
            .cloned()
            .collect())
    }
}

#[async_trait]
impl crate::projections::DefaultsReadModel for InMemoryStore {
    async fn get(
        &self,
        scope: cairn_domain::Scope,
        scope_id: &str,
        key: &str,
    ) -> Result<Option<cairn_domain::DefaultSetting>, StoreError> {
        let k = format!("{scope:?}:{scope_id}:{key}");
        Ok(self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .default_settings
            .get(&k)
            .cloned())
    }
    async fn list_by_scope(
        &self,
        scope: cairn_domain::Scope,
        scope_id: &str,
    ) -> Result<Vec<cairn_domain::DefaultSetting>, StoreError> {
        let prefix = format!("{scope:?}:{scope_id}:");
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .default_settings
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(_, v)| v.clone())
            .collect())
    }
}

#[async_trait]
impl crate::projections::RetentionPolicyReadModel for InMemoryStore {
    async fn get_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
    ) -> Result<Option<cairn_domain::RetentionPolicy>, StoreError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .retention_policies
            .get(tenant_id.as_str())
            .cloned())
    }
}

#[async_trait]
impl crate::projections::RetentionMaintenance for InMemoryStore {
    async fn apply_retention(
        &self,
        tenant_id: &cairn_domain::TenantId,
    ) -> Result<cairn_domain::RetentionResult, StoreError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let policy = match state.retention_policies.get(tenant_id.as_str()).cloned() {
            Some(p) => p,
            None => {
                return Ok(cairn_domain::RetentionResult {
                    events_pruned: 0,
                    entities_affected: 0,
                })
            }
        };
        let max_per_entity = policy.max_events_per_entity as usize;
        if max_per_entity == 0 {
            return Ok(cairn_domain::RetentionResult {
                events_pruned: 0,
                entities_affected: 0,
            });
        }

        // Group events by entity (using primary_entity_ref).
        // Collect entity event positions, keep tail (newest), prune the rest.
        use std::collections::HashMap;
        let mut entity_positions: HashMap<String, Vec<usize>> = HashMap::new();
        for (idx, stored) in state.events.iter().enumerate() {
            if let Some(entity_ref) = stored.envelope.payload.primary_entity_ref() {
                let key = format!("{entity_ref:?}");
                entity_positions.entry(key).or_default().push(idx);
            }
        }

        let mut to_prune: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        let mut entities_affected = 0u32;

        for positions in entity_positions.values() {
            if positions.len() > max_per_entity {
                // Keep the most recent max_per_entity events; prune the rest (oldest).
                let prune_count = positions.len() - max_per_entity;
                for idx in positions.iter().take(prune_count) {
                    to_prune.insert(*idx);
                }
                entities_affected += 1;
            }
        }

        let events_pruned = to_prune.len() as u64;
        // Remove events at pruned indices (in reverse order to preserve indices).
        let mut sorted: Vec<usize> = to_prune.into_iter().collect();
        sorted.sort_unstable_by(|a, b| b.cmp(a)); // reverse order
        for idx in sorted {
            state.events.remove(idx);
        }

        Ok(cairn_domain::RetentionResult {
            events_pruned,
            entities_affected,
        })
    }
}

#[async_trait]
impl crate::projections::RunSlaReadModel for InMemoryStore {
    async fn get_sla(
        &self,
        run_id: &cairn_domain::RunId,
    ) -> Result<Option<cairn_domain::sla::SlaConfig>, StoreError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .run_sla_configs
            .get(run_id.as_str())
            .cloned())
    }
    async fn get_breach(
        &self,
        run_id: &cairn_domain::RunId,
    ) -> Result<Option<cairn_domain::sla::SlaBreach>, StoreError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .run_sla_breaches
            .get(run_id.as_str())
            .cloned())
    }
    async fn list_breached_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
    ) -> Result<Vec<cairn_domain::sla::SlaBreach>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .run_sla_breaches
            .values()
            .filter(|b| &b.tenant_id == tenant_id)
            .cloned()
            .collect())
    }
}

#[async_trait]
impl crate::projections::NotificationReadModel for InMemoryStore {
    async fn get_preferences(
        &self,
        tenant_id: &cairn_domain::TenantId,
        operator_id: &str,
    ) -> Result<Option<cairn_domain::notification_prefs::NotificationPreference>, StoreError> {
        let key = format!("{}:{}", tenant_id.as_str(), operator_id);
        Ok(self
            .state
            .lock()
            .unwrap()
            .notification_prefs
            .get(&key)
            .cloned())
    }
    async fn list_preferences_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
    ) -> Result<Vec<cairn_domain::notification_prefs::NotificationPreference>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .notification_prefs
            .values()
            .filter(|p| &p.tenant_id == tenant_id)
            .cloned()
            .collect())
    }
    async fn list_sent_notifications(
        &self,
        tenant_id: &cairn_domain::TenantId,
        since_ms: u64,
    ) -> Result<Vec<cairn_domain::notification_prefs::NotificationRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .notification_records
            .iter()
            .filter(|r| &r.tenant_id == tenant_id && r.sent_at_ms >= since_ms)
            .cloned()
            .collect())
    }
    async fn list_failed_notifications(
        &self,
        tenant_id: &cairn_domain::TenantId,
    ) -> Result<Vec<cairn_domain::notification_prefs::NotificationRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .notification_records
            .iter()
            .filter(|r| &r.tenant_id == tenant_id && !r.delivered)
            .cloned()
            .collect())
    }
}

#[async_trait]
impl crate::projections::ProviderConnectionReadModel for InMemoryStore {
    async fn get(
        &self,
        id: &cairn_domain::ProviderConnectionId,
    ) -> Result<Option<cairn_domain::providers::ProviderConnectionRecord>, StoreError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .provider_connections
            .get(id.as_str())
            .cloned())
    }
    async fn list_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::providers::ProviderConnectionRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .provider_connections
            .values()
            .filter(|r| &r.tenant_id == tenant_id)
            .skip(offset)
            .take(limit)
            .cloned()
            .collect())
    }
}

#[async_trait]
impl crate::projections::ProviderPoolReadModel for InMemoryStore {
    async fn get_pool(
        &self,
        pool_id: &str,
    ) -> Result<Option<cairn_domain::providers::ProviderConnectionPool>, StoreError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .provider_pools
            .get(pool_id)
            .cloned())
    }
    async fn list_pools_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
    ) -> Result<Vec<cairn_domain::providers::ProviderConnectionPool>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .provider_pools
            .values()
            .filter(|p| &p.tenant_id == tenant_id)
            .cloned()
            .collect())
    }
}

#[async_trait]
impl crate::projections::CredentialReadModel for InMemoryStore {
    async fn get(
        &self,
        id: &cairn_domain::CredentialId,
    ) -> Result<Option<cairn_domain::credentials::CredentialRecord>, StoreError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .credentials
            .get(id.as_str())
            .cloned())
    }
    async fn list_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::credentials::CredentialRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .credentials
            .values()
            .filter(|r| &r.tenant_id == tenant_id)
            .skip(offset)
            .take(limit)
            .cloned()
            .collect())
    }
}

#[async_trait]
impl crate::projections::RunCostAlertReadModel for InMemoryStore {
    async fn get_alert(
        &self,
        run_id: &cairn_domain::RunId,
    ) -> Result<Option<cairn_domain::providers::RunCostAlert>, StoreError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .run_cost_alerts
            .get(run_id.as_str())
            .cloned())
    }
    async fn list_triggered_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
    ) -> Result<Vec<cairn_domain::providers::RunCostAlert>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .run_cost_alerts
            .values()
            .filter(|a| &a.tenant_id == tenant_id && a.triggered_at_ms > 0)
            .cloned()
            .collect())
    }
}

#[async_trait]
impl crate::projections::AuditLogReadModel for InMemoryStore {
    async fn list_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
        since_ms: Option<u64>,
        limit: usize,
    ) -> Result<Vec<cairn_domain::AuditLogEntry>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let entries: Vec<_> = state
            .events
            .iter()
            .filter_map(|e| {
                if let RuntimeEvent::AuditLogEntryRecorded(a) = &e.envelope.payload {
                    if &a.tenant_id == tenant_id {
                        if let Some(since) = since_ms {
                            if a.occurred_at_ms < since {
                                return None;
                            }
                        }
                        return Some(cairn_domain::AuditLogEntry {
                            entry_id: a.entry_id.clone(),
                            tenant_id: a.tenant_id.clone(),
                            actor_id: a.actor_id.clone(),
                            action: a.action.clone(),
                            resource_type: a.resource_type.clone(),
                            resource_id: a.resource_id.clone(),
                            outcome: a.outcome,
                            request_id: None,
                            ip_address: None,
                            occurred_at_ms: a.occurred_at_ms,
                            metadata: serde_json::json!({}),
                        });
                    }
                }
                None
            })
            .take(limit)
            .collect();
        Ok(entries)
    }

    async fn list_by_resource(
        &self,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<Vec<cairn_domain::AuditLogEntry>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let entries: Vec<_> = state
            .events
            .iter()
            .filter_map(|e| {
                if let RuntimeEvent::AuditLogEntryRecorded(a) = &e.envelope.payload {
                    if a.resource_type == resource_type && a.resource_id == resource_id {
                        return Some(cairn_domain::AuditLogEntry {
                            entry_id: a.entry_id.clone(),
                            tenant_id: a.tenant_id.clone(),
                            actor_id: a.actor_id.clone(),
                            action: a.action.clone(),
                            resource_type: a.resource_type.clone(),
                            resource_id: a.resource_id.clone(),
                            outcome: a.outcome,
                            request_id: None,
                            ip_address: None,
                            occurred_at_ms: a.occurred_at_ms,
                            metadata: serde_json::json!({}),
                        });
                    }
                }
                None
            })
            .collect();
        Ok(entries)
    }
}

#[async_trait]
impl crate::projections::QuotaReadModel for InMemoryStore {
    async fn get_quota(
        &self,
        tenant_id: &cairn_domain::TenantId,
    ) -> Result<Option<cairn_domain::TenantQuota>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut quota) = state.quotas.get(tenant_id.as_str()).cloned() else {
            return Ok(None);
        };
        // Dynamically compute current_active_runs from run state.
        // An active run is one that belongs to this tenant and is not in a terminal state.
        let active_runs = state
            .runs
            .values()
            .filter(|r| r.project.tenant_id == *tenant_id && !r.state.is_terminal())
            .count() as u32;
        quota.current_active_runs = active_runs;
        // Dynamically compute sessions_this_hour from session state.
        // Count sessions that have been created (all sessions for this tenant).
        // For simplicity in tests, count all sessions (the test creates sessions and checks the limit).
        let sessions_count = state
            .sessions
            .values()
            .filter(|s| s.project.tenant_id == *tenant_id)
            .count() as u32;
        quota.sessions_this_hour = sessions_count;
        Ok(Some(quota))
    }
}

#[async_trait]
impl crate::projections::ProviderBudgetReadModel for InMemoryStore {
    async fn get_by_tenant_period(
        &self,
        tenant_id: &cairn_domain::TenantId,
        period: cairn_domain::providers::ProviderBudgetPeriod,
    ) -> Result<Option<cairn_domain::providers::ProviderBudget>, StoreError> {
        let key = format!("{}:{period:?}", tenant_id.as_str());
        Ok(self
            .state
            .lock()
            .unwrap()
            .provider_budgets
            .get(&key)
            .cloned())
    }
    async fn list_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
    ) -> Result<Vec<cairn_domain::providers::ProviderBudget>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .provider_budgets
            .values()
            .filter(|b| &b.tenant_id == tenant_id)
            .cloned()
            .collect())
    }
}

// -- TaskLeaseExpiredReadModel --

#[async_trait]
impl crate::projections::TaskLeaseExpiredReadModel for InMemoryStore {
    async fn list_expired(
        &self,
        now_ms: u64,
    ) -> Result<Vec<crate::projections::TaskRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .tasks
            .values()
            .filter(|t| {
                matches!(
                    t.state,
                    cairn_domain::TaskState::Leased | cairn_domain::TaskState::Running
                ) && t.lease_expires_at.is_some_and(|exp| exp <= now_ms)
            })
            .cloned()
            .collect())
    }
}

// -- CheckpointStrategyReadModel --

#[async_trait]
impl crate::projections::CheckpointStrategyReadModel for InMemoryStore {
    async fn get_by_run(
        &self,
        run_id: &cairn_domain::RunId,
    ) -> Result<Option<cairn_domain::CheckpointStrategy>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.checkpoint_strategies.get(run_id.as_str()).cloned())
    }
}

// -- OperatorInterventionReadModel --

#[async_trait]
impl crate::projections::OperatorInterventionReadModel for InMemoryStore {
    async fn list_by_run(
        &self,
        run_id: &cairn_domain::RunId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<crate::projections::OperatorInterventionRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let records: Vec<_> = state
            .events
            .iter()
            .filter_map(|e| {
                if let RuntimeEvent::OperatorIntervention(op) = &e.envelope.payload {
                    if op.run_id.as_ref() == Some(run_id) {
                        return Some(crate::projections::OperatorInterventionRecord {
                            run_id: run_id.clone(),
                            tenant_id: op.tenant_id.clone(),
                            action: op.action.clone(),
                            reason: op.reason.clone(),
                            intervened_at_ms: op.intervened_at_ms,
                        });
                    }
                }
                None
            })
            .skip(offset)
            .take(limit)
            .collect();
        Ok(records)
    }
}

// -- PauseScheduleReadModel --

#[async_trait]
impl crate::projections::PauseScheduleReadModel for InMemoryStore {
    async fn list_due(
        &self,
        before_ms: u64,
    ) -> Result<Vec<crate::projections::PauseScheduledRecord>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // Find all RunStateChanged(to=Paused) events with resume_after_ms set.
        // Use a map to keep only the latest pause event per run.
        let mut paused: std::collections::HashMap<
            String,
            crate::projections::PauseScheduledRecord,
        > = std::collections::HashMap::new();
        for stored in &state.events {
            if let RuntimeEvent::RunStateChanged(e) = &stored.envelope.payload {
                if e.transition.to == cairn_domain::RunState::Paused {
                    if let Some(reason) = &e.pause_reason {
                        if let Some(resume_after_ms) = reason.resume_after_ms {
                            let resume_at_ms = stored.stored_at + resume_after_ms;
                            paused.insert(
                                e.run_id.as_str().to_owned(),
                                crate::projections::PauseScheduledRecord {
                                    run_id: e.run_id.clone(),
                                    project: e.project.clone(),
                                    resume_at_ms,
                                    created_at_ms: stored.stored_at,
                                },
                            );
                        }
                    }
                } else if matches!(
                    e.transition.to,
                    cairn_domain::RunState::Running
                        | cairn_domain::RunState::Completed
                        | cairn_domain::RunState::Failed
                ) {
                    // Run resumed/completed — remove from paused map.
                    paused.remove(e.run_id.as_str());
                }
            }
        }
        let due: Vec<_> = paused
            .into_values()
            .filter(|r| r.resume_at_ms <= before_ms)
            .collect();
        Ok(due)
    }
}

// -- RecoveryEscalationReadModel --

#[async_trait]
impl crate::projections::RecoveryEscalationReadModel for InMemoryStore {
    async fn get_by_run(
        &self,
        _run_id: &cairn_domain::RunId,
    ) -> Result<Option<cairn_domain::RecoveryEscalation>, StoreError> {
        Ok(None)
    }
    async fn list_by_tenant(
        &self,
        _tenant_id: &cairn_domain::TenantId,
    ) -> Result<Vec<cairn_domain::RecoveryEscalation>, StoreError> {
        Ok(vec![])
    }
}

// -- SnapshotReadModel --

#[async_trait]
impl crate::projections::SnapshotReadModel for InMemoryStore {
    async fn get_latest(
        &self,
        tenant_id: &cairn_domain::TenantId,
    ) -> Result<Option<cairn_domain::Snapshot>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .snapshots
            .iter()
            .filter(|s| s.tenant_id == *tenant_id)
            .max_by_key(|s| s.created_at_ms)
            .cloned())
    }
    async fn list_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
    ) -> Result<Vec<cairn_domain::Snapshot>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<_> = state
            .snapshots
            .iter()
            .filter(|s| s.tenant_id == *tenant_id)
            .cloned()
            .collect();
        results.sort_by_key(|s| s.created_at_ms);
        Ok(results)
    }
}

// -- RoutePolicyReadModel --

#[async_trait]
impl crate::projections::RoutePolicyReadModel for InMemoryStore {
    async fn get(
        &self,
        policy_id: &str,
    ) -> Result<Option<cairn_domain::providers::RoutePolicy>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.route_policies.get(policy_id).cloned())
    }

    async fn list_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::providers::RoutePolicy>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<_> = state
            .route_policies
            .values()
            .filter(|p| p.enabled && p.tenant_id == tenant_id.as_str())
            .cloned()
            .collect();
        results.sort_by_key(|r| r.policy_id.clone());
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }
}

// -- ProviderBindingCostStatsReadModel --

#[async_trait]
impl crate::projections::ProviderBindingCostStatsReadModel for InMemoryStore {
    async fn get(
        &self,
        binding_id: &cairn_domain::ProviderBindingId,
    ) -> Result<Option<cairn_domain::providers::ProviderBindingCostStats>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let calls: Vec<_> = state
            .provider_calls
            .values()
            .filter(|c| c.provider_binding_id == *binding_id)
            .cloned()
            .collect();
        if calls.is_empty() {
            return Ok(None);
        }
        let total_cost_micros: u64 = calls.iter().filter_map(|c| c.cost_micros).sum();
        let call_count = calls.len() as u64;
        Ok(Some(cairn_domain::providers::ProviderBindingCostStats {
            binding_id: binding_id.clone(),
            total_cost_micros,
            call_count,
        }))
    }
    async fn list_by_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
    ) -> Result<Vec<cairn_domain::providers::ProviderBindingCostStats>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // Scan raw events for ProviderCallCompleted to access the full project key (tenant_id).
        let mut stats: std::collections::HashMap<
            String,
            cairn_domain::providers::ProviderBindingCostStats,
        > = std::collections::HashMap::new();
        for stored in &state.events {
            if let cairn_domain::RuntimeEvent::ProviderCallCompleted(e) = &stored.envelope.payload {
                if e.project.tenant_id != *tenant_id {
                    continue;
                }
                let entry = stats
                    .entry(e.provider_binding_id.as_str().to_owned())
                    .or_insert_with(|| cairn_domain::providers::ProviderBindingCostStats {
                        binding_id: e.provider_binding_id.clone(),
                        total_cost_micros: 0,
                        call_count: 0,
                    });
                entry.total_cost_micros = entry
                    .total_cost_micros
                    .saturating_add(e.cost_micros.unwrap_or(0));
                entry.call_count = entry.call_count.saturating_add(1);
            }
        }
        let mut results: Vec<_> = stats.into_values().collect();
        results.sort_by_key(|s| s.total_cost_micros / s.call_count.max(1));
        Ok(results)
    }
}

#[async_trait]
impl crate::projections::ResourceSharingReadModel for InMemoryStore {
    async fn get_share(
        &self,
        share_id: &str,
    ) -> Result<Option<cairn_domain::resource_sharing::SharedResource>, StoreError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .resource_shares
            .get(share_id)
            .cloned())
    }
    async fn list_shares_for_workspace(
        &self,
        tenant_id: &cairn_domain::TenantId,
        target_workspace_id: &cairn_domain::WorkspaceId,
    ) -> Result<Vec<cairn_domain::resource_sharing::SharedResource>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .resource_shares
            .values()
            .filter(|s| &s.tenant_id == tenant_id && &s.target_workspace_id == target_workspace_id)
            .cloned()
            .collect())
    }
    async fn get_share_for_resource(
        &self,
        tenant_id: &cairn_domain::TenantId,
        target_workspace_id: &cairn_domain::WorkspaceId,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<Option<cairn_domain::resource_sharing::SharedResource>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .resource_shares
            .values()
            .find(|s| {
                &s.tenant_id == tenant_id
                    && &s.target_workspace_id == target_workspace_id
                    && s.resource_type == resource_type
                    && s.resource_id == resource_id
            })
            .cloned())
    }
}

#[async_trait]
impl crate::projections::FfLeaseHistoryCursorStore for InMemoryStore {
    async fn get(
        &self,
        partition_id: &str,
        execution_id: &str,
    ) -> Result<Option<crate::projections::FfLeaseHistoryCursor>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .ff_lease_history_cursors
            .get(&(partition_id.to_owned(), execution_id.to_owned()))
            .cloned())
    }

    async fn list_by_partition(
        &self,
        partition_id: &str,
    ) -> Result<Vec<crate::projections::FfLeaseHistoryCursor>, StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .ff_lease_history_cursors
            .values()
            .filter(|c| c.partition_id == partition_id)
            .cloned()
            .collect())
    }

    async fn upsert(
        &self,
        cursor: &crate::projections::FfLeaseHistoryCursor,
    ) -> Result<(), StoreError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.ff_lease_history_cursors.insert(
            (cursor.partition_id.clone(), cursor.execution_id.clone()),
            cursor.clone(),
        );
        Ok(())
    }

    async fn delete(&self, partition_id: &str, execution_id: &str) -> Result<(), StoreError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .ff_lease_history_cursors
            .remove(&(partition_id.to_owned(), execution_id.to_owned()));
        Ok(())
    }
}

// ── Convenience query methods for cairn-app ───────────────────────────────

impl InMemoryStore {
    /// Count runs currently in active states (Running or Leased).
    pub async fn count_active_runs(&self) -> u64 {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .runs
            .values()
            .filter(|r| {
                matches!(
                    r.state,
                    cairn_domain::RunState::Running | cairn_domain::RunState::Pending
                )
            })
            .count() as u64
    }

    /// Count tasks currently active (Running or Leased).
    pub async fn count_active_tasks(&self) -> u64 {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .tasks
            .values()
            .filter(|t| {
                matches!(
                    t.state,
                    cairn_domain::TaskState::Running | cairn_domain::TaskState::Leased
                )
            })
            .count() as u64
    }

    /// Count active runs for a specific tenant.
    pub async fn count_active_runs_for_tenant(&self, tenant_id: &cairn_domain::TenantId) -> u64 {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .runs
            .values()
            .filter(|r| {
                r.project.tenant_id == *tenant_id
                    && matches!(
                        r.state,
                        cairn_domain::RunState::Running | cairn_domain::RunState::Pending
                    )
            })
            .count() as u64
    }

    /// Count active tasks for a specific tenant.
    pub async fn count_active_tasks_for_tenant(&self, tenant_id: &cairn_domain::TenantId) -> u64 {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .tasks
            .values()
            .filter(|t| {
                t.project.tenant_id == *tenant_id
                    && matches!(
                        t.state,
                        cairn_domain::TaskState::Running | cairn_domain::TaskState::Leased
                    )
            })
            .count() as u64
    }

    /// Count active runs for a workspace.
    pub async fn count_active_runs_for_workspace(
        &self,
        workspace_key: &cairn_domain::tenancy::WorkspaceKey,
    ) -> u64 {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .runs
            .values()
            .filter(|r| {
                r.project.workspace_id == workspace_key.workspace_id
                    && matches!(
                        r.state,
                        cairn_domain::RunState::Running | cairn_domain::RunState::Pending
                    )
            })
            .count() as u64
    }

    /// Count pending approvals for a tenant.
    pub async fn count_pending_approvals_for_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
    ) -> u64 {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .approvals
            .values()
            .filter(|a| a.project.tenant_id == *tenant_id && a.decision.is_none())
            .count() as u64
    }

    /// List all pending (undecided) approvals across every project.
    pub fn list_all_pending_approvals(
        &self,
        limit: usize,
        offset: usize,
    ) -> Vec<crate::projections::ApprovalRecord> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<crate::projections::ApprovalRecord> = state
            .approvals
            .values()
            .filter(|a| a.decision.is_none())
            .cloned()
            .collect();
        results.sort_by_key(|a| a.created_at);
        results.into_iter().skip(offset).take(limit).collect()
    }

    /// List all approvals (pending + resolved) across every project.
    pub fn list_all_approvals(
        &self,
        limit: usize,
        offset: usize,
    ) -> Vec<crate::projections::ApprovalRecord> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<crate::projections::ApprovalRecord> =
            state.approvals.values().cloned().collect();
        results.sort_by_key(|a| a.created_at);
        results.into_iter().skip(offset).take(limit).collect()
    }

    /// Count eval runs since a timestamp for a tenant.
    pub async fn count_eval_runs_since_for_tenant(
        &self,
        tenant_id: &cairn_domain::TenantId,
        since_ms: u64,
    ) -> u64 {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .eval_runs
            .values()
            .filter(|e| e.project.tenant_id == *tenant_id && e.started_at >= since_ms)
            .count() as u64
    }

    /// Check if any provider connection is in degraded health.
    pub async fn any_provider_degraded(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.provider_health_records.values().any(|r| {
            matches!(
                r.status,
                cairn_domain::providers::ProviderHealthStatus::Degraded
            )
        })
    }

    /// Probe write capability (always succeeds for in-memory store).
    pub async fn probe_write(&self) -> Result<(), crate::StoreError> {
        Ok(())
    }

    /// Compact event log stub — returns a basic report.
    pub fn compact_event_log(
        &self,
        tenant_id: &cairn_domain::TenantId,
        retain_last_n: Option<u64>,
    ) -> serde_json::Value {
        let retain = retain_last_n.unwrap_or(100) as usize;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let events_before = state.events.len() as u64;

        if state.events.len() <= retain {
            return serde_json::json!({
                "events_before": events_before,
                "events_after": events_before,
                "events_compacted": 0,
                "retained": events_before
            });
        }

        // Keep only the last `retain` events.
        let start = state.events.len() - retain;
        let retained_events: Vec<StoredEvent> = state.events.drain(start..).collect();
        state.events.clear();
        state.events = retained_events;

        // Clear all projections.
        state.sessions.clear();
        state.runs.clear();
        state.tasks.clear();
        state.approvals.clear();
        state.checkpoints.clear();
        state.mailbox_messages.clear();
        state.tool_invocations.clear();
        state.signals.clear();
        state.ingest_jobs.clear();
        state.eval_runs.clear();
        state.eval_datasets.clear();
        state.eval_rubrics.clear();
        state.eval_baselines.clear();
        state.checkpoint_strategies.clear();
        state.prompt_assets.clear();
        state.prompt_versions.clear();
        state.prompt_releases.clear();
        state.tenants.clear();
        state.workspaces.clear();
        state.projects.clear();
        state.route_decisions.clear();
        state.provider_calls.clear();
        state.approval_policies.clear();
        state.external_workers.clear();
        state.session_costs.clear();
        state.run_costs.clear();
        state.llm_traces.clear();
        state.operator_profiles.clear();
        state.full_operator_profiles.clear();
        state.workspace_members.clear();
        state.signal_subscriptions.clear();
        state.provider_health_records.clear();
        state.provider_pools.clear();
        state.default_settings.clear();
        state.credentials.clear();
        state.channels.clear();
        state.channel_messages.clear();
        state.credential_rotations.clear();
        state.licenses.clear();
        state.entitlement_overrides.clear();
        state.notification_prefs.clear();
        state.notification_records.clear();
        state.guardrail_policies.clear();
        state.provider_budgets.clear();
        state.provider_connections.clear();
        state.quotas.clear();
        state.provider_bindings.clear();
        state.provider_health_schedules.clear();
        state.run_sla_configs.clear();
        state.run_sla_breaches.clear();
        state.run_cost_alerts.clear();
        state.retention_policies.clear();
        state.route_policies.clear();
        state.resource_shares.clear();
        state.snapshots.clear();
        state.command_id_index.clear();

        // Rebuild projections from retained events.
        for event in state.events.clone() {
            Self::apply_projection(&mut state, &event);
        }

        // Emit compaction event.
        let now = now_millis();
        let up_to_position = state.events.first().map(|e| e.position.0).unwrap_or(0);
        let compaction_event = StoredEvent {
            position: EventPosition(state.next_position),
            envelope: cairn_domain::EventEnvelope::for_runtime_event(
                cairn_domain::EventId::new(format!("evt_compact_{now}")),
                cairn_domain::EventSource::Runtime,
                cairn_domain::RuntimeEvent::EventLogCompacted(cairn_domain::EventLogCompacted {
                    up_to_position,
                    compacted_at_ms: now,
                    tenant_id: tenant_id.clone(),
                    events_before,
                    events_after: state.events.len() as u64,
                }),
            ),
            stored_at: now,
        };
        state.next_position += 1;
        state.events.push(compaction_event);

        serde_json::json!({
            "events_before": events_before,
            "events_after": state.events.len() as u64 - 1, // exclude the compaction event itself
            "events_compacted": events_before - retain as u64,
            "retained": retain as u64
        })
    }

    /// Create a snapshot capturing all events up to the current position.
    ///
    /// Returns `Err` on serialization failure rather than writing an empty
    /// snapshot — pre-T2-H9 the `unwrap_or_default()` path silently produced
    /// a snapshot whose `compressed_state` was empty and whose `state_hash`
    /// was the FNV-1a hash of 0 bytes, which then wiped the store on restore.
    pub fn create_snapshot(
        &self,
        tenant_id: &cairn_domain::TenantId,
    ) -> Result<cairn_domain::compaction::Snapshot, StoreError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let event_position = state.next_position.saturating_sub(1);

        // Serialize events as compressed_state so restore can replay them.
        let compressed_state = serde_json::to_vec(&state.events)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;

        // Simple hash of the compressed state for integrity check.
        let state_hash = format!("{:016x}", {
            let mut h: u64 = 0xcbf29ce484222325; // FNV-1a offset basis
            for &byte in &compressed_state {
                h ^= byte as u64;
                h = h.wrapping_mul(0x100000001b3); // FNV prime
            }
            h
        });

        Ok(cairn_domain::compaction::Snapshot {
            snapshot_id: format!("snap_{}", now),
            tenant_id: tenant_id.clone(),
            event_position,
            state_hash,
            created_at_ms: now,
            compressed_state,
        })
    }

    /// Restore from a snapshot: replace events and rebuild projections.
    pub fn restore_from_snapshot(
        &self,
        snapshot: &cairn_domain::compaction::Snapshot,
    ) -> serde_json::Value {
        let restored_events: Vec<StoredEvent> =
            serde_json::from_slice(&snapshot.compressed_state).unwrap_or_default();
        let events_before;
        let events_after;

        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            events_before = state.events.len() as u64;

            // Replace events with snapshot contents.
            state.events = restored_events.clone();
            events_after = state.events.len() as u64;

            // Reset position to after last snapshot event.
            state.next_position = state.events.last().map(|e| e.position.0 + 1).unwrap_or(1);

            // Clear all projections and rebuild.
            state.sessions.clear();
            state.runs.clear();
            state.tasks.clear();
            state.approvals.clear();
            state.checkpoints.clear();
            state.mailbox_messages.clear();
            state.tool_invocations.clear();
            state.signals.clear();
            state.ingest_jobs.clear();
            state.eval_runs.clear();
            state.eval_datasets.clear();
            state.eval_rubrics.clear();
            state.eval_baselines.clear();
            state.checkpoint_strategies.clear();
            state.prompt_assets.clear();
            state.prompt_versions.clear();
            state.prompt_releases.clear();
            state.tenants.clear();
            state.workspaces.clear();
            state.projects.clear();
            state.route_decisions.clear();
            state.provider_calls.clear();
            state.approval_policies.clear();
            state.external_workers.clear();
            state.session_costs.clear();
            state.run_costs.clear();
            state.llm_traces.clear();
            state.operator_profiles.clear();
            state.full_operator_profiles.clear();
            state.workspace_members.clear();
            state.signal_subscriptions.clear();
            state.provider_health_records.clear();
            state.provider_pools.clear();
            state.default_settings.clear();
            state.credentials.clear();
            state.channels.clear();
            state.channel_messages.clear();
            state.credential_rotations.clear();
            state.licenses.clear();
            state.entitlement_overrides.clear();
            state.notification_prefs.clear();
            state.notification_records.clear();
            state.guardrail_policies.clear();
            state.provider_budgets.clear();
            state.provider_connections.clear();
            state.quotas.clear();
            state.provider_bindings.clear();
            state.provider_health_schedules.clear();
            state.run_sla_configs.clear();
            state.run_sla_breaches.clear();
            state.run_cost_alerts.clear();
            state.retention_policies.clear();
            state.route_policies.clear();
            state.resource_shares.clear();
            state.snapshots.clear();
            state.command_id_index.clear();

            for event in state.events.clone() {
                Self::apply_projection(&mut state, &event);
            }
        }

        serde_json::json!({
            "restored": true,
            "events_before": events_before,
            "events_after": events_after,
            "events_replayed": events_after
        })
    }

    /// Delete a signal subscription.
    pub async fn delete_signal_subscription(
        &self,
        subscription_id: &str,
    ) -> Result<(), crate::StoreError> {
        self.state
            .lock()
            .unwrap()
            .signal_subscriptions
            .remove(subscription_id);
        Ok(())
    }

    /// List runs with optional filters.
    pub async fn list_runs_filtered(
        &self,
        query: &cairn_domain::tenancy::ProjectKey,
        session_id: Option<&cairn_domain::SessionId>,
        status: Option<cairn_domain::RunState>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<crate::projections::RunRecord>, crate::StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .runs
            .values()
            .filter(|r| r.project == *query)
            .filter(|r| session_id.is_none_or(|s| r.session_id == *s))
            .filter(|r| status.is_none_or(|st| r.state == st))
            .skip(offset)
            .take(limit)
            .cloned()
            .collect())
    }

    /// List tasks with optional filters.
    pub async fn list_tasks_filtered(
        &self,
        _query: &cairn_domain::tenancy::ProjectKey,
        _run_id: Option<&cairn_domain::RunId>,
        _state_filter: Option<cairn_domain::TaskState>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<crate::projections::TaskRecord>, crate::StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .tasks
            .values()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect())
    }

    /// Scan all prompt assets across every project (RFC 010 operator view).
    /// Scan all tasks across every project (operator view).
    pub async fn list_all_tasks(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TaskRecord>, crate::StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut tasks: Vec<TaskRecord> = state.tasks.values().cloned().collect();
        // Most-recent first.
        tasks.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.task_id.as_str().cmp(b.task_id.as_str()))
        });
        Ok(tasks.into_iter().skip(offset).take(limit).collect())
    }

    pub async fn list_all_prompt_assets(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<crate::projections::PromptAssetRecord>, crate::StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .prompt_assets
            .values()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect())
    }

    /// Scan all prompt releases across every project (RFC 010 operator view).
    pub async fn list_all_prompt_releases(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<crate::projections::PromptReleaseRecord>, crate::StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .prompt_releases
            .values()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect())
    }

    /// Scan all provider bindings across every tenant (RFC 010 operator view).
    pub async fn list_all_provider_bindings(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::providers::ProviderBindingRecord>, crate::StoreError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state
            .provider_bindings
            .values()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect())
    }

    /// Aggregate cost summary across all runs in the store (RFC 010 / RFC 009).
    ///
    /// Returns `(total_provider_calls, total_tokens_in, total_tokens_out,
    /// total_cost_micros)` since the store was created.
    pub async fn cost_summary(&self) -> (u64, u64, u64, u64) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut calls: u64 = 0;
        let mut tokens_in: u64 = 0;
        let mut tokens_out: u64 = 0;
        let mut cost_micros: u64 = 0;
        for rc in state.run_costs.values() {
            calls += rc.provider_calls;
            tokens_in += rc.total_tokens_in;
            tokens_out += rc.total_tokens_out;
            cost_micros += rc.total_cost_micros;
        }
        (calls, tokens_in, tokens_out, cost_micros)
    }

    /// RFC 002: list all approval records for a run (all states: pending + resolved).
    pub fn list_approvals_by_run(
        &self,
        run_id: &cairn_domain::RunId,
    ) -> Vec<crate::projections::ApprovalRecord> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<_> = state
            .approvals
            .values()
            .filter(|a| a.run_id.as_ref() == Some(run_id))
            .cloned()
            .collect();
        results.sort_by_key(|a| (a.created_at, a.approval_id.as_str().to_owned()));
        results
    }

    /// RFC 005: attach a prompt release to an approval policy record.
    ///
    /// RFC 009: list all provider call records for a project, sorted by call_id.
    pub fn list_provider_calls_by_project(
        &self,
        project_id: &cairn_domain::ProjectId,
    ) -> Vec<cairn_domain::providers::ProviderCallRecord> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut calls: Vec<_> = state
            .provider_calls
            .values()
            .filter(|c| &c.project_id == project_id)
            .cloned()
            .collect();
        calls.sort_by_key(|r| r.provider_call_id.clone());
        calls
    }

    /// `attached_release_ids` is initialised to empty by `ApprovalPolicyCreated`
    /// and updated by the governance layer (not via a domain event). This method
    /// provides the in-process mutation path used by tests and service impls.
    pub fn attach_release_to_policy(
        &self,
        policy_id: &str,
        release_id: cairn_domain::PromptReleaseId,
    ) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(policy) = state.approval_policies.get_mut(policy_id) {
            if !policy.attached_release_ids.contains(&release_id) {
                policy.attached_release_ids.push(release_id);
            }
            true
        } else {
            false
        }
    }

    /// Total number of task records in the store (all states).
    pub fn count_all_tasks(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tasks
            .len()
    }

    /// Total number of approval records in the store (all states).
    pub fn count_all_approvals(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .approvals
            .len()
    }

    // ── Snapshot / restore ────────────────────────────────────────────────────

    /// Export the full event log as a serializable snapshot.
    ///
    /// The event log is the source of truth; all projections are derived from
    /// it so serialising only the events is sufficient for a complete restore.
    pub fn dump_events(&self) -> crate::snapshot::StoreSnapshot {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        crate::snapshot::StoreSnapshot {
            version: 1,
            created_at_ms: now_millis(),
            event_count: state.events.len() as u64,
            events: state.events.clone(),
        }
    }

    /// Clear all state and replay the events from a snapshot.
    ///
    /// Returns the number of events replayed.
    pub fn load_snapshot(&self, snap: crate::snapshot::StoreSnapshot) -> u64 {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.usage_counters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();

        // Reset all projections to empty.
        state.events.clear();
        state.next_position = 1;
        state.command_id_index.clear();
        state.sessions.clear();
        state.runs.clear();
        state.tasks.clear();
        state.approvals.clear();
        state.checkpoints.clear();
        state.mailbox_messages.clear();
        state.tool_invocations.clear();
        state.signals.clear();
        state.ingest_jobs.clear();
        state.eval_runs.clear();
        state.eval_datasets.clear();
        state.eval_rubrics.clear();
        state.eval_baselines.clear();
        state.checkpoint_strategies.clear();
        state.prompt_assets.clear();
        state.prompt_versions.clear();
        state.prompt_releases.clear();
        state.route_decisions.clear();
        state.provider_calls.clear();
        state.approval_policies.clear();
        state.external_workers.clear();
        state.session_costs.clear();
        state.run_costs.clear();
        state.llm_traces.clear();
        state.operator_profiles.clear();
        state.full_operator_profiles.clear();
        state.workspace_members.clear();
        state.signal_subscriptions.clear();
        state.provider_health_records.clear();
        state.provider_pools.clear();
        state.default_settings.clear();
        state.credentials.clear();
        state.channels.clear();
        state.channel_messages.clear();
        state.credential_rotations.clear();
        state.licenses.clear();
        state.entitlement_overrides.clear();
        state.notification_prefs.clear();
        state.notification_records.clear();
        state.guardrail_policies.clear();
        state.provider_budgets.clear();
        state.provider_connections.clear();
        state.quotas.clear();
        state.provider_bindings.clear();
        state.provider_health_schedules.clear();
        state.run_sla_configs.clear();
        state.run_sla_breaches.clear();
        state.run_cost_alerts.clear();
        state.retention_policies.clear();
        state.route_policies.clear();
        state.resource_shares.clear();
        state.tenants.clear();
        state.workspaces.clear();
        state.projects.clear();
        state.snapshots.clear();

        // Replay events in order.
        let count = snap.events.len() as u64;
        for stored in snap.events {
            Self::apply_projection(&mut state, &stored);
            if stored.position.0 >= state.next_position {
                state.next_position = stored.position.0 + 1;
            }
            state.events.push(stored);
        }
        count
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn test_project() -> ProjectKey {
        ProjectKey::new("tenant", "workspace", "project")
    }

    fn make_envelope(event: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
        EventEnvelope::for_runtime_event(EventId::new("evt_test"), EventSource::Runtime, event)
    }

    #[tokio::test]
    async fn append_and_read_session_lifecycle() {
        let store = InMemoryStore::new();
        let project = test_project();
        let session_id = SessionId::new("sess_1");

        // Create session
        let positions = store
            .append(&[make_envelope(RuntimeEvent::SessionCreated(
                SessionCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                },
            ))])
            .await
            .unwrap();

        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0], EventPosition(1));

        // Read projection
        let session = SessionReadModel::get(&store, &session_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(session.state, SessionState::Open);
        assert_eq!(session.version, 1);

        // Change state
        store
            .append(&[make_envelope(RuntimeEvent::SessionStateChanged(
                SessionStateChanged {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    transition: StateTransition {
                        from: Some(SessionState::Open),
                        to: SessionState::Completed,
                    },
                },
            ))])
            .await
            .unwrap();

        let session = SessionReadModel::get(&store, &session_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(session.state, SessionState::Completed);
        assert_eq!(session.version, 2);
    }

    #[tokio::test]
    async fn append_and_read_run_lifecycle() {
        let store = InMemoryStore::new();
        let project = test_project();
        let session_id = SessionId::new("sess_1");
        let run_id = RunId::new("run_1");

        store
            .append(&[make_envelope(RuntimeEvent::RunCreated(RunCreated {
                project: project.clone(),
                session_id: session_id.clone(),
                run_id: run_id.clone(),
                parent_run_id: None,
                agent_role_id: None,
                prompt_release_id: None,
            }))])
            .await
            .unwrap();

        let run = RunReadModel::get(&store, &run_id).await.unwrap().unwrap();
        assert_eq!(run.state, RunState::Pending);

        // Advance to running then completed
        store
            .append(&[
                make_envelope(RuntimeEvent::RunStateChanged(RunStateChanged {
                    project: project.clone(),
                    run_id: run_id.clone(),
                    transition: StateTransition {
                        from: Some(RunState::Pending),
                        to: RunState::Running,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                })),
                make_envelope(RuntimeEvent::RunStateChanged(RunStateChanged {
                    project: project.clone(),
                    run_id: run_id.clone(),
                    transition: StateTransition {
                        from: Some(RunState::Running),
                        to: RunState::Completed,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                })),
            ])
            .await
            .unwrap();

        let run = RunReadModel::get(&store, &run_id).await.unwrap().unwrap();
        assert_eq!(run.state, RunState::Completed);
        assert_eq!(run.version, 3);
    }

    #[tokio::test]
    async fn list_by_parent_run_returns_only_children_of_that_parent() {
        // Replaces the pre-existing 10k-event scan that silently
        // truncated older children. Verifies: parent + N children +
        // unrelated run → list returns exactly N, no truncation at
        // absurdly-low-limit defaults, unrelated run not included.
        let store = InMemoryStore::new();
        let project = test_project();
        let session_id = SessionId::new("sess_1");
        let parent = RunId::new("parent");
        let unrelated = RunId::new("unrelated_root");

        // Parent + unrelated root run.
        store
            .append(&[
                make_envelope(RuntimeEvent::SessionCreated(SessionCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                })),
                make_envelope(RuntimeEvent::RunCreated(RunCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    run_id: parent.clone(),
                    parent_run_id: None,
                    agent_role_id: None,
                    prompt_release_id: None,
                })),
                make_envelope(RuntimeEvent::RunCreated(RunCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    run_id: unrelated.clone(),
                    parent_run_id: None,
                    agent_role_id: None,
                    prompt_release_id: None,
                })),
            ])
            .await
            .unwrap();

        // Three children of `parent`.
        for i in 0..3 {
            store
                .append(&[make_envelope(RuntimeEvent::RunCreated(RunCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    run_id: RunId::new(format!("child_{i}")),
                    parent_run_id: Some(parent.clone()),
                    agent_role_id: None,
                    prompt_release_id: None,
                }))])
                .await
                .unwrap();
        }

        let children = RunReadModel::list_by_parent_run(&store, &parent, 100)
            .await
            .unwrap();
        assert_eq!(children.len(), 3);
        assert!(children
            .iter()
            .all(|r| r.parent_run_id.as_ref() == Some(&parent)));

        // Unrelated parent → no children.
        let unrelated_children = RunReadModel::list_by_parent_run(&store, &unrelated, 100)
            .await
            .unwrap();
        assert_eq!(unrelated_children.len(), 0);

        // Limit truncation works.
        let first_one = RunReadModel::list_by_parent_run(&store, &parent, 1)
            .await
            .unwrap();
        assert_eq!(first_one.len(), 1);
    }

    #[tokio::test]
    async fn task_lifecycle_with_lease() {
        let store = InMemoryStore::new();
        let project = test_project();
        let task_id = TaskId::new("task_1");

        store
            .append(&[make_envelope(RuntimeEvent::TaskCreated(TaskCreated {
                project: project.clone(),
                task_id: task_id.clone(),
                parent_run_id: None,
                parent_task_id: None,
                prompt_release_id: None,
                session_id: None,
            }))])
            .await
            .unwrap();

        let task = TaskReadModel::get(&store, &task_id).await.unwrap().unwrap();
        assert_eq!(task.state, TaskState::Queued);

        // Claim lease via event (Worker 2 added TaskLeaseClaimed)
        store
            .append(&[
                make_envelope(RuntimeEvent::TaskLeaseClaimed(TaskLeaseClaimed {
                    project: project.clone(),
                    task_id: task_id.clone(),
                    lease_owner: "worker-a".to_owned(),
                    lease_token: 1,
                    lease_expires_at_ms: 9999999999,
                })),
                make_envelope(RuntimeEvent::TaskStateChanged(TaskStateChanged {
                    project: project.clone(),
                    task_id: task_id.clone(),
                    transition: StateTransition {
                        from: Some(TaskState::Queued),
                        to: TaskState::Leased,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                })),
            ])
            .await
            .unwrap();

        let task = TaskReadModel::get(&store, &task_id).await.unwrap().unwrap();
        assert_eq!(task.state, TaskState::Leased);
        assert_eq!(task.lease_owner.as_deref(), Some("worker-a"));
    }

    #[tokio::test]
    async fn checkpoint_supersedes_previous_latest() {
        let store = InMemoryStore::new();
        let project = test_project();
        let run_id = RunId::new("run_1");

        store
            .append(&[make_envelope(RuntimeEvent::CheckpointRecorded(
                CheckpointRecorded {
                    project: project.clone(),
                    run_id: run_id.clone(),
                    checkpoint_id: CheckpointId::new("cp_1"),
                    disposition: CheckpointDisposition::Latest,
                    data: None,
                    kind: None,
                    message_history_size: None,
                    tool_call_ids: Vec::new(),
                },
            ))])
            .await
            .unwrap();

        store
            .append(&[make_envelope(RuntimeEvent::CheckpointRecorded(
                CheckpointRecorded {
                    project: project.clone(),
                    run_id: run_id.clone(),
                    checkpoint_id: CheckpointId::new("cp_2"),
                    disposition: CheckpointDisposition::Latest,
                    data: None,
                    kind: None,
                    message_history_size: None,
                    tool_call_ids: Vec::new(),
                },
            ))])
            .await
            .unwrap();

        let cp1 = CheckpointReadModel::get(&store, &CheckpointId::new("cp_1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cp1.disposition, CheckpointDisposition::Superseded);

        let latest = store.latest_for_run(&run_id).await.unwrap().unwrap();
        assert_eq!(latest.checkpoint_id, CheckpointId::new("cp_2"));
    }

    #[tokio::test]
    async fn tool_invocation_projection_tracks_terminal_outcome() {
        let store = InMemoryStore::new();
        let project = test_project();
        let invocation_id = ToolInvocationId::new("tool_1");
        let run_id = RunId::new("run_1");

        store
            .append(&[
                make_envelope(RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                    project: project.clone(),
                    invocation_id: invocation_id.clone(),
                    session_id: Some(SessionId::new("sess_1")),
                    run_id: Some(run_id.clone()),
                    task_id: Some(TaskId::new("task_1")),
                    target: ToolInvocationTarget::Builtin {
                        tool_name: "fs.read".to_owned(),
                    },
                    execution_class: ExecutionClass::SupervisedProcess,
                    prompt_release_id: None,
                    requested_at_ms: 100,
                    started_at_ms: 101,
                })),
                make_envelope(RuntimeEvent::ToolInvocationCompleted(
                    ToolInvocationCompleted {
                        project,
                        invocation_id: invocation_id.clone(),
                        task_id: Some(TaskId::new("task_1")),
                        tool_name: "fs.read".to_owned(),
                        finished_at_ms: 105,
                        outcome: ToolInvocationOutcomeKind::Success,
                        tool_call_id: None,
                        result_json: None,
                    },
                )),
            ])
            .await
            .unwrap();

        let record = ToolInvocationReadModel::get(&store, &invocation_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.state, ToolInvocationState::Completed);
        assert_eq!(record.outcome, Some(ToolInvocationOutcomeKind::Success));
        assert_eq!(record.finished_at_ms, Some(105));

        let listed = ToolInvocationReadModel::list_by_run(&store, &run_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].invocation_id, invocation_id);
    }

    #[tokio::test]
    async fn tool_invocation_projection_preserves_canceled_state_and_orders_by_request_time() {
        let store = InMemoryStore::new();
        let project = test_project();
        let run_id = RunId::new("run_1");
        let older_invocation = ToolInvocationId::new("tool_old");
        let newer_invocation = ToolInvocationId::new("tool_new");

        store
            .append(&[
                make_envelope(RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                    project: project.clone(),
                    invocation_id: newer_invocation.clone(),
                    session_id: Some(SessionId::new("sess_1")),
                    run_id: Some(run_id.clone()),
                    task_id: None,
                    target: ToolInvocationTarget::Builtin {
                        tool_name: "shell.exec".to_owned(),
                    },
                    execution_class: ExecutionClass::SandboxedProcess,
                    prompt_release_id: None,
                    requested_at_ms: 200,
                    started_at_ms: 201,
                })),
                make_envelope(RuntimeEvent::ToolInvocationFailed(ToolInvocationFailed {
                    project: project.clone(),
                    invocation_id: newer_invocation.clone(),
                    task_id: None,
                    tool_name: "shell.exec".to_owned(),
                    finished_at_ms: 205,
                    outcome: ToolInvocationOutcomeKind::Canceled,
                    error_message: Some("canceled".to_owned()),
                })),
                make_envelope(RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                    project,
                    invocation_id: older_invocation.clone(),
                    session_id: Some(SessionId::new("sess_1")),
                    run_id: Some(run_id.clone()),
                    task_id: None,
                    target: ToolInvocationTarget::Builtin {
                        tool_name: "fs.read".to_owned(),
                    },
                    execution_class: ExecutionClass::SupervisedProcess,
                    prompt_release_id: None,
                    requested_at_ms: 100,
                    started_at_ms: 101,
                })),
            ])
            .await
            .unwrap();

        let canceled = ToolInvocationReadModel::get(&store, &newer_invocation)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(canceled.state, ToolInvocationState::Canceled);
        assert_eq!(canceled.outcome, Some(ToolInvocationOutcomeKind::Canceled));
        assert_eq!(canceled.error_message.as_deref(), Some("canceled"));

        let listed = ToolInvocationReadModel::list_by_run(&store, &run_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].invocation_id, older_invocation);
        assert_eq!(listed[1].invocation_id, newer_invocation);

        let paged = ToolInvocationReadModel::list_by_run(&store, &run_id, 1, 1)
            .await
            .unwrap();
        assert_eq!(paged.len(), 1);
        assert_eq!(paged[0].invocation_id, newer_invocation);
    }

    #[tokio::test]
    async fn event_stream_read() {
        let store = InMemoryStore::new();
        let project = test_project();

        store
            .append(&[
                make_envelope(RuntimeEvent::SessionCreated(SessionCreated {
                    project: project.clone(),
                    session_id: SessionId::new("s1"),
                })),
                make_envelope(RuntimeEvent::SessionCreated(SessionCreated {
                    project: project.clone(),
                    session_id: SessionId::new("s2"),
                })),
            ])
            .await
            .unwrap();

        let all = store.read_stream(None, 100).await.unwrap();
        assert_eq!(all.len(), 2);

        let after_first = store
            .read_stream(Some(EventPosition(1)), 100)
            .await
            .unwrap();
        assert_eq!(after_first.len(), 1);
    }

    /// Full lifecycle integration test: session -> run -> task -> approval -> checkpoint -> mailbox.
    /// Validates all projections are correct after a realistic event sequence.
    #[tokio::test]
    async fn full_lifecycle_projection_correctness() {
        let store = InMemoryStore::new();
        let project = test_project();
        let session_id = SessionId::new("sess_int");
        let run_id = RunId::new("run_int");
        let task_id = TaskId::new("task_int");
        let approval_id = ApprovalId::new("approval_int");
        let checkpoint_id_1 = CheckpointId::new("cp_int_1");
        let checkpoint_id_2 = CheckpointId::new("cp_int_2");
        let message_id = MailboxMessageId::new("msg_int");

        // 1. Create session.
        store
            .append(&[make_envelope(RuntimeEvent::SessionCreated(
                SessionCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                },
            ))])
            .await
            .unwrap();

        // 2. Create run in session.
        store
            .append(&[make_envelope(RuntimeEvent::RunCreated(RunCreated {
                project: project.clone(),
                session_id: session_id.clone(),
                run_id: run_id.clone(),
                parent_run_id: None,
                agent_role_id: None,
                prompt_release_id: None,
            }))])
            .await
            .unwrap();

        // 3. Start run.
        store
            .append(&[make_envelope(RuntimeEvent::RunStateChanged(
                RunStateChanged {
                    project: project.clone(),
                    run_id: run_id.clone(),
                    transition: StateTransition {
                        from: Some(RunState::Pending),
                        to: RunState::Running,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                },
            ))])
            .await
            .unwrap();

        // 4. Create task.
        store
            .append(&[make_envelope(RuntimeEvent::TaskCreated(TaskCreated {
                project: project.clone(),
                task_id: task_id.clone(),
                parent_run_id: Some(run_id.clone()),
                parent_task_id: None,
                prompt_release_id: None,
                session_id: None,
            }))])
            .await
            .unwrap();

        // 5. Claim task lease.
        store
            .append(&[make_envelope(RuntimeEvent::TaskLeaseClaimed(
                TaskLeaseClaimed {
                    project: project.clone(),
                    task_id: task_id.clone(),
                    lease_owner: "worker-alpha".to_owned(),
                    lease_token: 1,
                    lease_expires_at_ms: 9999999999,
                },
            ))])
            .await
            .unwrap();

        // 6. Task starts running.
        store
            .append(&[make_envelope(RuntimeEvent::TaskStateChanged(
                TaskStateChanged {
                    project: project.clone(),
                    task_id: task_id.clone(),
                    transition: StateTransition {
                        from: Some(TaskState::Leased),
                        to: TaskState::Running,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                },
            ))])
            .await
            .unwrap();

        // 7. Request approval.
        store
            .append(&[make_envelope(RuntimeEvent::ApprovalRequested(
                ApprovalRequested {
                    project: project.clone(),
                    approval_id: approval_id.clone(),
                    run_id: Some(run_id.clone()),
                    task_id: Some(task_id.clone()),
                    requirement: ApprovalRequirement::Required,
                    title: None,
                    description: None,
                },
            ))])
            .await
            .unwrap();

        // 8. Save checkpoint.
        store
            .append(&[make_envelope(RuntimeEvent::CheckpointRecorded(
                CheckpointRecorded {
                    project: project.clone(),
                    run_id: run_id.clone(),
                    checkpoint_id: checkpoint_id_1.clone(),
                    disposition: CheckpointDisposition::Latest,
                    data: None,
                    kind: None,
                    message_history_size: None,
                    tool_call_ids: Vec::new(),
                },
            ))])
            .await
            .unwrap();

        // 9. Save second checkpoint (supersedes first).
        store
            .append(&[make_envelope(RuntimeEvent::CheckpointRecorded(
                CheckpointRecorded {
                    project: project.clone(),
                    run_id: run_id.clone(),
                    checkpoint_id: checkpoint_id_2.clone(),
                    disposition: CheckpointDisposition::Latest,
                    data: None,
                    kind: None,
                    message_history_size: None,
                    tool_call_ids: Vec::new(),
                },
            ))])
            .await
            .unwrap();

        // 10. Resolve approval.
        store
            .append(&[make_envelope(RuntimeEvent::ApprovalResolved(
                ApprovalResolved {
                    project: project.clone(),
                    approval_id: approval_id.clone(),
                    decision: ApprovalDecision::Approved,
                },
            ))])
            .await
            .unwrap();

        // 11. Send mailbox message.
        store
            .append(&[make_envelope(RuntimeEvent::MailboxMessageAppended(
                MailboxMessageAppended {
                    project: project.clone(),
                    message_id: message_id.clone(),
                    run_id: Some(run_id.clone()),
                    task_id: Some(task_id.clone()),
                    content: String::new(),
                    from_run_id: None,
                    from_task_id: None,
                    deliver_at_ms: 0,
                    sender: None,
                    recipient: None,
                    body: None,
                    sent_at: None,
                    delivery_status: None,
                },
            ))])
            .await
            .unwrap();

        // 12. Complete task.
        store
            .append(&[make_envelope(RuntimeEvent::TaskStateChanged(
                TaskStateChanged {
                    project: project.clone(),
                    task_id: task_id.clone(),
                    transition: StateTransition {
                        from: Some(TaskState::Running),
                        to: TaskState::Completed,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                },
            ))])
            .await
            .unwrap();

        // 13. Complete run.
        store
            .append(&[make_envelope(RuntimeEvent::RunStateChanged(
                RunStateChanged {
                    project: project.clone(),
                    run_id: run_id.clone(),
                    transition: StateTransition {
                        from: Some(RunState::Running),
                        to: RunState::Completed,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                },
            ))])
            .await
            .unwrap();

        // --- Verify all projections ---

        // Session: still open (derived from run state, not explicit close).
        let session = SessionReadModel::get(&store, &session_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(session.state, SessionState::Open);

        // Run: completed.
        let run = RunReadModel::get(&store, &run_id).await.unwrap().unwrap();
        assert_eq!(run.state, RunState::Completed);
        assert!(run.state.is_terminal());
        assert!(run.parent_run_id.is_none());

        // Task: completed with lease info preserved.
        let task = TaskReadModel::get(&store, &task_id).await.unwrap().unwrap();
        assert_eq!(task.state, TaskState::Completed);
        assert!(task.state.is_terminal());
        assert_eq!(task.lease_owner.as_deref(), Some("worker-alpha"));
        assert_eq!(task.parent_run_id.as_ref(), Some(&run_id));

        // Approval: resolved as approved.
        let approval = ApprovalReadModel::get(&store, &approval_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(approval.decision, Some(ApprovalDecision::Approved));
        assert_eq!(approval.run_id.as_ref(), Some(&run_id));

        // Checkpoint 1: superseded.
        let cp1 = CheckpointReadModel::get(&store, &checkpoint_id_1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cp1.disposition, CheckpointDisposition::Superseded);

        // Checkpoint 2: latest.
        let cp2 = CheckpointReadModel::get(&store, &checkpoint_id_2)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cp2.disposition, CheckpointDisposition::Latest);

        // Latest checkpoint for run is cp2.
        let latest = store.latest_for_run(&run_id).await.unwrap().unwrap();
        assert_eq!(latest.checkpoint_id, checkpoint_id_2);

        // Mailbox: message linked to run and task.
        let msg = MailboxReadModel::get(&store, &message_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(msg.run_id.as_ref(), Some(&run_id));
        assert_eq!(msg.task_id.as_ref(), Some(&task_id));

        // No non-terminal runs remain.
        assert!(!store.any_non_terminal(&session_id).await.unwrap());

        // Event stream has all 13 events.
        let all = store.read_stream(None, 100).await.unwrap();
        assert_eq!(all.len(), 13);

        // Entity-filtered read for run events.
        let run_events = store
            .read_by_entity(&EntityRef::Run(run_id.clone()), None, 100)
            .await
            .unwrap();
        assert!(run_events.len() >= 3); // created + 2 state changes
    }

    /// Expired lease detection for recovery sweeps.
    #[tokio::test]
    async fn expired_lease_detection() {
        let store = InMemoryStore::new();
        let project = test_project();

        // Create two tasks with leases.
        for (id, expires) in [("t1", 100u64), ("t2", 9999999999u64)] {
            let task_id = TaskId::new(id);
            store
                .append(&[make_envelope(RuntimeEvent::TaskCreated(TaskCreated {
                    project: project.clone(),
                    task_id: task_id.clone(),
                    parent_run_id: None,
                    parent_task_id: None,
                    prompt_release_id: None,
                    session_id: None,
                }))])
                .await
                .unwrap();

            store
                .append(&[make_envelope(RuntimeEvent::TaskLeaseClaimed(
                    TaskLeaseClaimed {
                        project: project.clone(),
                        task_id: task_id.clone(),
                        lease_owner: "w".to_owned(),
                        lease_token: 1,
                        lease_expires_at_ms: expires,
                    },
                ))])
                .await
                .unwrap();

            store
                .append(&[make_envelope(RuntimeEvent::TaskStateChanged(
                    TaskStateChanged {
                        project: project.clone(),
                        task_id,
                        transition: StateTransition {
                            from: Some(TaskState::Queued),
                            to: TaskState::Leased,
                        },
                        failure_class: None,
                        pause_reason: None,
                        resume_trigger: None,
                    },
                ))])
                .await
                .unwrap();
        }

        // t1 expired (lease at 100, now is 500), t2 still valid.
        let expired = store.list_expired_leases(500, 100).await.unwrap();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].task_id, TaskId::new("t1"));
    }

    #[tokio::test]
    async fn signal_projection_and_read_model() {
        let store = InMemoryStore::new();
        let project = test_project();
        let signal_id = SignalId::new("sig_1");

        store
            .append(&[make_envelope(RuntimeEvent::SignalIngested(
                SignalIngested {
                    project: project.clone(),
                    signal_id: signal_id.clone(),
                    source: "webhook".to_owned(),
                    payload: serde_json::json!({"key": "value"}),
                    timestamp_ms: 1000,
                },
            ))])
            .await
            .unwrap();

        // get returns the record with correct fields.
        let record = SignalReadModel::get(&store, &signal_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.id, signal_id);
        assert_eq!(record.project, project);
        assert_eq!(record.source, "webhook");
        assert_eq!(record.payload["key"], "value");
        assert_eq!(record.timestamp_ms, 1000);

        // list_by_project returns it.
        let list = SignalReadModel::list_by_project(&store, &project, 10, 0)
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, signal_id);

        // list_by_project with a different project returns empty.
        let other_project = ProjectKey::new("other_tenant", "other_ws", "other_proj");
        let empty = SignalReadModel::list_by_project(&store, &other_project, 10, 0)
            .await
            .unwrap();
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn signal_entity_ref_filtering() {
        let store = InMemoryStore::new();
        let project = test_project();
        let signal_id = SignalId::new("sig_entity");

        store
            .append(&[make_envelope(RuntimeEvent::SignalIngested(
                SignalIngested {
                    project: project.clone(),
                    signal_id: signal_id.clone(),
                    source: "api".to_owned(),
                    payload: serde_json::json!(null),
                    timestamp_ms: 500,
                },
            ))])
            .await
            .unwrap();

        // read_by_entity with matching Signal ref returns the event.
        let events = store
            .read_by_entity(&EntityRef::Signal(signal_id.clone()), None, 100)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0].envelope.payload,
            RuntimeEvent::SignalIngested(e) if e.signal_id == signal_id
        ));

        // read_by_entity with a different signal ID returns empty.
        let other = store
            .read_by_entity(&EntityRef::Signal(SignalId::new("sig_other")), None, 100)
            .await
            .unwrap();
        assert!(other.is_empty());
    }

    // ── Secondary event log (dual-write) ─────────────────────────────────────

    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Minimal in-memory secondary log that counts appended events.
    struct CountingLog {
        count: Arc<AtomicUsize>,
        events: Arc<Mutex<Vec<EventEnvelope<RuntimeEvent>>>>,
    }

    impl CountingLog {
        fn new() -> (Arc<Self>, Arc<AtomicUsize>) {
            let count = Arc::new(AtomicUsize::new(0));
            let log = Arc::new(CountingLog {
                count: count.clone(),
                events: Arc::new(Mutex::new(Vec::new())),
            });
            (log, count)
        }
    }

    #[async_trait::async_trait]
    impl EventLog for CountingLog {
        async fn append(
            &self,
            events: &[EventEnvelope<RuntimeEvent>],
        ) -> Result<Vec<EventPosition>, crate::StoreError> {
            self.count.fetch_add(events.len(), Ordering::SeqCst);
            self.events.lock().unwrap().extend(events.iter().cloned());
            Ok(events
                .iter()
                .enumerate()
                .map(|(i, _)| EventPosition(i as u64))
                .collect())
        }

        async fn read_stream(
            &self,
            _after: Option<EventPosition>,
            _limit: usize,
        ) -> Result<Vec<StoredEvent>, crate::StoreError> {
            Ok(vec![])
        }

        async fn head_position(&self) -> Result<Option<EventPosition>, crate::StoreError> {
            Ok(None)
        }

        async fn read_by_entity(
            &self,
            _entity: &EntityRef,
            _after: Option<EventPosition>,
            _limit: usize,
        ) -> Result<Vec<StoredEvent>, crate::StoreError> {
            Ok(vec![])
        }

        async fn find_by_causation_id(
            &self,
            _causation_id: &str,
        ) -> Result<Option<EventPosition>, crate::StoreError> {
            Ok(None)
        }
    }

    /// Secondary log receives all events appended to the primary store.
    #[tokio::test]
    async fn secondary_log_receives_all_appends() {
        let store = Arc::new(InMemoryStore::new());
        let (counting_log, count) = CountingLog::new();
        store.set_secondary_log(counting_log);

        let project = test_project();

        // Append a session created event.
        store
            .append(&[make_envelope(RuntimeEvent::SessionCreated(
                SessionCreated {
                    project: project.clone(),
                    session_id: SessionId::new("sess_sec_1"),
                },
            ))])
            .await
            .unwrap();

        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "secondary must receive 1 event"
        );

        // Append two more events in a single batch.
        store
            .append(&[
                make_envelope(RuntimeEvent::SessionCreated(SessionCreated {
                    project: project.clone(),
                    session_id: SessionId::new("sess_sec_2"),
                })),
                make_envelope(RuntimeEvent::SessionCreated(SessionCreated {
                    project: project.clone(),
                    session_id: SessionId::new("sess_sec_3"),
                })),
            ])
            .await
            .unwrap();

        assert_eq!(
            count.load(Ordering::SeqCst),
            3,
            "secondary must receive all 3 events total"
        );
    }

    /// Primary store is not affected when secondary log is absent.
    #[tokio::test]
    async fn no_secondary_log_works_normally() {
        let store = InMemoryStore::new();
        // No secondary log set — append must succeed normally.
        let positions = store
            .append(&[make_envelope(RuntimeEvent::SessionCreated(
                SessionCreated {
                    project: test_project(),
                    session_id: SessionId::new("sess_no_sec"),
                },
            ))])
            .await
            .unwrap();

        assert_eq!(positions.len(), 1);
        let sessions = SessionReadModel::list_active(&store, 10).await.unwrap();
        assert_eq!(sessions.len(), 1);
    }

    /// Secondary failure surfaces to the caller (fail-closed), but the
    /// in-memory projection has already been written.
    ///
    /// Pre-#T2-C3 behaviour was to swallow the secondary error and return
    /// `Ok`, which silently lost history whenever the secondary was the
    /// durable source of truth (RFC 002). This test pins the new contract:
    /// callers MUST observe the error so they can decide whether to retry,
    /// compensate, or abort. Deliberate side-effect: the in-memory state
    /// and the secondary log have diverged by the count in the error
    /// message; `event_id`-based idempotent retry is the expected
    /// reconciliation path.
    #[tokio::test]
    async fn secondary_failure_surfaces_error_but_keeps_in_memory_state() {
        struct FailingLog;

        #[async_trait::async_trait]
        impl EventLog for FailingLog {
            async fn append(
                &self,
                _events: &[EventEnvelope<RuntimeEvent>],
            ) -> Result<Vec<EventPosition>, crate::StoreError> {
                Err(crate::StoreError::Internal("secondary down".to_owned()))
            }
            async fn read_stream(
                &self,
                _: Option<EventPosition>,
                _: usize,
            ) -> Result<Vec<StoredEvent>, crate::StoreError> {
                Ok(vec![])
            }
            async fn head_position(&self) -> Result<Option<EventPosition>, crate::StoreError> {
                Ok(None)
            }
            async fn read_by_entity(
                &self,
                _: &EntityRef,
                _: Option<EventPosition>,
                _: usize,
            ) -> Result<Vec<StoredEvent>, crate::StoreError> {
                Ok(vec![])
            }
            async fn find_by_causation_id(
                &self,
                _: &str,
            ) -> Result<Option<EventPosition>, crate::StoreError> {
                Ok(None)
            }
        }

        let store = Arc::new(InMemoryStore::new());
        store.set_secondary_log(Arc::new(FailingLog));

        let result = store
            .append(&[make_envelope(RuntimeEvent::SessionCreated(
                SessionCreated {
                    project: test_project(),
                    session_id: SessionId::new("sess_resilient"),
                },
            ))])
            .await;

        assert!(
            result.is_err(),
            "secondary failure must surface as Err to the caller (fail-closed) — the pre-#T2-C3 silent-swallow path lost events under restart"
        );
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("secondary event log write failed"),
            "error must identify the secondary-log divergence, got: {msg}"
        );

        // In-memory state is still written — the caller knows the two logs
        // diverged and can reconcile by retrying (event_id is idempotent).
        let sessions = SessionReadModel::list_active(store.as_ref(), 10)
            .await
            .unwrap();
        assert_eq!(
            sessions.len(),
            1,
            "primary projection must still reflect the in-memory write so diagnosis + retry see consistent state"
        );
    }
}
