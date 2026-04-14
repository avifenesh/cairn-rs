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

use crate::errors::{now_ms, runtime_error_response, store_error_response, AppApiError};
use crate::extractors::TenantScope;
use crate::state::{AppState, MailboxMessageView};
use crate::{
    default_repo_sandbox_policy, event_type_name, DiagnosedTaskActivity, DiagnosisReport,
    RecoveryStatusResponse, ReplayResult, ReplayTaskStateView, RunRecordView,
};

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
        // No repo allowlisted — use CWD. This is expected for API-driven
        // orchestration (POST /v1/runs/:id/orchestrate) where the operator
        // hasn't registered a repo. Webhook-driven orchestration (GitHub
        // pipeline) allowlists the repo before calling this function.
        tracing::debug!(
            run_id = %run.run_id,
            "no repo allowlisted for project; using process working directory"
        );
        return Ok(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
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
