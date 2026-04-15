//! Run, event, and utility helper functions.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::IntoResponse;

use cairn_api::feed::FeedItem;
use cairn_domain::workers::{ExternalWorkerProgress, ExternalWorkerRecord, ExternalWorkerReport};
use cairn_domain::{
    ApprovalId, CheckpointId, EventEnvelope, ProjectKey, RunId, RunState, RuntimeEvent, Scope,
    TaskId, TaskState, TenantId, ToolInvocationId, WorkerId,
};
use cairn_runtime::{DefaultsService, RunService};
use cairn_store::projections::{RunReadModel, RunRecord, TaskReadModel};
use cairn_store::{EntityRef, EventLog, EventPosition, StoredEvent};

use crate::default_repo_sandbox_policy;
use crate::errors::{
    now_ms, operator_event_envelope, runtime_error_response, store_error_response, AppApiError,
};
use crate::extractors::TenantScope;
use crate::state::{AppState, MailboxMessageView};

// ── Shared DTOs used across multiple handlers ───────────────────────────────

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct RunRecordView {
    #[serde(flatten)]
    pub(crate) run: RunRecord,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) mode: Option<cairn_domain::decisions::RunMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) created_by_trigger_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sandbox_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sandbox_path: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecoveryStatusResponse {
    pub(crate) run_id: String,
    pub(crate) last_attempt_reason: Option<String>,
    pub(crate) last_recovered: Option<bool>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct ActivityEntry {
    #[serde(rename = "type")]
    pub(crate) entry_type: String,
    pub(crate) timestamp_ms: u64,
    pub(crate) run_id: Option<String>,
    pub(crate) task_id: Option<String>,
    pub(crate) state: Option<String>,
    pub(crate) description: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct ReplayTaskStateView {
    pub(crate) task_id: String,
    pub(crate) state: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct ReplayResult {
    pub(crate) events_replayed: u32,
    pub(crate) final_run_state: Option<String>,
    pub(crate) final_task_states: Vec<ReplayTaskStateView>,
    pub(crate) checkpoints_found: u32,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct DiagnosedTaskActivity {
    pub(crate) task_id: String,
    pub(crate) state: TaskState,
    pub(crate) last_activity_ms: u64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct DiagnosisReport {
    pub(crate) run_id: String,
    pub(crate) state: RunState,
    pub(crate) duration_ms: u64,
    pub(crate) active_tasks: Vec<DiagnosedTaskActivity>,
    pub(crate) stalled_tasks: Vec<String>,
    pub(crate) last_event_type: String,
    pub(crate) last_event_ms: u64,
    pub(crate) suggested_action: String,
}

// ---------------------------------------------------------------------------
// Run helpers
// ---------------------------------------------------------------------------

pub(crate) async fn working_dir_for_run(
    state: &AppState,
    run: &RunRecord,
) -> Result<PathBuf, cairn_workspace::WorkspaceError> {
    let repo_ctx = cairn_domain::RepoAccessContext {
        project: run.project.clone(),
    };
    let repo_ids = state.project_repo_access.list_for_project(&repo_ctx).await;
    let Some(repo_id) = repo_ids.first().cloned() else {
        // No repo allowlisted — create an isolated ephemeral directory for
        // this run.  This is expected for API-driven orchestration where the
        // agent works on external systems (APIs, infra) and doesn't need a
        // repo checkout.  We NEVER fall back to the server process CWD
        // because that would expose cairn's own filesystem to agent tools.
        let ephemeral = std::env::temp_dir()
            .join("cairn-runs")
            .join(run.run_id.as_str());
        if let Err(e) = std::fs::create_dir_all(&ephemeral) {
            tracing::warn!(
                run_id = %run.run_id,
                path = %ephemeral.display(),
                error = %e,
                "failed to create ephemeral run directory; falling back to temp root"
            );
            return Ok(std::env::temp_dir().join("cairn-runs"));
        }
        tracing::debug!(
            run_id = %run.run_id,
            path = %ephemeral.display(),
            "no repo allowlisted for project; using ephemeral run directory"
        );
        return Ok(ephemeral);
    };

    if repo_ids.len() > 1 {
        tracing::warn!(
            run_id = %run.run_id,
            project = ?run.project,
            selected_repo = %repo_id,
            repo_count = repo_ids.len(),
            "multiple repos allowlisted for run; provisioning sandbox from the first sorted repo"
        );
    }

    state
        .repo_clone_cache
        .ensure_cloned(&run.project.tenant_id, &repo_id)
        .await?;

    state
        .sandbox_service
        .provision_or_reconnect(
            &run.run_id,
            None,
            run.project.clone(),
            default_repo_sandbox_policy(repo_id),
        )
        .await?;

    let sandbox = state.sandbox_service.activate(&run.run_id, None).await?;
    Ok(sandbox.path)
}

pub(crate) fn run_default_key(run_id: &RunId, suffix: &str) -> String {
    format!("run:{}:{suffix}", run_id.as_str())
}

pub(crate) async fn resolve_run_string_default(
    state: &AppState,
    project: &ProjectKey,
    run_id: &RunId,
    suffix: &str,
) -> Option<String> {
    let key = run_default_key(run_id, suffix);
    state
        .runtime
        .defaults
        .resolve(project, &key)
        .await
        .ok()
        .flatten()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
}

pub(crate) async fn resolve_run_mode_default(
    state: &AppState,
    project: &ProjectKey,
    run_id: &RunId,
) -> Option<cairn_domain::decisions::RunMode> {
    let key = run_default_key(run_id, "run_mode");
    state
        .runtime
        .defaults
        .resolve(project, &key)
        .await
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_value(value).ok())
}

pub(crate) async fn persist_run_mode_default(
    state: &AppState,
    project: &ProjectKey,
    run_id: &RunId,
    mode: &cairn_domain::decisions::RunMode,
) -> Result<(), cairn_runtime::RuntimeError> {
    state
        .runtime
        .defaults
        .set(
            cairn_domain::tenancy::Scope::Project,
            project.project_id.to_string(),
            run_default_key(run_id, "run_mode"),
            serde_json::to_value(mode).unwrap_or(serde_json::Value::Null),
        )
        .await
        .map(|_| ())
}

pub(crate) async fn build_run_record_view(state: &AppState, run: RunRecord) -> RunRecordView {
    let created_by_trigger_id =
        resolve_run_string_default(state, &run.project, &run.run_id, "created_by_trigger_id").await;
    let mode = resolve_run_mode_default(state, &run.project, &run.run_id).await;
    let sandbox_id =
        resolve_run_string_default(state, &run.project, &run.run_id, "sandbox_id").await;
    let sandbox_path =
        resolve_run_string_default(state, &run.project, &run.run_id, "sandbox_path").await;

    RunRecordView {
        run,
        mode,
        created_by_trigger_id,
        sandbox_id,
        sandbox_path,
    }
}

pub(crate) async fn load_run_visible_to_tenant(
    state: &AppState,
    tenant_scope: &TenantScope,
    run_id: &RunId,
) -> Result<Option<RunRecord>, axum::response::Response> {
    match state.runtime.runs.get(run_id).await {
        Ok(Some(run))
            if tenant_scope.is_admin || run.project.tenant_id == *tenant_scope.tenant_id() =>
        {
            return Ok(Some(run));
        }
        Ok(Some(_)) => return Ok(None),
        Ok(None) => {}
        Err(err) => return Err(runtime_error_response(err)),
    }

    let events = match state
        .runtime
        .store
        .read_by_entity(&EntityRef::Run(run_id.clone()), None, 1_000)
        .await
    {
        Ok(events) => events,
        Err(err) => return Err(store_error_response(err)),
    };

    let mut reconstructed: Option<RunRecord> = None;
    for stored in events {
        match stored.envelope.payload {
            RuntimeEvent::RunCreated(created) if created.run_id == *run_id => {
                reconstructed = Some(RunRecord {
                    run_id: created.run_id.clone(),
                    session_id: created.session_id.clone(),
                    parent_run_id: created.parent_run_id.clone(),
                    project: created.project.clone(),
                    state: RunState::Pending,
                    prompt_release_id: created.prompt_release_id.clone(),
                    agent_role_id: created.agent_role_id.clone(),
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                    version: 1,
                    created_at: stored.stored_at,
                    updated_at: stored.stored_at,
                });
            }
            RuntimeEvent::RunStateChanged(change) if change.run_id == *run_id => {
                if let Some(run) = reconstructed.as_mut() {
                    run.state = change.transition.to;
                    run.failure_class = change.failure_class;
                    run.pause_reason = change.pause_reason;
                    run.resume_trigger = change.resume_trigger;
                    run.version += 1;
                    run.updated_at = stored.stored_at;
                }
            }
            _ => {}
        }
    }

    Ok(reconstructed
        .filter(|run| tenant_scope.is_admin || run.project.tenant_id == *tenant_scope.tenant_id()))
}

// ---------------------------------------------------------------------------
// Event helpers
// ---------------------------------------------------------------------------

pub(crate) fn event_relates_to_run(
    event: &RuntimeEvent,
    run_id: &RunId,
    tracked_tasks: &mut HashSet<TaskId>,
    tracked_approvals: &mut HashSet<ApprovalId>,
    tracked_invocations: &mut HashSet<ToolInvocationId>,
) -> bool {
    match event {
        RuntimeEvent::RunCreated(run) => run.run_id == *run_id,
        RuntimeEvent::RunStateChanged(run) => run.run_id == *run_id,
        RuntimeEvent::OperatorIntervention(intervention) => {
            intervention.run_id.as_ref() == Some(run_id)
        }
        RuntimeEvent::TaskCreated(task) => {
            let matches = task.parent_run_id.as_ref() == Some(run_id);
            if matches {
                tracked_tasks.insert(task.task_id.clone());
            }
            matches
        }
        RuntimeEvent::TaskLeaseClaimed(task) => tracked_tasks.contains(&task.task_id),
        RuntimeEvent::TaskLeaseHeartbeated(task) => tracked_tasks.contains(&task.task_id),
        RuntimeEvent::TaskStateChanged(task) => tracked_tasks.contains(&task.task_id),
        RuntimeEvent::TaskDependencyAdded(task) => {
            let matches = tracked_tasks.contains(&task.dependent_task_id)
                || tracked_tasks.contains(&task.depends_on_task_id);
            if matches {
                tracked_tasks.insert(task.dependent_task_id.clone());
                tracked_tasks.insert(task.depends_on_task_id.clone());
            }
            matches
        }
        RuntimeEvent::TaskDependencyResolved(task) => {
            tracked_tasks.contains(&task.dependent_task_id)
                || tracked_tasks.contains(&task.depends_on_task_id)
        }
        RuntimeEvent::ApprovalRequested(approval) => {
            let matches = approval.run_id.as_ref() == Some(run_id)
                || approval
                    .task_id
                    .as_ref()
                    .is_some_and(|task_id| tracked_tasks.contains(task_id));
            if matches {
                tracked_approvals.insert(approval.approval_id.clone());
            }
            matches
        }
        RuntimeEvent::ApprovalResolved(approval) => {
            tracked_approvals.contains(&approval.approval_id)
        }
        RuntimeEvent::ApprovalDelegated(approval) => {
            tracked_approvals.contains(&approval.approval_id)
        }
        RuntimeEvent::CheckpointRecorded(checkpoint) => checkpoint.run_id == *run_id,
        RuntimeEvent::CheckpointStrategySet(strategy) => strategy.run_id.as_ref() == Some(run_id),
        RuntimeEvent::CheckpointRestored(checkpoint) => checkpoint.run_id == *run_id,
        RuntimeEvent::MailboxMessageAppended(message) => {
            message.run_id.as_ref() == Some(run_id)
                || message
                    .task_id
                    .as_ref()
                    .is_some_and(|task_id| tracked_tasks.contains(task_id))
        }
        RuntimeEvent::ToolInvocationStarted(invocation) => {
            let matches = invocation.run_id.as_ref() == Some(run_id)
                || invocation
                    .task_id
                    .as_ref()
                    .is_some_and(|task_id| tracked_tasks.contains(task_id));
            if matches {
                tracked_invocations.insert(invocation.invocation_id.clone());
            }
            matches
        }
        RuntimeEvent::PermissionDecisionRecorded(invocation) => invocation
            .invocation_id
            .as_deref()
            .map(|id| tracked_invocations.contains(&ToolInvocationId::new(id)))
            .unwrap_or(false),
        RuntimeEvent::ToolInvocationProgressUpdated(invocation) => {
            tracked_invocations.contains(&invocation.invocation_id)
        }
        RuntimeEvent::ToolInvocationCompleted(invocation) => {
            tracked_invocations.contains(&invocation.invocation_id)
                || invocation
                    .task_id
                    .as_ref()
                    .is_some_and(|task_id| tracked_tasks.contains(task_id))
        }
        RuntimeEvent::ToolInvocationFailed(invocation) => {
            tracked_invocations.contains(&invocation.invocation_id)
                || invocation
                    .task_id
                    .as_ref()
                    .is_some_and(|task_id| tracked_tasks.contains(task_id))
        }
        RuntimeEvent::ExternalWorkerReported(report) => {
            report.report.run_id.as_ref() == Some(run_id)
                || tracked_tasks.contains(&report.report.task_id)
        }
        RuntimeEvent::SubagentSpawned(spawned) => spawned.parent_run_id == *run_id,
        RuntimeEvent::RecoveryAttempted(recovery) => {
            recovery.run_id.as_ref() == Some(run_id)
                || recovery
                    .task_id
                    .as_ref()
                    .is_some_and(|task_id| tracked_tasks.contains(task_id))
        }
        RuntimeEvent::RecoveryCompleted(recovery) => {
            recovery.run_id.as_ref() == Some(run_id)
                || recovery
                    .task_id
                    .as_ref()
                    .is_some_and(|task_id| tracked_tasks.contains(task_id))
        }
        RuntimeEvent::UserMessageAppended(message) => message.run_id == *run_id,
        RuntimeEvent::ProviderCallCompleted(call) => call.run_id.as_ref() == Some(run_id),
        RuntimeEvent::RunCostUpdated(cost) => cost.run_id == *run_id,
        _ => false,
    }
}

pub(crate) fn event_is_replay_relevant(event: &RuntimeEvent) -> bool {
    !matches!(
        event,
        RuntimeEvent::SessionCostUpdated(_)
            | RuntimeEvent::RunCostUpdated(_)
            | RuntimeEvent::ProviderBudgetSet(_)
            | RuntimeEvent::ProviderBudgetAlertTriggered(_)
            | RuntimeEvent::ProviderBudgetExceeded(_)
    )
}

pub(crate) fn task_activity_task_id(event: &RuntimeEvent) -> Option<&TaskId> {
    match event {
        RuntimeEvent::TaskCreated(task) => Some(&task.task_id),
        RuntimeEvent::TaskLeaseClaimed(task) => Some(&task.task_id),
        RuntimeEvent::TaskLeaseHeartbeated(task) => Some(&task.task_id),
        RuntimeEvent::TaskStateChanged(task) => Some(&task.task_id),
        RuntimeEvent::ExternalWorkerReported(report) => Some(&report.report.task_id),
        _ => None,
    }
}

pub(crate) async fn collect_run_events(
    state: &AppState,
    run_id: &RunId,
) -> Result<Vec<StoredEvent>, cairn_store::StoreError> {
    let current_tasks =
        TaskReadModel::list_by_parent_run(state.runtime.store.as_ref(), run_id, 1_000).await?;
    let mut tracked_tasks: HashSet<TaskId> =
        current_tasks.into_iter().map(|task| task.task_id).collect();
    let mut tracked_approvals: HashSet<ApprovalId> = HashSet::new();
    let mut tracked_invocations: HashSet<ToolInvocationId> = HashSet::new();

    let mut cursor = None;
    let mut related = Vec::new();

    loop {
        let batch = state.runtime.store.read_stream(cursor, 512).await?;
        if batch.is_empty() {
            break;
        }

        for stored in &batch {
            if event_relates_to_run(
                &stored.envelope.payload,
                run_id,
                &mut tracked_tasks,
                &mut tracked_approvals,
                &mut tracked_invocations,
            ) {
                related.push(stored.clone());
            }
        }

        cursor = batch.last().map(|stored| stored.position);
    }

    Ok(related)
}

pub(crate) async fn build_diagnosis_report(
    state: &AppState,
    run: &RunRecord,
    stale_after_ms: u64,
) -> Result<(DiagnosisReport, bool), cairn_store::StoreError> {
    let now = now_ms();
    let tasks =
        TaskReadModel::list_by_parent_run(state.runtime.store.as_ref(), &run.run_id, 1_000).await?;
    let events = collect_run_events(state, &run.run_id).await?;

    let mut task_activity = HashMap::<String, u64>::new();
    for stored in &events {
        if let Some(task_id) = task_activity_task_id(&stored.envelope.payload) {
            task_activity.insert(task_id.as_str().to_owned(), stored.stored_at);
        }
    }

    let active_tasks: Vec<DiagnosedTaskActivity> = tasks
        .iter()
        .filter(|task| !task.state.is_terminal())
        .map(|task| DiagnosedTaskActivity {
            task_id: task.task_id.to_string(),
            state: task.state,
            last_activity_ms: task_activity
                .get(task.task_id.as_str())
                .copied()
                .unwrap_or(task.updated_at),
        })
        .collect();

    let stalled_tasks: Vec<String> = tasks
        .iter()
        .filter(|task| !task.state.is_terminal())
        .filter(|task| {
            let last_activity_ms = task_activity
                .get(task.task_id.as_str())
                .copied()
                .unwrap_or(task.updated_at);
            let activity_stale = now.saturating_sub(last_activity_ms) > stale_after_ms;
            let lease_expired = task.state == TaskState::Leased
                && task
                    .lease_expires_at
                    .is_some_and(|lease_expires_at| lease_expires_at <= now);
            activity_stale || lease_expired
        })
        .map(|task| task.task_id.to_string())
        .collect();

    let has_expired_leases = tasks.iter().any(|task| {
        task.state == TaskState::Leased
            && task
                .lease_expires_at
                .is_some_and(|lease_expires_at| lease_expires_at <= now)
    });

    let (last_event_type, last_event_ms) = events
        .last()
        .map(|stored| {
            (
                event_type_name(&stored.envelope.payload).to_owned(),
                stored.stored_at,
            )
        })
        .unwrap_or_else(|| ("unknown".to_owned(), run.updated_at));

    let suggested_action = if has_expired_leases {
        "release_leases"
    } else if active_tasks.is_empty() {
        "check_session"
    } else if !stalled_tasks.is_empty() {
        "intervene_or_recover"
    } else {
        "observe"
    };

    let is_stalled = if active_tasks.is_empty() {
        now.saturating_sub(run.updated_at) > stale_after_ms
    } else {
        active_tasks.iter().all(|task| {
            now.saturating_sub(task.last_activity_ms) > stale_after_ms
                || stalled_tasks.iter().any(|stalled| stalled == &task.task_id)
        })
    };

    Ok((
        DiagnosisReport {
            run_id: run.run_id.to_string(),
            state: run.state,
            duration_ms: now.saturating_sub(run.created_at),
            active_tasks,
            stalled_tasks,
            last_event_type,
            last_event_ms,
            suggested_action: suggested_action.to_owned(),
        },
        is_stalled,
    ))
}

pub(crate) fn state_label<S: serde::Serialize>(state: &S) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

pub(crate) async fn build_run_replay_result(
    state: &AppState,
    run_id: &RunId,
    from_position: Option<u64>,
    to_position: Option<u64>,
) -> Result<ReplayResult, cairn_store::StoreError> {
    let events = collect_run_events(state, run_id).await?;
    let selected: Vec<StoredEvent> = events
        .into_iter()
        .filter(|event| from_position.is_none_or(|from| event.position.0 >= from))
        .filter(|event| to_position.is_none_or(|to| event.position.0 <= to))
        .collect();

    let replay_store = Arc::new(cairn_store::InMemoryStore::new());
    let replay_events: Vec<EventEnvelope<RuntimeEvent>> = selected
        .iter()
        .filter(|event| event_is_replay_relevant(&event.envelope.payload))
        .map(|event| {
            let mut envelope = event.envelope.clone();
            envelope.causation_id = None;
            envelope
        })
        .collect();
    if !replay_events.is_empty() {
        replay_store.append(&replay_events).await?;
    }

    let final_run_state = RunReadModel::get(replay_store.as_ref(), run_id)
        .await?
        .map(|run| state_label(&run.state));
    let final_task_states = TaskReadModel::list_by_parent_run(replay_store.as_ref(), run_id, 1_000)
        .await?
        .into_iter()
        .map(|task| ReplayTaskStateView {
            task_id: task.task_id.to_string(),
            state: state_label(&task.state),
        })
        .collect();
    let checkpoints_found = selected
        .iter()
        .filter(|event| matches!(event.envelope.payload, RuntimeEvent::CheckpointRecorded(_)))
        .count() as u32;

    Ok(ReplayResult {
        events_replayed: selected.len() as u32,
        final_run_state,
        final_task_states,
        checkpoints_found,
    })
}

pub(crate) async fn checkpoint_recorded_position(
    store: &cairn_store::InMemoryStore,
    checkpoint_id: &CheckpointId,
    run_id: &RunId,
) -> Result<Option<EventPosition>, cairn_store::StoreError> {
    let events = store
        .read_by_entity(&EntityRef::Checkpoint(checkpoint_id.clone()), None, 100)
        .await?;
    Ok(events
        .into_iter()
        .find_map(|stored| match stored.envelope.payload {
            RuntimeEvent::CheckpointRecorded(ref checkpoint) if checkpoint.run_id == *run_id => {
                Some(stored.position)
            }
            _ => None,
        }))
}

pub(crate) async fn derive_recovery_status(
    state: &AppState,
    run_id: &RunId,
) -> Result<RecoveryStatusResponse, axum::response::Response> {
    let events = state
        .runtime
        .store
        .read_stream(None, 10_000)
        .await
        .map_err(|err| {
            tracing::error!("derive_recovery_status read_stream failed: {err}");
            AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                err.to_string(),
            )
            .into_response()
        })?;

    let mut last_attempt_reason = None;
    let mut last_recovered = None;

    for stored in events {
        match &stored.envelope.payload {
            cairn_domain::RuntimeEvent::RecoveryAttempted(event)
                if event.run_id.as_ref() == Some(run_id) =>
            {
                last_attempt_reason = Some(event.reason.clone());
            }
            cairn_domain::RuntimeEvent::RecoveryCompleted(event)
                if event.run_id.as_ref() == Some(run_id) =>
            {
                last_recovered = Some(event.recovered);
            }
            _ => {}
        }
    }

    Ok(RecoveryStatusResponse {
        run_id: run_id.to_string(),
        last_attempt_reason,
        last_recovered,
    })
}

pub(crate) async fn append_runtime_event(
    state: &AppState,
    payload: cairn_domain::RuntimeEvent,
    suffix: &str,
) -> Result<(), cairn_runtime::RuntimeError> {
    let event = cairn_domain::EventEnvelope::for_runtime_event(
        cairn_domain::EventId::new(format!("evt_{}_{}", now_ms(), suffix)),
        cairn_domain::EventSource::Runtime,
        payload,
    );
    state.runtime.store.append(&[event]).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Parse / utility helpers
// ---------------------------------------------------------------------------

pub(crate) fn parse_csv_values(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(crate) fn parse_project_scope(project: &str) -> Option<(&str, &str, &str)> {
    let mut parts = project.split('/');
    let tenant_id = parts.next()?;
    let workspace_id = parts.next()?;
    let project_id = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some((tenant_id, workspace_id, project_id))
}

pub(crate) fn parse_scope_name(scope: &str) -> Option<Scope> {
    match scope {
        "system" => Some(Scope::System),
        "tenant" => Some(Scope::Tenant),
        "workspace" => Some(Scope::Workspace),
        "project" => Some(Scope::Project),
        _ => None,
    }
}

pub(crate) fn mailbox_message_view(
    state: &AppState,
    record: cairn_store::projections::MailboxRecord,
) -> Option<MailboxMessageView> {
    let metadata = state
        .mailbox_messages
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(record.message_id.as_str())
        .cloned()?;

    Some(MailboxMessageView {
        message_id: record.message_id.to_string(),
        run_id: record.run_id.map(|id| id.to_string()),
        task_id: record.task_id.map(|id| id.to_string()),
        sender_id: metadata.sender_id,
        body: metadata.body,
        delivered: metadata.delivered,
        created_at: record.created_at,
    })
}

pub(crate) fn feed_item_from_signal(record: &cairn_domain::SignalRecord) -> FeedItem {
    FeedItem {
        id: record.id.to_string(),
        source: record.source.clone(),
        kind: Some("signal".to_owned()),
        title: Some(format!("Signal from {}", record.source)),
        body: Some(record.payload.to_string()),
        url: None,
        author: None,
        avatar_url: None,
        repo_full_name: None,
        is_read: false,
        is_archived: false,
        group_key: Some(format!("signal:{}", record.source)),
        created_at: record.timestamp_ms.to_string(),
    }
}

pub(crate) async fn scoped_worker(
    state: &AppState,
    tenant_id: &TenantId,
    worker_id: &str,
) -> Result<ExternalWorkerRecord, AppApiError> {
    match state
        .runtime
        .external_workers
        .get(&WorkerId::new(worker_id))
        .await
    {
        Ok(Some(worker)) if worker.tenant_id == *tenant_id => Ok(worker),
        Ok(Some(_)) | Ok(None) => Err(AppApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "worker not found",
        )),
        Err(err) => Err(AppApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            err.to_string(),
        )),
    }
}

pub(crate) fn build_external_worker_report(
    worker_id: &str,
    project: &ProjectKey,
    task_id: &str,
    lease_token: u64,
    run_id: Option<&str>,
    message: Option<String>,
    percent: Option<u16>,
    outcome: Option<&str>,
) -> Result<ExternalWorkerReport, String> {
    let outcome = outcome
        .map(cairn_runtime::parse_outcome)
        .transpose()
        .map_err(|err| err.to_string())?;

    Ok(ExternalWorkerReport {
        project: project.clone(),
        worker_id: WorkerId::new(worker_id),
        run_id: run_id.map(RunId::new),
        task_id: TaskId::new(task_id),
        lease_token,
        reported_at_ms: now_ms(),
        progress: if message.is_some() || percent.is_some() {
            Some(ExternalWorkerProgress {
                message,
                percent_milli: percent,
            })
        } else {
            None
        },
        outcome,
    })
}

// current_event_head and publish_runtime_frames_since are defined in
// handlers::sse and re-exported via crate::handlers::sse::*.

// ── Graph trace snapshot ──────────────────────────────────────────────────────

use cairn_graph::in_memory::InMemoryGraphStore;
use cairn_graph::projections::{GraphEdge, GraphNode, NodeKind};

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct GraphTraceResponse {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub root: Option<String>,
}

pub(crate) fn graph_trace_snapshot(
    graph: &InMemoryGraphStore,
    project: &ProjectKey,
    limit: usize,
) -> GraphTraceResponse {
    let mut nodes = graph
        .all_nodes()
        .into_values()
        .filter(|node| node.project.as_ref() == Some(project))
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.node_id.cmp(&right.node_id))
    });
    nodes.truncate(limit.clamp(1, 500));

    let node_ids = nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect::<std::collections::HashSet<_>>();
    let mut edges = graph
        .all_edges()
        .into_iter()
        .filter(|edge| {
            node_ids.contains(&edge.source_node_id) && node_ids.contains(&edge.target_node_id)
        })
        .collect::<Vec<_>>();
    edges.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.source_node_id.cmp(&right.source_node_id))
            .then_with(|| left.target_node_id.cmp(&right.target_node_id))
    });

    let root = nodes
        .iter()
        .find(|node| {
            matches!(
                node.kind,
                NodeKind::Session | NodeKind::Run | NodeKind::Task
            )
        })
        .map(|node| node.node_id.clone());

    GraphTraceResponse { nodes, edges, root }
}

pub fn event_type_name(event: &RuntimeEvent) -> &'static str {
    match event {
        RuntimeEvent::SessionCreated(_) => "session_created",
        RuntimeEvent::SessionStateChanged(_) => "session_state_changed",
        RuntimeEvent::SessionCostUpdated(_) => "session_cost_updated",
        RuntimeEvent::RunCostUpdated(_) => "run_cost_updated",
        RuntimeEvent::RunCreated(_) => "run_created",
        RuntimeEvent::RunStateChanged(_) => "run_state_changed",
        RuntimeEvent::TaskCreated(_) => "task_created",
        RuntimeEvent::PauseScheduled(_) => "pause_scheduled",
        RuntimeEvent::OperatorIntervention(_) => "operator_intervention",
        RuntimeEvent::TaskLeaseClaimed(_) => "task_lease_claimed",
        RuntimeEvent::TaskLeaseHeartbeated(_) => "task_lease_heartbeated",
        RuntimeEvent::TaskStateChanged(_) => "task_state_changed",
        RuntimeEvent::TaskDependencyAdded(_) => "task_dependency_added",
        RuntimeEvent::TaskDependencyResolved(_) => "task_dependency_resolved",
        RuntimeEvent::ApprovalRequested(_) => "approval_requested",
        RuntimeEvent::ApprovalResolved(_) => "approval_resolved",
        RuntimeEvent::ApprovalDelegated(_) => "approval_delegated",
        RuntimeEvent::AuditLogEntryRecorded(_) => "audit_log_entry_recorded",
        RuntimeEvent::ApprovalPolicyCreated(_) => "approval_policy_created",
        RuntimeEvent::CheckpointRecorded(_) => "checkpoint_recorded",
        RuntimeEvent::CheckpointStrategySet(_) => "checkpoint_strategy_set",
        RuntimeEvent::CheckpointRestored(_) => "checkpoint_restored",
        RuntimeEvent::MailboxMessageAppended(_) => "mailbox_message_appended",
        RuntimeEvent::ChannelCreated(_) => "channel_created",
        RuntimeEvent::ChannelMessageSent(_) => "channel_message_sent",
        RuntimeEvent::ChannelMessageConsumed(_) => "channel_message_consumed",
        RuntimeEvent::ToolInvocationStarted(_) => "tool_invocation_started",
        RuntimeEvent::PermissionDecisionRecorded(_) => "permission_decision_recorded",
        RuntimeEvent::ToolInvocationProgressUpdated(_) => "tool_invocation_progress_updated",
        RuntimeEvent::ToolInvocationCompleted(_) => "tool_invocation_completed",
        RuntimeEvent::ToolInvocationFailed(_) => "tool_invocation_failed",
        RuntimeEvent::SignalIngested(_) => "signal_ingested",
        RuntimeEvent::SignalSubscriptionCreated(_) => "signal_subscription_created",
        RuntimeEvent::SignalRouted(_) => "signal_routed",
        RuntimeEvent::TriggerCreated(_) => "trigger_created",
        RuntimeEvent::TriggerEnabled(_) => "trigger_enabled",
        RuntimeEvent::TriggerDisabled(_) => "trigger_disabled",
        RuntimeEvent::TriggerSuspended(_) => "trigger_suspended",
        RuntimeEvent::TriggerResumed(_) => "trigger_resumed",
        RuntimeEvent::TriggerDeleted(_) => "trigger_deleted",
        RuntimeEvent::TriggerFired(_) => "trigger_fired",
        RuntimeEvent::TriggerSkipped(_) => "trigger_skipped",
        RuntimeEvent::TriggerDenied(_) => "trigger_denied",
        RuntimeEvent::TriggerRateLimited(_) => "trigger_rate_limited",
        RuntimeEvent::TriggerPendingApproval(_) => "trigger_pending_approval",
        RuntimeEvent::RunTemplateCreated(_) => "run_template_created",
        RuntimeEvent::RunTemplateDeleted(_) => "run_template_deleted",
        RuntimeEvent::ExternalWorkerRegistered(_) => "external_worker_registered",
        RuntimeEvent::ExternalWorkerReported(_) => "external_worker_reported",
        RuntimeEvent::ExternalWorkerSuspended(_) => "external_worker_suspended",
        RuntimeEvent::ExternalWorkerReactivated(_) => "external_worker_reactivated",
        RuntimeEvent::SubagentSpawned(_) => "subagent_spawned",
        RuntimeEvent::RecoveryAttempted(_) => "recovery_attempted",
        RuntimeEvent::RecoveryCompleted(_) => "recovery_completed",
        RuntimeEvent::RecoveryEscalated(_) => "recovery_escalated",
        RuntimeEvent::RunSlaSet(_) => "run_sla_set",
        RuntimeEvent::EventLogCompacted(_) => "event_log_compacted",
        RuntimeEvent::SnapshotCreated(_) => "snapshot_created",
        RuntimeEvent::ProviderPoolCreated(_) => "provider_pool_created",
        RuntimeEvent::ProviderPoolConnectionAdded(_) => "provider_pool_connection_added",
        RuntimeEvent::ProviderPoolConnectionRemoved(_) => "provider_pool_connection_removed",
        RuntimeEvent::ResourceShared(_) => "resource_shared",
        RuntimeEvent::ResourceShareRevoked(_) => "resource_share_revoked",
        RuntimeEvent::RunSlaBreached(_) => "run_sla_breached",
        RuntimeEvent::UserMessageAppended(_) => "user_message_appended",
        RuntimeEvent::IngestJobStarted(_) => "ingest_job_started",
        RuntimeEvent::IngestJobCompleted(_) => "ingest_job_completed",
        RuntimeEvent::EvalDatasetCreated(_) => "eval_dataset_created",
        RuntimeEvent::EvalDatasetEntryAdded(_) => "eval_dataset_entry_added",
        RuntimeEvent::EvalRubricCreated(_) => "eval_rubric_created",
        RuntimeEvent::EvalBaselineSet(_) => "eval_baseline_set",
        RuntimeEvent::EvalBaselineLocked(_) => "eval_baseline_locked",
        RuntimeEvent::EvalRunStarted(_) => "eval_run_started",
        RuntimeEvent::EvalRunCompleted(_) => "eval_run_completed",
        RuntimeEvent::PromptAssetCreated(_) => "prompt_asset_created",
        RuntimeEvent::PromptVersionCreated(_) => "prompt_version_created",
        RuntimeEvent::PromptReleaseCreated(_) => "prompt_release_created",
        RuntimeEvent::PromptReleaseTransitioned(_) => "prompt_release_transitioned",
        RuntimeEvent::TenantCreated(_) => "tenant_created",
        RuntimeEvent::TenantQuotaSet(_) => "tenant_quota_set",
        RuntimeEvent::TenantQuotaViolated(_) => "tenant_quota_violated",
        RuntimeEvent::WorkspaceCreated(_) => "workspace_created",
        RuntimeEvent::WorkspaceMemberAdded(_) => "workspace_member_added",
        RuntimeEvent::WorkspaceMemberRemoved(_) => "workspace_member_removed",
        RuntimeEvent::DefaultSettingSet(_) => "default_setting_set",
        RuntimeEvent::DefaultSettingCleared(_) => "default_setting_cleared",
        RuntimeEvent::RetentionPolicySet(_) => "retention_policy_set",
        RuntimeEvent::LicenseActivated(_) => "license_activated",
        RuntimeEvent::EntitlementOverrideSet(_) => "entitlement_override_set",
        RuntimeEvent::ProjectCreated(_) => "project_created",
        RuntimeEvent::OperatorProfileCreated(_) => "operator_profile_created",
        RuntimeEvent::OperatorProfileUpdated(_) => "operator_profile_updated",
        RuntimeEvent::CredentialStored(_) => "credential_stored",
        RuntimeEvent::CredentialRevoked(_) => "credential_revoked",
        RuntimeEvent::CredentialKeyRotated(_) => "credential_key_rotated",
        RuntimeEvent::GuardrailPolicyCreated(_) => "guardrail_policy_created",
        RuntimeEvent::GuardrailPolicyEvaluated(_) => "guardrail_policy_evaluated",
        RuntimeEvent::ProviderConnectionRegistered(_) => "provider_connection_registered",
        RuntimeEvent::ProviderBindingCreated(_) => "provider_binding_created",
        RuntimeEvent::ProviderBindingStateChanged(_) => "provider_binding_state_changed",
        RuntimeEvent::ProviderHealthChecked(_) => "provider_health_checked",
        RuntimeEvent::ProviderMarkedDegraded(_) => "provider_marked_degraded",
        RuntimeEvent::ProviderRecovered(_) => "provider_recovered",
        RuntimeEvent::ProviderHealthScheduleSet(_) => "provider_health_schedule_set",
        RuntimeEvent::ProviderHealthScheduleTriggered(_) => "provider_health_schedule_triggered",
        RuntimeEvent::ProviderBudgetSet(_) => "provider_budget_set",
        RuntimeEvent::ProviderBudgetAlertTriggered(_) => "provider_budget_alert_triggered",
        RuntimeEvent::ProviderBudgetExceeded(_) => "provider_budget_exceeded",
        RuntimeEvent::RoutePolicyCreated(_) => "route_policy_created",
        RuntimeEvent::RoutePolicyUpdated(_) => "route_policy_updated",
        RuntimeEvent::RouteDecisionMade(_) => "route_decision_made",
        RuntimeEvent::ProviderCallCompleted(_) => "provider_call_completed",
        RuntimeEvent::ProviderModelRegistered(_) => "provider_model_registered",
        RuntimeEvent::RunCostAlertSet(_) => "run_cost_alert_set",
        RuntimeEvent::RunCostAlertTriggered(_) => "run_cost_alert_triggered",
        RuntimeEvent::NotificationPreferenceSet(_) => "notification_preference_set",
        RuntimeEvent::NotificationSent(_) => "notification_sent",
        RuntimeEvent::PromptRolloutStarted(_) => "prompt_rollout_started",
        RuntimeEvent::TaskPriorityChanged(_) => "task_priority_changed",
        RuntimeEvent::TaskLeaseExpired(_) => "task_lease_expired",
        RuntimeEvent::ProviderRetryPolicySet(_) => "provider_retry_policy_set",
        RuntimeEvent::SoulPatchProposed(_) => "soul_patch_proposed",
        RuntimeEvent::SoulPatchApplied(_) => "soul_patch_applied",
        RuntimeEvent::SpendAlertTriggered(_) => "spend_alert_triggered",
        RuntimeEvent::OutcomeRecorded(_) => "outcome_recorded",
        RuntimeEvent::ScheduledTaskCreated(_) => "scheduled_task_created",
        RuntimeEvent::PlanProposed(_) => "plan_proposed",
        RuntimeEvent::PlanApproved(_) => "plan_approved",
        RuntimeEvent::PlanRejected(_) => "plan_rejected",
        RuntimeEvent::PlanRevisionRequested(_) => "plan_revision_requested",
    }
}

pub(crate) fn event_message(event: &RuntimeEvent) -> String {
    match event {
        RuntimeEvent::SessionCreated(created) => format!("Session {} created", created.session_id),
        RuntimeEvent::SessionStateChanged(change) => {
            format!(
                "Session {} moved to {:?}",
                change.session_id, change.transition.to
            )
        }
        RuntimeEvent::SessionCostUpdated(cost) => {
            format!("Session {} cost updated", cost.session_id)
        }
        RuntimeEvent::RunCostUpdated(cost) => {
            format!("Run {} cost updated", cost.run_id)
        }
        RuntimeEvent::RunCreated(created) => format!("Run {} created", created.run_id),
        RuntimeEvent::RunStateChanged(change) => {
            format!("Run {} moved to {:?}", change.run_id, change.transition.to)
        }
        RuntimeEvent::OperatorIntervention(intervention) => format!(
            "Operator intervention {} applied to run {}",
            intervention.action,
            intervention
                .run_id
                .as_ref()
                .map(|id| id.as_str())
                .unwrap_or("?")
        ),
        RuntimeEvent::TaskCreated(created) => format!("Task {} created", created.task_id),
        RuntimeEvent::PauseScheduled(schedule) => {
            format!(
                "Pause scheduled for run {}",
                schedule
                    .run_id
                    .as_ref()
                    .map(|id| id.as_str())
                    .unwrap_or("?")
            )
        }
        RuntimeEvent::TaskLeaseClaimed(claimed) => {
            format!("Task {} leased to {}", claimed.task_id, claimed.lease_owner)
        }
        RuntimeEvent::TaskLeaseHeartbeated(heartbeated) => {
            format!("Task {} lease heartbeated", heartbeated.task_id)
        }
        RuntimeEvent::TaskStateChanged(change) => {
            format!(
                "Task {} moved to {:?}",
                change.task_id, change.transition.to
            )
        }
        RuntimeEvent::TaskDependencyAdded(change) => {
            format!(
                "Task {} now depends on {}",
                change.dependent_task_id, change.depends_on_task_id
            )
        }
        RuntimeEvent::TaskDependencyResolved(change) => {
            format!(
                "Task {} dependency on {} resolved",
                change.dependent_task_id, change.depends_on_task_id
            )
        }
        RuntimeEvent::ApprovalRequested(requested) => {
            format!("Approval {} requested", requested.approval_id)
        }
        RuntimeEvent::ApprovalResolved(resolved) => {
            format!(
                "Approval {} resolved as {:?}",
                resolved.approval_id, resolved.decision
            )
        }
        RuntimeEvent::ApprovalDelegated(delegated) => {
            format!(
                "Approval {} delegated to {}",
                delegated.approval_id, delegated.delegated_to
            )
        }
        RuntimeEvent::AuditLogEntryRecorded(entry) => {
            format!(
                "Audit {} recorded for {} {}",
                entry.entry_id, entry.resource_type, entry.resource_id
            )
        }
        RuntimeEvent::CheckpointRecorded(recorded) => {
            format!("Checkpoint {} recorded", recorded.checkpoint_id)
        }
        RuntimeEvent::CheckpointStrategySet(strategy) => {
            format!(
                "Checkpoint strategy {} set for run {}",
                strategy.strategy_id,
                strategy
                    .run_id
                    .as_ref()
                    .map(|id| id.as_str())
                    .unwrap_or("?")
            )
        }
        RuntimeEvent::CheckpointRestored(restored) => {
            format!("Checkpoint {} restored", restored.checkpoint_id)
        }
        RuntimeEvent::MailboxMessageAppended(message) => {
            format!("Mailbox message {} appended", message.message_id)
        }
        RuntimeEvent::ChannelCreated(created) => format!("Channel {} created", created.channel_id),
        RuntimeEvent::ChannelMessageSent(sent) => {
            format!("Message sent to channel {}", sent.channel_id)
        }
        RuntimeEvent::ChannelMessageConsumed(consumed) => {
            format!("Message consumed from channel {}", consumed.channel_id)
        }
        RuntimeEvent::ToolInvocationStarted(started) => {
            format!("Tool invocation {} started", started.invocation_id)
        }
        RuntimeEvent::PermissionDecisionRecorded(recorded) => {
            format!(
                "Permission decision recorded for {}",
                recorded.invocation_id.as_deref().unwrap_or("unknown")
            )
        }
        RuntimeEvent::ToolInvocationProgressUpdated(progress) => {
            format!(
                "Tool invocation {} progress updated",
                progress.invocation_id
            )
        }
        RuntimeEvent::ToolInvocationCompleted(completed) => {
            format!("Tool invocation {} completed", completed.invocation_id)
        }
        RuntimeEvent::ToolInvocationFailed(failed) => {
            format!("Tool invocation {} failed", failed.invocation_id)
        }
        RuntimeEvent::SignalIngested(ingested) => format!("Signal {} ingested", ingested.signal_id),
        RuntimeEvent::SignalSubscriptionCreated(subscription) => {
            format!(
                "Signal subscription {} created",
                subscription.subscription_id
            )
        }
        RuntimeEvent::SignalRouted(routed) => {
            format!("Signal {} routed", routed.signal_id)
        }
        RuntimeEvent::TriggerCreated(trigger) => {
            format!("Trigger {} created", trigger.trigger_id)
        }
        RuntimeEvent::TriggerEnabled(trigger) => {
            format!("Trigger {} enabled", trigger.trigger_id)
        }
        RuntimeEvent::TriggerDisabled(trigger) => {
            format!("Trigger {} disabled", trigger.trigger_id)
        }
        RuntimeEvent::TriggerSuspended(trigger) => {
            format!("Trigger {} suspended", trigger.trigger_id)
        }
        RuntimeEvent::TriggerResumed(trigger) => {
            format!("Trigger {} resumed", trigger.trigger_id)
        }
        RuntimeEvent::TriggerDeleted(trigger) => {
            format!("Trigger {} deleted", trigger.trigger_id)
        }
        RuntimeEvent::TriggerFired(trigger) => {
            format!(
                "Trigger {} fired run {} from signal {}",
                trigger.trigger_id, trigger.run_id, trigger.signal_id
            )
        }
        RuntimeEvent::TriggerSkipped(trigger) => {
            format!(
                "Trigger {} skipped for signal {}",
                trigger.trigger_id, trigger.signal_id
            )
        }
        RuntimeEvent::TriggerDenied(trigger) => {
            format!(
                "Trigger {} denied for signal {}",
                trigger.trigger_id, trigger.signal_id
            )
        }
        RuntimeEvent::TriggerRateLimited(trigger) => {
            format!(
                "Trigger {} rate limited for signal {}",
                trigger.trigger_id, trigger.signal_id
            )
        }
        RuntimeEvent::TriggerPendingApproval(trigger) => {
            format!(
                "Trigger {} pending approval for signal {}",
                trigger.trigger_id, trigger.signal_id
            )
        }
        RuntimeEvent::RunTemplateCreated(template) => {
            format!("Run template {} created", template.template_id)
        }
        RuntimeEvent::RunTemplateDeleted(template) => {
            format!("Run template {} deleted", template.template_id)
        }
        RuntimeEvent::ExternalWorkerRegistered(registered) => {
            format!("Worker {} registered", registered.worker_id)
        }
        RuntimeEvent::ExternalWorkerReported(reported) => {
            format!(
                "Worker {} reported on task {}",
                reported.report.worker_id, reported.report.task_id
            )
        }
        RuntimeEvent::ExternalWorkerSuspended(suspended) => {
            format!(
                "Worker {} suspended: {}",
                suspended.worker_id,
                suspended.reason.as_deref().unwrap_or("")
            )
        }
        RuntimeEvent::ExternalWorkerReactivated(reactivated) => {
            format!("Worker {} reactivated", reactivated.worker_id)
        }
        RuntimeEvent::SubagentSpawned(spawned) => {
            format!("Subagent task {} spawned", spawned.child_task_id)
        }
        RuntimeEvent::RecoveryAttempted(recovery) => recovery
            .run_id
            .as_ref()
            .map(|run_id| format!("Recovery attempted for run {run_id}"))
            .or_else(|| {
                recovery
                    .task_id
                    .as_ref()
                    .map(|task_id| format!("Recovery attempted for task {task_id}"))
            })
            .unwrap_or_else(|| "Recovery attempted".to_owned()),
        RuntimeEvent::RecoveryCompleted(recovery) => recovery
            .run_id
            .as_ref()
            .map(|run_id| format!("Recovery completed for run {run_id}"))
            .or_else(|| {
                recovery
                    .task_id
                    .as_ref()
                    .map(|task_id| format!("Recovery completed for task {task_id}"))
            })
            .unwrap_or_else(|| "Recovery completed".to_owned()),
        RuntimeEvent::RecoveryEscalated(e) => {
            format!(
                "Run {} escalated after {} recovery attempts: {}",
                e.run_id.as_ref().map(|r| r.to_string()).unwrap_or_default(),
                e.attempt_count,
                e.last_error.as_deref().unwrap_or("unknown")
            )
        }
        RuntimeEvent::UserMessageAppended(message) => {
            format!("User message appended to session {}", message.session_id)
        }
        RuntimeEvent::IngestJobStarted(job) => format!("Ingest job {} started", job.job_id),
        RuntimeEvent::IngestJobCompleted(job) => format!("Ingest job {} completed", job.job_id),
        RuntimeEvent::EvalDatasetCreated(dataset) => {
            format!("Eval dataset {} created", dataset.dataset_id)
        }
        RuntimeEvent::EvalDatasetEntryAdded(dataset) => {
            format!("Eval dataset {} entry added", dataset.dataset_id)
        }
        RuntimeEvent::EvalRubricCreated(rubric) => {
            format!("Eval rubric {} created", rubric.rubric_id)
        }
        RuntimeEvent::EvalBaselineSet(baseline) => {
            format!("Eval baseline {} set", baseline.baseline_id)
        }
        RuntimeEvent::EvalBaselineLocked(baseline) => {
            format!("Eval baseline {} locked", baseline.baseline_id)
        }
        RuntimeEvent::EvalRunStarted(eval_run) => {
            format!("Eval run {} started", eval_run.eval_run_id)
        }
        RuntimeEvent::EvalRunCompleted(eval_run) => {
            format!("Eval run {} completed", eval_run.eval_run_id)
        }
        RuntimeEvent::PromptAssetCreated(asset) => {
            format!("Prompt asset {} created", asset.prompt_asset_id)
        }
        RuntimeEvent::PromptVersionCreated(version) => {
            format!("Prompt version {} created", version.prompt_version_id)
        }
        RuntimeEvent::PromptReleaseCreated(release) => {
            format!("Prompt release {} created", release.prompt_release_id)
        }
        RuntimeEvent::PromptReleaseTransitioned(release) => {
            format!(
                "Prompt release {} moved to {:?}",
                release.prompt_release_id, release.to_state
            )
        }
        RuntimeEvent::TenantCreated(tenant) => {
            format!("Tenant {} created", tenant.tenant_id)
        }
        RuntimeEvent::TenantQuotaSet(quota) => {
            format!("Tenant quota set for {}", quota.tenant_id)
        }
        RuntimeEvent::TenantQuotaViolated(quota) => {
            format!(
                "Tenant {} quota violated: {} {}/{}",
                quota.tenant_id, quota.quota_type, quota.current, quota.limit
            )
        }
        RuntimeEvent::WorkspaceCreated(workspace) => {
            format!("Workspace {} created", workspace.workspace_id)
        }
        RuntimeEvent::WorkspaceMemberAdded(member) => {
            format!("Workspace member {} added", member.member_id)
        }
        RuntimeEvent::WorkspaceMemberRemoved(member) => {
            format!("Workspace member {} removed", member.member_id)
        }
        RuntimeEvent::DefaultSettingSet(setting) => {
            format!(
                "Default setting {} set for {:?}",
                setting.key, setting.scope
            )
        }
        RuntimeEvent::DefaultSettingCleared(setting) => {
            format!(
                "Default setting {} cleared for {:?}",
                setting.key, setting.scope
            )
        }
        RuntimeEvent::RetentionPolicySet(policy) => {
            format!("Retention policy set for tenant {}", policy.tenant_id)
        }
        RuntimeEvent::LicenseActivated(license) => {
            format!("License activated for tenant {}", license.tenant_id)
        }
        RuntimeEvent::EntitlementOverrideSet(override_set) => {
            format!("Entitlement override set for {}", override_set.feature)
        }
        RuntimeEvent::ProjectCreated(project) => {
            format!("Project {} created", project.project.project_id)
        }
        RuntimeEvent::OperatorProfileCreated(profile) => {
            format!("Operator profile {} created", profile.profile_id)
        }
        RuntimeEvent::OperatorProfileUpdated(profile) => {
            format!("Operator profile {} updated", profile.profile_id)
        }
        RuntimeEvent::CredentialStored(credential) => {
            format!("Credential {} stored", credential.credential_id)
        }
        RuntimeEvent::CredentialRevoked(credential) => {
            format!("Credential {} revoked", credential.credential_id)
        }
        RuntimeEvent::CredentialKeyRotated(rotation) => {
            format!("Credential key rotation {} completed", rotation.rotation_id)
        }
        RuntimeEvent::GuardrailPolicyCreated(policy) => {
            format!("Guardrail policy {} created", policy.policy_id)
        }
        RuntimeEvent::GuardrailPolicyEvaluated(policy) => {
            format!("Guardrail policy {} evaluated", policy.policy_id)
        }
        RuntimeEvent::ProviderConnectionRegistered(connection) => {
            format!(
                "Provider connection {} registered",
                connection.provider_connection_id
            )
        }
        RuntimeEvent::ProviderBindingCreated(binding) => {
            format!("Provider binding {} created", binding.provider_binding_id)
        }
        RuntimeEvent::ProviderBindingStateChanged(binding) => {
            format!(
                "Provider binding {} active={}",
                binding.provider_binding_id, binding.active
            )
        }
        RuntimeEvent::ProviderHealthChecked(health) => {
            format!(
                "Provider connection {} health checked",
                health.connection_id
            )
        }
        RuntimeEvent::ProviderMarkedDegraded(provider) => {
            format!(
                "Provider connection {} marked degraded",
                provider.connection_id
            )
        }
        RuntimeEvent::ProviderRecovered(provider) => {
            format!("Provider connection {} recovered", provider.connection_id)
        }
        RuntimeEvent::ProviderHealthScheduleSet(schedule) => {
            format!(
                "Provider health schedule {} set (interval {}ms)",
                schedule.schedule_id, schedule.interval_ms
            )
        }
        RuntimeEvent::ProviderHealthScheduleTriggered(schedule) => {
            format!(
                "Provider health schedule {} triggered",
                schedule.schedule_id
            )
        }
        RuntimeEvent::ProviderBudgetSet(budget) => {
            format!("Provider budget {} set", budget.budget_id)
        }
        RuntimeEvent::ProviderBudgetAlertTriggered(budget) => {
            format!("Provider budget {} alert triggered", budget.budget_id)
        }
        RuntimeEvent::ProviderBudgetExceeded(budget) => {
            format!("Provider budget {} exceeded", budget.budget_id)
        }
        RuntimeEvent::RoutePolicyCreated(policy) => {
            format!("Route policy {} created", policy.policy_id)
        }
        RuntimeEvent::RoutePolicyUpdated(policy) => {
            format!("Route policy {} updated", policy.policy_id)
        }
        RuntimeEvent::RouteDecisionMade(decision) => {
            format!("Route decision {} made", decision.route_decision_id)
        }
        RuntimeEvent::ProviderCallCompleted(call) => {
            format!("Provider call {} completed", call.provider_call_id)
        }
        RuntimeEvent::ApprovalPolicyCreated(policy) => {
            format!("Approval policy {} created", policy.policy_id)
        }
        RuntimeEvent::RunCostAlertSet(e) => {
            format!("Run cost alert set for run {}", e.run_id)
        }
        RuntimeEvent::RunCostAlertTriggered(e) => {
            format!(
                "Run cost alert triggered for run {} (actual {} micros)",
                e.run_id, e.actual_cost_micros
            )
        }
        RuntimeEvent::RunSlaSet(e) => {
            format!(
                "SLA set for run {}: {}ms target",
                e.run_id, e.target_completion_ms
            )
        }
        RuntimeEvent::RunSlaBreached(e) => {
            format!(
                "SLA breached for run {}: {}ms elapsed vs {}ms target",
                e.run_id, e.elapsed_ms, e.target_ms
            )
        }
        RuntimeEvent::EventLogCompacted(e) => {
            format!(
                "Event log compacted for tenant {}: {} → {} events",
                e.tenant_id, e.events_before, e.events_after
            )
        }
        RuntimeEvent::SnapshotCreated(e) => {
            format!(
                "Snapshot {} created for tenant {} at position {}",
                e.snapshot_id, e.tenant_id, e.event_position
            )
        }
        RuntimeEvent::PromptRolloutStarted(e) => {
            format!(
                "Prompt rollout started for release {} at {}%",
                e.release_id
                    .as_ref()
                    .map(|r| r.to_string())
                    .unwrap_or_default(),
                e.percent
            )
        }
        RuntimeEvent::TaskPriorityChanged(_)
        | RuntimeEvent::TaskLeaseExpired(_)
        | RuntimeEvent::ProviderModelRegistered(_)
        | RuntimeEvent::ProviderRetryPolicySet(_)
        | RuntimeEvent::NotificationPreferenceSet(_)
        | RuntimeEvent::NotificationSent(_)
        | RuntimeEvent::ProviderPoolCreated(_)
        | RuntimeEvent::ProviderPoolConnectionAdded(_)
        | RuntimeEvent::ProviderPoolConnectionRemoved(_)
        | RuntimeEvent::ResourceShared(_)
        | RuntimeEvent::ResourceShareRevoked(_)
        | RuntimeEvent::SoulPatchProposed(_)
        | RuntimeEvent::SoulPatchApplied(_)
        | RuntimeEvent::SpendAlertTriggered(_)
        | RuntimeEvent::OutcomeRecorded(_)
        | RuntimeEvent::ScheduledTaskCreated(_)
        | RuntimeEvent::PlanProposed(_)
        | RuntimeEvent::PlanApproved(_)
        | RuntimeEvent::PlanRejected(_)
        | RuntimeEvent::PlanRevisionRequested(_) => "unknown".to_string(),
    }
}

pub(crate) fn run_id_for_event(event: &RuntimeEvent) -> Option<String> {
    match event {
        RuntimeEvent::RunCreated(run) => Some(run.run_id.to_string()),
        RuntimeEvent::RunStateChanged(run) => Some(run.run_id.to_string()),
        RuntimeEvent::OperatorIntervention(intervention) => {
            intervention.run_id.as_ref().map(ToString::to_string)
        }
        RuntimeEvent::ApprovalRequested(approval) => {
            approval.run_id.as_ref().map(ToString::to_string)
        }
        RuntimeEvent::CheckpointRecorded(checkpoint) => Some(checkpoint.run_id.to_string()),
        RuntimeEvent::CheckpointStrategySet(strategy) => {
            strategy.run_id.as_ref().map(ToString::to_string)
        }
        RuntimeEvent::CheckpointRestored(checkpoint) => Some(checkpoint.run_id.to_string()),
        RuntimeEvent::ExternalWorkerReported(report) => {
            report.report.run_id.as_ref().map(ToString::to_string)
        }
        RuntimeEvent::RecoveryAttempted(recovery) => {
            recovery.run_id.as_ref().map(ToString::to_string)
        }
        RuntimeEvent::RecoveryCompleted(recovery) => {
            recovery.run_id.as_ref().map(ToString::to_string)
        }
        RuntimeEvent::RecoveryEscalated(recovery) => {
            recovery.run_id.as_ref().map(ToString::to_string)
        }
        RuntimeEvent::ToolInvocationStarted(invocation) => {
            invocation.run_id.as_ref().map(ToString::to_string)
        }
        RuntimeEvent::ToolInvocationProgressUpdated(_) => None,
        RuntimeEvent::ProviderCallCompleted(call) => call.run_id.as_ref().map(ToString::to_string),
        RuntimeEvent::UserMessageAppended(message) => Some(message.run_id.to_string()),
        RuntimeEvent::AuditLogEntryRecorded(_)
        | RuntimeEvent::DefaultSettingSet(_)
        | RuntimeEvent::DefaultSettingCleared(_) => None,
        _ => None,
    }
}

pub(crate) async fn append_run_intervention_event(
    state: &Arc<AppState>,
    run_id: &RunId,
    tenant_id: &TenantId,
    action: &str,
    reason: &str,
) -> Result<(), cairn_store::StoreError> {
    state
        .runtime
        .store
        .append(
            &[operator_event_envelope(RuntimeEvent::OperatorIntervention(
                cairn_domain::OperatorIntervention {
                    run_id: Some(run_id.clone()),
                    tenant_id: tenant_id.clone(),
                    action: action.to_owned(),
                    reason: reason.to_owned(),
                    intervened_at_ms: now_ms(),
                },
            ))],
        )
        .await
        .map(|_| ())
}

pub(crate) fn runtime_event_to_activity_entry(
    event: &RuntimeEvent,
    timestamp_ms: u64,
) -> Option<ActivityEntry> {
    match event {
        RuntimeEvent::RunCreated(e) => Some(ActivityEntry {
            entry_type: "run_created".to_owned(),
            timestamp_ms,
            run_id: Some(e.run_id.to_string()),
            task_id: None,
            state: None,
            description: format!("Run {} created", e.run_id),
        }),
        RuntimeEvent::RunStateChanged(e) => Some(ActivityEntry {
            entry_type: "run_state_changed".to_owned(),
            timestamp_ms,
            run_id: Some(e.run_id.to_string()),
            task_id: None,
            state: Some(format!("{:?}", e.transition.to).to_lowercase()),
            description: format!("Run {} moved to {:?}", e.run_id, e.transition.to),
        }),
        RuntimeEvent::TaskCreated(e) => Some(ActivityEntry {
            entry_type: "task_created".to_owned(),
            timestamp_ms,
            run_id: e.parent_run_id.as_ref().map(ToString::to_string),
            task_id: Some(e.task_id.to_string()),
            state: None,
            description: format!("Task {} created", e.task_id),
        }),
        RuntimeEvent::TaskStateChanged(e) => Some(ActivityEntry {
            entry_type: "task_state_changed".to_owned(),
            timestamp_ms,
            run_id: None,
            task_id: Some(e.task_id.to_string()),
            state: Some(format!("{:?}", e.transition.to).to_lowercase()),
            description: format!("Task {} moved to {:?}", e.task_id, e.transition.to),
        }),
        RuntimeEvent::ApprovalRequested(e) => Some(ActivityEntry {
            entry_type: "approval_requested".to_owned(),
            timestamp_ms,
            run_id: e.run_id.as_ref().map(ToString::to_string),
            task_id: e.task_id.as_ref().map(ToString::to_string),
            state: None,
            description: format!("Approval {} requested", e.approval_id),
        }),
        RuntimeEvent::SignalIngested(e) => Some(ActivityEntry {
            entry_type: "signal_received".to_owned(),
            timestamp_ms,
            run_id: None,
            task_id: None,
            state: None,
            description: format!("Signal {} received from {}", e.signal_id, e.source),
        }),
        _ => None,
    }
}
