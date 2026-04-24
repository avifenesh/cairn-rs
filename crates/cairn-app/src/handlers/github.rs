//! GitHub webhook, scan, queue, and installation handlers.
//!
//! Extracted from `lib.rs` — contains GitHub webhook reception, signature
//! verification, event-to-action matching, scan/queue processing, queue
//! control (pause/resume/skip/retry), installations, and concurrency settings.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use cairn_api::sse::SseFrame;
use cairn_store::EventLog;

use crate::errors::AppApiError;
use crate::state::{
    default_github_project_from_env, AppState, GitHubEventAction, GitHubIntegration,
    IssueQueueEntry, IssueQueueStatus, WebhookAction,
};

// ── DTOs ────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
pub(crate) struct ScanRequest {
    /// Repository in owner/repo format.
    pub repo: String,
    /// GitHub App installation ID (auto-resolved from repo owner if omitted).
    #[serde(default)]
    pub installation_id: Option<u64>,
    /// Optional label filter (comma-separated).
    #[serde(default)]
    pub labels: Option<String>,
    /// Max issues to scan (default 30, max 100).
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct QueuedIssue {
    pub issue_number: u64,
    pub title: String,
    pub session_id: String,
    pub run_id: String,
}

#[derive(serde::Deserialize)]
pub(crate) struct SetWebhookActionsRequest {
    pub actions: Vec<GitHubEventAction>,
}

// SetQueueConcurrencyRequest uses raw serde_json::Value

// ── Helpers ─────────────────────────────────────────────────────────────────

pub(crate) fn find_matching_action(
    actions: &[GitHubEventAction],
    event: &cairn_github::WebhookEvent,
) -> Option<GitHubEventAction> {
    let event_key = event.event_key();
    let repo = event.repository().unwrap_or("");

    for action in actions {
        if !event_pattern_matches(&action.event_pattern, &event_key) {
            continue;
        }
        if let Some(ref repo_filter) = action.repo_filter {
            if repo_filter != repo {
                continue;
            }
        }
        if let Some(ref label_filter) = action.label_filter {
            let has_label = match &event.payload {
                cairn_github::WebhookEventPayload::Issues(e) => {
                    e.issue.labels.iter().any(|l| &l.name == label_filter)
                }
                _ => false,
            };
            if !has_label {
                continue;
            }
        }
        return Some(action.clone());
    }
    None
}

pub(crate) fn event_pattern_matches(pattern: &str, event_key: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix(".*") {
        return event_key.starts_with(prefix);
    }
    pattern == event_key
}

pub(crate) async fn acknowledge_event(
    github: &GitHubIntegration,
    installation_id: u64,
    event: &cairn_github::WebhookEvent,
) -> Result<(), cairn_github::GitHubError> {
    let client = github.client_for_installation(installation_id).await;
    let repo = event.repository().unwrap_or("");
    let (owner, repo_name) = repo.split_once('/').unwrap_or(("", repo));

    let issue_number = match &event.payload {
        cairn_github::WebhookEventPayload::Issues(e) => Some(e.issue.number),
        cairn_github::WebhookEventPayload::IssueComment(e) => Some(e.issue.number),
        cairn_github::WebhookEventPayload::PullRequest(e) => Some(e.pull_request.number),
        _ => None,
    };

    if let Some(number) = issue_number {
        let msg = format!(
            "Cairn received this event (`{}`). Processing...",
            event.event_key()
        );
        client
            .create_comment(owner, repo_name, number, &msg)
            .await?;
    }
    Ok(())
}

pub(crate) async fn process_webhook_orchestrate(
    state: &AppState,
    github: &GitHubIntegration,
    event: &cairn_github::WebhookEvent,
) -> Result<(), String> {
    use cairn_domain::{RunId, SessionId};
    use cairn_store::projections::SessionReadModel;

    let repo_full = event.repository().unwrap_or("unknown/unknown");
    let (owner, repo_name) = repo_full.split_once('/').unwrap_or(("unknown", "unknown"));
    let installation_id = event.installation_id().ok_or("no installation_id")?;

    let (goal, issue_number) = match &event.payload {
        cairn_github::WebhookEventPayload::Issues(e) => {
            let body = e.issue.body.as_deref().unwrap_or("");
            (
                format!(
                    "GitHub Issue #{}: {}\n\nRepository: {}\n\n{}",
                    e.issue.number, e.issue.title, repo_full, body
                ),
                Some(e.issue.number),
            )
        }
        cairn_github::WebhookEventPayload::IssueComment(e) => (
            format!(
                "GitHub Issue #{} comment by @{}:\n{}\n\nRepository: {}",
                e.issue.number, e.comment.user.login, e.comment.body, repo_full
            ),
            Some(e.issue.number),
        ),
        cairn_github::WebhookEventPayload::PullRequest(e) => {
            let body = e.pull_request.body.as_deref().unwrap_or("");
            (
                format!(
                    "GitHub PR #{}: {}\n\nRepository: {}\n\n{}",
                    e.pull_request.number, e.pull_request.title, repo_full, body
                ),
                Some(e.pull_request.number),
            )
        }
        _ => (
            format!("GitHub event: {} on {}", event.event_key(), repo_full),
            None,
        ),
    };

    // T6a-C5: derive the project from the GitHub installation_id. Fall
    // back to an operator-configured default (env) only when no explicit
    // mapping exists. Never route an unmapped webhook into the legacy
    // "default_tenant" / "default_workspace" / "default_project" triple —
    // doing so commingles events across tenants and lets any operator in
    // that default tenant intervene in every tenant's GitHub pipeline.
    let project = match github.project_for_installation(installation_id).await {
        Some(p) => p,
        None => match default_github_project_from_env() {
            Some(p) => {
                tracing::warn!(
                    installation_id,
                    %repo_full,
                    "webhook routed to CAIRN_GITHUB_DEFAULT_PROJECT — operator should \
                     configure an explicit installation→project mapping"
                );
                p
            }
            None => {
                tracing::error!(
                    installation_id,
                    %repo_full,
                    "rejecting webhook: no project mapping for installation and no \
                     CAIRN_GITHUB_DEFAULT_PROJECT env set"
                );
                return Err("installation has no mapped project".to_owned());
            }
        },
    };

    let session_id_str = match issue_number {
        Some(n) => format!("gh-{}-{}-issue-{}", owner, repo_name, n),
        None => format!("gh-{}-{}-{}", owner, repo_name, event.delivery_id),
    };
    let session_id = SessionId::new(&session_id_str);

    if SessionReadModel::get(state.runtime.store.as_ref(), &session_id)
        .await
        .map_err(|e| e.to_string())?
        .is_none()
    {
        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .map_err(|e| e.to_string())?;
    }

    let run_id_str = format!("{}-run-{}", session_id_str, event.delivery_id);
    let run_id = RunId::new(&run_id_str);
    let run = state
        .runtime
        .runs
        .start(&project, &session_id, run_id.clone(), None)
        .await
        .map_err(|e| e.to_string())?;

    tracing::info!(
        session_id = session_id_str,
        run_id = run_id_str,
        event = event.event_key(),
        repo = repo_full,
        "Created session + run for GitHub webhook"
    );

    if let Some(number) = issue_number {
        let client = github.client_for_installation(installation_id).await;
        let msg = format!(
            "Cairn is working on this.\n\n- Session: `{}`\n- Run: `{}`",
            session_id_str, run_id_str
        );
        if let Err(e) = client.create_comment(owner, repo_name, number, &msg).await {
            tracing::warn!(error = %e, "Failed to post GitHub comment");
        }
    }

    webhook_trigger_orchestration(state, &run, &goal, None, None).await
}

pub(crate) async fn webhook_trigger_orchestration(
    state: &AppState,
    run: &cairn_store::projections::RunRecord,
    goal: &str,
    _installation_id: Option<u64>,
    work_item: Option<&cairn_integrations::WorkItem>,
) -> Result<(), String> {
    use cairn_orchestrator::{
        LlmDecidePhase, LoopConfig, LoopTermination, OrchestrationContext, OrchestratorLoop,
        RuntimeExecutePhase, StandardGatherPhase,
    };
    use cairn_runtime::services::{
        ApprovalServiceImpl, CheckpointServiceImpl, MailboxServiceImpl, ToolInvocationServiceImpl,
    };

    if run.state == cairn_domain::RunState::Pending {
        use cairn_domain::{RunState, RunStateChanged, RuntimeEvent, StateTransition};
        use cairn_runtime::make_envelope;
        let evt = make_envelope(RuntimeEvent::RunStateChanged(RunStateChanged {
            project: run.project.clone(),
            run_id: run.run_id.clone(),
            transition: StateTransition {
                from: Some(RunState::Pending),
                to: RunState::Running,
            },
            failure_class: None,
            pause_reason: None,
            resume_trigger: None,
        }));
        let _ = state.runtime.store.append(&[evt]).await;
    }

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let working_dir = crate::helpers::working_dir_for_run(state, run)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(run_id = %run.run_id, error = %e, "workspace setup failed; using ephemeral dir");
            let fallback = std::env::temp_dir().join("cairn-runs").join(run.run_id.as_str());
            let _ = std::fs::create_dir_all(&fallback);
            fallback
        });
    let working_dir_for_tools = working_dir.clone();

    let ctx = OrchestrationContext {
        project: run.project.clone(),
        session_id: run.session_id.clone(),
        run_id: run.run_id.clone(),
        task_id: None,
        iteration: 0,
        goal: goal.to_owned(),
        agent_type: "github_agent".to_owned(),
        run_started_at_ms: now_ms,
        working_dir,
        run_mode: cairn_domain::decisions::RunMode::default(),
        discovered_tool_names: vec![],
        step_history: vec![],
        is_recovery: false,
        approval_timeout: None,
    };

    let model_id = {
        let brain = state.runtime.runtime_config.default_brain_model().await;
        if brain.trim().is_empty() || brain == "default" {
            state.runtime.runtime_config.default_generate_model().await
        } else {
            brain
        }
    };
    let model_id = model_id.trim().to_owned();
    if model_id.is_empty() || model_id == "default" {
        return Err("No brain model configured".to_owned());
    }

    let is_bedrock = model_id.contains('.') && !model_id.contains('/');
    let brain = match state
        .runtime
        .provider_registry
        .resolve_generation_for_model(
            &run.project.tenant_id,
            &model_id,
            cairn_runtime::ProviderResolutionPurpose::Brain,
        )
        .await
    {
        Ok(Some(p)) => p,
        Ok(None) if is_bedrock => state
            .bedrock_provider
            .as_ref()
            .ok_or("No Bedrock provider")?
            .clone(),
        Ok(None) => state
            .brain_provider
            .as_ref()
            .ok_or("No brain provider")?
            .clone(),
        Err(e) => return Err(format!("Provider error: {e}")),
    };

    let gather = StandardGatherPhase::builder(state.runtime.store.clone())
        .with_retrieval(state.retrieval.clone())
        .with_graph(state.graph.clone())
        .with_defaults(state.runtime.store.clone())
        .with_checkpoints(state.runtime.store.clone())
        .build();

    let base = state
        .tool_registry
        .as_deref()
        .unwrap_or_else(|| Box::leak(Box::new(cairn_tools::BuiltinToolRegistry::new())));
    let full = crate::tool_impls::build_full_tool_registry(base, working_dir_for_tools);
    let registry = if let Some(item) = work_item {
        build_integration_tool_registry_from_base(state, &item.integration_id, item, &full).await
    } else {
        Arc::new(full)
    };

    let decide = LlmDecidePhase::new(brain, model_id).with_tools(registry.clone());

    let store = state.runtime.store.clone();
    let config = LoopConfig {
        max_iterations: 50,
        timeout_ms: 30 * 60 * 1_000,
        ..LoopConfig::default()
    };

    let execute = RuntimeExecutePhase::builder()
        .tool_registry(registry)
        .run_service(state.runtime.runs.clone())
        .task_service(state.runtime.tasks.clone())
        .approval_service(Arc::new(ApprovalServiceImpl::new(store.clone())))
        .checkpoint_service(Arc::new(CheckpointServiceImpl::new(store.clone())))
        .mailbox_service(Arc::new(MailboxServiceImpl::new(store.clone())))
        .tool_invocation_service(Arc::new(ToolInvocationServiceImpl::new(store)))
        .checkpoint_every_n_tool_calls(config.checkpoint_every_n_tool_calls)
        .tool_result_cache(state.tool_result_cache.clone())
        .build();

    let emitter = build_orchestrator_emitter(state);

    let orchestrator = OrchestratorLoop::new(gather, decide, execute, config).with_emitter(emitter);
    let run_id = run.run_id.clone();
    let project = run.project.clone();

    match orchestrator.run(ctx).await {
        Ok(term) => {
            tracing::info!(run_id = %run_id, ?term, "Webhook orchestration finished");
            let (to_state, failure_class) = match term {
                LoopTermination::Completed { .. } | LoopTermination::PlanProposed { .. } => {
                    (cairn_domain::RunState::Completed, None)
                }
                LoopTermination::WaitingApproval { .. } => return Ok(()),
                LoopTermination::WaitingSubagent { .. } => return Ok(()),
                _ => (
                    cairn_domain::RunState::Failed,
                    Some(cairn_domain::FailureClass::ExecutionError),
                ),
            };
            use cairn_domain::{RunStateChanged, RuntimeEvent, StateTransition};
            use cairn_runtime::make_envelope;
            let evt = make_envelope(RuntimeEvent::RunStateChanged(RunStateChanged {
                project,
                run_id,
                transition: StateTransition {
                    from: Some(cairn_domain::RunState::Running),
                    to: to_state,
                },
                failure_class,
                pause_reason: None,
                resume_trigger: None,
            }));
            let _ = state.runtime.store.append(&[evt]).await;
            Ok(())
        }
        Err(e) => Err(format!("Orchestration error: {e}")),
    }
}

/// Build the tool registry for integration-triggered orchestration runs.
pub(crate) async fn build_integration_tool_registry_from_base(
    state: &AppState,
    integration_id: &str,
    work_item: &cairn_integrations::WorkItem,
    full_base: &cairn_tools::BuiltinToolRegistry,
) -> Arc<cairn_tools::BuiltinToolRegistry> {
    if let Some(integration) = state.integrations.get(integration_id).await {
        integration
            .prepare_tool_registry(full_base, work_item)
            .await
    } else {
        Arc::new(cairn_tools::BuiltinToolRegistry::from_existing(full_base))
    }
}

/// Build the composite SSE + tracing emitter for the orchestrator loop.
pub(crate) fn build_orchestrator_emitter(
    state: &AppState,
) -> std::sync::Arc<dyn cairn_orchestrator::OrchestratorEventEmitter> {
    let sse_emitter = std::sync::Arc::new(crate::sse_hooks::SseOrchestratorEmitter::new(
        state.runtime_sse_tx.clone(),
        state.sse_event_buffer.clone(),
        state.sse_seq.clone(),
    ));

    struct TracingEmitter {
        inner: std::sync::Arc<crate::sse_hooks::SseOrchestratorEmitter>,
        store: std::sync::Arc<cairn_store::InMemoryStore>,
        exporter: std::sync::Arc<cairn_runtime::telemetry::OtlpExporter>,
        fatal_error: std::sync::Mutex<Option<String>>,
    }

    #[async_trait::async_trait]
    impl cairn_orchestrator::OrchestratorEventEmitter for TracingEmitter {
        async fn on_started(&self, ctx: &cairn_orchestrator::OrchestrationContext) {
            self.inner.on_started(ctx).await;
        }
        async fn on_gather_completed(
            &self,
            ctx: &cairn_orchestrator::OrchestrationContext,
            g: &cairn_orchestrator::GatherOutput,
        ) {
            self.inner.on_gather_completed(ctx, g).await;
        }
        async fn on_decide_completed(
            &self,
            ctx: &cairn_orchestrator::OrchestrationContext,
            d: &cairn_orchestrator::DecideOutput,
        ) {
            self.inner.on_decide_completed(ctx, d).await;
            // Emit RouteDecisionMade + ProviderCallCompleted in a single
            // append so the Postgres projection applies RouteDecisionMade
            // before ProviderCallCompleted's FK reference to it. See
            // `handlers/runs.rs::on_decide_completed` for the F24 fix
            // rationale (2026-04-23).
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let call_id = format!("orch_{}_{}", ctx.run_id.as_str(), now);
            let route_decision_id =
                cairn_domain::RouteDecisionId::new(format!("rd_{call_id}"));
            let route_attempt_id =
                cairn_domain::RouteAttemptId::new(format!("ra_{call_id}"));
            let provider_binding_id = cairn_domain::ProviderBindingId::new("brain");

            let route_event = cairn_domain::EventEnvelope::for_runtime_event(
                cairn_domain::EventId::new(format!("evt_route_{call_id}")),
                cairn_domain::EventSource::Runtime,
                cairn_domain::RuntimeEvent::RouteDecisionMade(
                    cairn_domain::events::RouteDecisionMade {
                        project: ctx.project.clone(),
                        route_decision_id: route_decision_id.clone(),
                        operation_kind: cairn_domain::providers::OperationKind::Generate,
                        selected_provider_binding_id: Some(provider_binding_id.clone()),
                        final_status:
                            cairn_domain::providers::RouteDecisionStatus::Selected,
                        attempt_count: 1,
                        fallback_used: false,
                        decided_at: now,
                    },
                ),
            );

            let provider_event = cairn_domain::EventEnvelope::for_runtime_event(
                cairn_domain::EventId::new(format!("evt_trace_{call_id}")),
                cairn_domain::EventSource::Runtime,
                cairn_domain::RuntimeEvent::ProviderCallCompleted(
                    cairn_domain::events::ProviderCallCompleted {
                        project: ctx.project.clone(),
                        provider_call_id: cairn_domain::ProviderCallId::new(&call_id),
                        route_decision_id,
                        route_attempt_id,
                        provider_binding_id,
                        provider_connection_id: cairn_domain::ProviderConnectionId::new("brain"),
                        provider_model_id: cairn_domain::ProviderModelId::new(&d.model_id),
                        operation_kind: cairn_domain::providers::OperationKind::Generate,
                        status: cairn_domain::providers::ProviderCallStatus::Succeeded,
                        latency_ms: Some(d.latency_ms),
                        input_tokens: d.input_tokens,
                        output_tokens: d.output_tokens,
                        cost_micros: Some(
                            ((d.input_tokens.unwrap_or(0) as u64).saturating_mul(500)
                                + (d.output_tokens.unwrap_or(0) as u64).saturating_mul(1500))
                                / 1_000,
                        ),
                        completed_at: now,
                        session_id: Some(ctx.session_id.clone()),
                        run_id: Some(ctx.run_id.clone()),
                        error_class: None,
                        raw_error_message: None,
                        retry_count: 0,
                        task_id: ctx
                            .task_id
                            .as_ref()
                            .map(|t| cairn_domain::TaskId::new(t.as_str())),
                        prompt_release_id: None,
                        fallback_position: 0,
                        started_at: now.saturating_sub(d.latency_ms),
                        finished_at: now,
                    },
                ),
            );
            let provider_payload = provider_event.payload.clone();
            if let Err(e) = self
                .store
                .append(&[route_event, provider_event])
                .await
            {
                tracing::error!(
                    run_id = %ctx.run_id,
                    error = %e,
                    "event store append failed — in-memory/secondary logs have diverged, aborting run"
                );
                let mut slot = self.fatal_error.lock().unwrap_or_else(|p| p.into_inner());
                *slot = Some(format!(
                    "dual-write divergence on run={}: {e}",
                    ctx.run_id.as_str()
                ));
            }
            let _ = self.exporter.export_event(&provider_payload).await;

            use cairn_store::projections::LlmCallTraceReadModel;
            let input_tokens = d.input_tokens.unwrap_or(0);
            let output_tokens = d.output_tokens.unwrap_or(0);
            let cost_micros = ((input_tokens as u64).saturating_mul(500)
                + (output_tokens as u64).saturating_mul(1500))
                / 1_000;
            let trace = cairn_domain::LlmCallTrace {
                trace_id: call_id,
                model_id: d.model_id.clone(),
                prompt_tokens: input_tokens,
                completion_tokens: output_tokens,
                latency_ms: d.latency_ms,
                cost_micros,
                session_id: Some(ctx.session_id.clone()),
                run_id: Some(ctx.run_id.clone()),
                created_at_ms: now,
                is_error: false,
            };
            let _ = self.store.insert_trace(trace).await;
        }
        async fn on_tool_called(
            &self,
            ctx: &cairn_orchestrator::OrchestrationContext,
            name: &str,
            args: Option<&serde_json::Value>,
        ) {
            self.inner.on_tool_called(ctx, name, args).await;
        }
        async fn on_tool_result(
            &self,
            ctx: &cairn_orchestrator::OrchestrationContext,
            name: &str,
            ok: bool,
            out: Option<&serde_json::Value>,
            err: Option<&str>,
            duration_ms: u64,
        ) {
            self.inner
                .on_tool_result(ctx, name, ok, out, err, duration_ms)
                .await;
        }
        async fn on_context_compacted(
            &self,
            ctx: &cairn_orchestrator::OrchestrationContext,
            before: usize,
            after: usize,
            before_tokens: usize,
            after_tokens: usize,
        ) {
            self.inner
                .on_context_compacted(ctx, before, after, before_tokens, after_tokens)
                .await;
        }
        async fn on_plan_proposed(
            &self,
            ctx: &cairn_orchestrator::OrchestrationContext,
            plan: &str,
        ) {
            self.inner.on_plan_proposed(ctx, plan).await;
        }
        async fn on_step_completed(
            &self,
            ctx: &cairn_orchestrator::OrchestrationContext,
            d: &cairn_orchestrator::DecideOutput,
            e: &cairn_orchestrator::ExecuteOutcome,
        ) {
            self.inner.on_step_completed(ctx, d, e).await;
        }
        async fn on_finished(
            &self,
            ctx: &cairn_orchestrator::OrchestrationContext,
            t: &cairn_orchestrator::LoopTermination,
        ) {
            self.inner.on_finished(ctx, t).await;
        }
        fn take_fatal_error(&self) -> Option<String> {
            let mut slot = self.fatal_error.lock().unwrap_or_else(|p| p.into_inner());
            slot.take()
        }
    }

    std::sync::Arc::new(TracingEmitter {
        inner: sse_emitter,
        store: state.runtime.store.clone(),
        exporter: state.otlp_exporter.clone(),
        fatal_error: std::sync::Mutex::new(None),
    })
}

/// Emit a GitHub progress SSE event.
pub(crate) fn emit_github_progress(state: &AppState, data: serde_json::Value) {
    let frame = SseFrame {
        event: cairn_api::sse::SseEventName::GitHubProgress,
        data,
        id: None,
        tenant_id: None,
    };
    let seq = state
        .sse_seq
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut frame_with_id = frame.clone();
    frame_with_id.id = Some(seq.to_string());
    {
        let mut buf = state
            .sse_event_buffer
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if buf.len() >= 10_000 {
            buf.pop_front();
        }
        buf.push_back((seq, frame_with_id));
    }
    let _ = state.runtime_sse_tx.send(frame);
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// POST /v1/webhooks/github
pub(crate) async fn github_webhook_handler(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let github = match &state.github {
        Some(gh) => gh.clone(),
        None => {
            return AppApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "github_not_configured",
                "GitHub App integration is not configured.",
            )
            .into_response();
        }
    };

    let signature = match headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
    {
        Some(sig) => sig.to_owned(),
        None => {
            tracing::warn!("GitHub webhook: missing X-Hub-Signature-256 header");
            return AppApiError::new(
                StatusCode::UNAUTHORIZED,
                "missing_signature",
                "X-Hub-Signature-256 header is required",
            )
            .into_response();
        }
    };

    if cairn_github::verify_signature(&signature, github.webhook_secret.as_bytes(), &body).is_err()
    {
        tracing::warn!("GitHub webhook: invalid signature");
        return AppApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_signature",
            "Webhook signature verification failed",
        )
        .into_response();
    }

    let event_type = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_owned();
    let delivery_id = headers
        .get("x-github-delivery")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_owned();

    let event = match cairn_github::WebhookEvent::parse(&event_type, &delivery_id, &body) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(event_type, delivery_id, error = %e, "GitHub webhook: failed to parse event");
            return AppApiError::new(
                StatusCode::BAD_REQUEST,
                "parse_error",
                format!("Failed to parse webhook event: {e}"),
            )
            .into_response();
        }
    };

    let event_key = event.event_key();
    let repo = event.repository().unwrap_or("unknown").to_owned();
    let installation_id = event.installation_id();

    tracing::info!(
        event_key,
        repo,
        delivery_id,
        ?installation_id,
        "GitHub webhook received"
    );

    let matched_action = {
        let actions = github.event_actions.read().await;
        find_matching_action(&actions, &event)
    };

    let action = match matched_action {
        Some(action) => action,
        None => {
            tracing::debug!(event_key, "GitHub webhook: no matching action configured");
            return Json(serde_json::json!({
                "status": "ignored",
                "event": event_key,
                "reason": "no matching action configured",
            }))
            .into_response();
        }
    };

    match action.action {
        WebhookAction::Ignore => Json(serde_json::json!({
            "status": "ignored",
            "event": event_key,
        }))
        .into_response(),
        WebhookAction::Acknowledge => {
            if let Some(inst_id) = installation_id {
                let github_clone = github.clone();
                let event_key_clone = event_key.clone();
                tokio::spawn(async move {
                    if let Err(e) = acknowledge_event(&github_clone, inst_id, &event).await {
                        tracing::warn!(event = event_key_clone, error = %e, "Failed to acknowledge");
                    }
                });
            }
            Json(serde_json::json!({
                "status": "acknowledged",
                "event": event_key,
            }))
            .into_response()
        }
        WebhookAction::CreateAndOrchestrate => {
            let state_clone = state.clone();
            let github_clone = github.clone();
            let event_key_clone = event_key.clone();
            let delivery_id_clone = delivery_id.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    process_webhook_orchestrate(&state_clone, &github_clone, &event).await
                {
                    tracing::error!(event = event_key_clone, error = %e, "Webhook processing failed");
                }
            });
            Json(serde_json::json!({
                "status": "accepted",
                "event": event_key,
                "delivery_id": delivery_id_clone,
            }))
            .into_response()
        }
    }
}

pub(crate) async fn list_webhook_actions_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let github = match &state.github {
        Some(gh) => gh,
        None => {
            return Json(serde_json::json!({"actions": [], "github_configured": false}))
                .into_response();
        }
    };
    let actions = github.event_actions.read().await;
    Json(serde_json::json!({"actions": *actions, "github_configured": true})).into_response()
}

pub(crate) async fn set_webhook_actions_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SetWebhookActionsRequest>,
) -> impl IntoResponse {
    let github = match &state.github {
        Some(gh) => gh,
        None => {
            return AppApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "github_not_configured",
                "GitHub App integration is not configured",
            )
            .into_response();
        }
    };
    let mut actions = github.event_actions.write().await;
    *actions = body.actions;
    let count = actions.len();
    tracing::info!(count, "GitHub webhook actions updated");
    Json(serde_json::json!({"status": "ok", "actions_count": count})).into_response()
}

pub(crate) async fn github_scan_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ScanRequest>,
) -> impl IntoResponse {
    let github = match &state.github {
        Some(gh) => gh.clone(),
        None => {
            return AppApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "github_not_configured",
                "GitHub App integration is not configured",
            )
            .into_response();
        }
    };
    let (owner, repo_name) = match body.repo.split_once('/') {
        Some(pair) => pair,
        None => {
            return AppApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid_repo",
                "repo must be owner/repo format",
            )
            .into_response();
        }
    };
    let installation_id = match body.installation_id {
        Some(id) => id,
        None => match github.credentials.list_installations(&github.http).await {
            Ok(installs) => match installs.iter().find(|i| i.account.login == owner) {
                Some(i) => i.id,
                None if !installs.is_empty() => installs[0].id,
                None => {
                    return AppApiError::new(
                        StatusCode::BAD_REQUEST,
                        "no_installation",
                        format!("No GitHub App installation found for {owner}"),
                    )
                    .into_response();
                }
            },
            Err(e) => {
                return AppApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "github_api_error",
                    format!("Failed to list installations: {e}"),
                )
                .into_response();
            }
        },
    };
    let client = github.client_for_installation(installation_id).await;
    let labels_str = body.labels.as_deref();
    let per_page = body.limit.unwrap_or(30).min(100);
    let issues = match client
        .list_issues(owner, repo_name, Some("open"), labels_str, per_page)
        .await
    {
        Ok(issues) => issues,
        Err(e) => {
            return AppApiError::new(
                StatusCode::BAD_GATEWAY,
                "github_api_error",
                format!("Failed to list issues: {e}"),
            )
            .into_response();
        }
    };
    if issues.is_empty() {
        return Json(serde_json::json!({"status": "no_issues", "repo": body.repo, "message": "No open issues found matching filters"})).into_response();
    }
    let issues: Vec<_> = issues
        .into_iter()
        .filter(|i| !i.html_url.contains("/pull/"))
        .collect();
    let issue_count = issues.len();
    // T6a-C5: same per-installation mapping as the webhook path — reject
    // when no mapping exists rather than leaking scans into a shared
    // default_tenant project.
    let project = match github.project_for_installation(installation_id).await {
        Some(p) => p,
        None => match default_github_project_from_env() {
            Some(p) => {
                tracing::warn!(
                    installation_id,
                    repo = %body.repo,
                    "scan routed to CAIRN_GITHUB_DEFAULT_PROJECT — configure \
                     CAIRN_GITHUB_INSTALLATION_<id>_PROJECT for proper tenant routing"
                );
                p
            }
            None => {
                return AppApiError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "no_installation_project",
                    format!(
                        "No project mapping for installation {installation_id}. \
                         Set CAIRN_GITHUB_INSTALLATION_{installation_id}_PROJECT or \
                         CAIRN_GITHUB_DEFAULT_PROJECT (tenant/workspace/project)."
                    ),
                )
                .into_response();
            }
        },
    };
    let mut queued = Vec::new();
    for issue in &issues {
        let session_id_str = format!("gh-{}-{}-issue-{}", owner, repo_name, issue.number);
        let session_id = cairn_domain::SessionId::new(&session_id_str);
        if cairn_store::projections::SessionReadModel::get(
            state.runtime.store.as_ref(),
            &session_id,
        )
        .await
        .unwrap_or(None)
        .is_none()
        {
            if let Err(e) = state
                .runtime
                .sessions
                .create(&project, session_id.clone())
                .await
            {
                tracing::warn!(issue = issue.number, error = %e, "Failed to create session");
                continue;
            }
        }
        let run_id_str = format!("{}-scan-run", session_id_str);
        let run_id = cairn_domain::RunId::new(&run_id_str);
        if cairn_store::projections::RunReadModel::get(state.runtime.store.as_ref(), &run_id)
            .await
            .unwrap_or(None)
            .is_some()
        {
            tracing::info!(issue = issue.number, "Run already exists");
            continue;
        }
        match state
            .runtime
            .runs
            .start(&project, &session_id, run_id.clone(), None)
            .await
        {
            Ok(_) => {
                queued.push(QueuedIssue {
                    issue_number: issue.number,
                    title: issue.title.clone(),
                    session_id: session_id_str.clone(),
                    run_id: run_id_str.clone(),
                });
            }
            Err(e) => {
                tracing::warn!(issue = issue.number, error = %e, "Failed to create run");
            }
        }
    }
    let queued_count = queued.len();
    tracing::info!(
        repo = body.repo,
        total_issues = issue_count,
        queued = queued_count,
        "GitHub scan: issues queued for processing"
    );
    if !queued.is_empty() {
        let mut queue = github.issue_queue.write().await;
        for item in &queued {
            queue.push_back(IssueQueueEntry {
                repo: body.repo.clone(),
                installation_id,
                issue_number: item.issue_number,
                title: item.title.clone(),
                session_id: item.session_id.clone(),
                run_id: item.run_id.clone(),
                status: IssueQueueStatus::Pending,
            });
        }
    }
    Json(serde_json::json!({"status": "queued", "repo": body.repo, "total_issues": issue_count, "queued": queued_count, "issues": queued})).into_response()
}

pub(crate) async fn process_issue_queue(state: Arc<AppState>, github: Arc<GitHubIntegration>) {
    if github
        .queue_running
        .swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        tracing::debug!("Queue dispatcher already running");
        return;
    }
    tracing::info!(
        max_concurrent = github
            .max_concurrent
            .load(std::sync::atomic::Ordering::SeqCst),
        "Queue dispatcher started"
    );
    loop {
        if github
            .queue_paused
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            continue;
        }
        let entry = {
            let mut queue = github.issue_queue.write().await;
            let next = queue
                .iter_mut()
                .find(|e| e.status == IssueQueueStatus::Pending);
            match next {
                Some(e) => {
                    e.status = IssueQueueStatus::Processing;
                    e.clone()
                }
                None => {
                    let has_processing = queue
                        .iter()
                        .any(|e| e.status == IssueQueueStatus::Processing);
                    if !has_processing {
                        tracing::info!("Queue empty");
                        github
                            .queue_running
                            .store(false, std::sync::atomic::Ordering::SeqCst);
                        return;
                    }
                    drop(queue);
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    continue;
                }
            }
        };
        let permit = github.run_semaphore.clone().acquire_owned().await;
        let Ok(permit) = permit else {
            github
                .queue_running
                .store(false, std::sync::atomic::Ordering::SeqCst);
            return;
        };
        emit_github_progress(
            &state,
            serde_json::json!({"action": "issue_started", "issue": entry.issue_number, "title": entry.title, "run_id": entry.run_id}),
        );
        tracing::info!(
            issue = entry.issue_number,
            run_id = entry.run_id,
            "Dispatching issue to orchestrator"
        );
        let state_clone = state.clone();
        let github_clone = github.clone();
        tokio::spawn(async move {
            let result = orchestrate_single_issue(&state_clone, &github_clone, &entry).await;
            let final_status = match result {
                Ok(status) => status,
                Err(e) => {
                    tracing::error!(issue = entry.issue_number, error = %e, "Orchestration failed");
                    IssueQueueStatus::Failed(e)
                }
            };
            update_queue_status(&github_clone, entry.issue_number, final_status.clone()).await;
            emit_github_progress(
                &state_clone,
                serde_json::json!({"action": "issue_finished", "issue": entry.issue_number, "status": format!("{:?}", final_status), "run_id": entry.run_id}),
            );
            drop(permit);
        });
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

pub(crate) async fn orchestrate_single_issue(
    state: &AppState,
    github: &GitHubIntegration,
    entry: &IssueQueueEntry,
) -> Result<IssueQueueStatus, String> {
    let (owner, repo_name) = entry.repo.split_once('/').unwrap_or(("", &entry.repo));
    let client = github.client_for_installation(entry.installation_id).await;
    let start_msg = format!(
        "Cairn is working on this issue.\n\n- Run: `{}`\n- Track progress in the Cairn dashboard",
        entry.run_id
    );
    let _ = client
        .create_comment(owner, repo_name, entry.issue_number, &start_msg)
        .await;
    let goal = match client.get_issue(owner, repo_name, entry.issue_number).await {
        Ok(issue) => { let body = issue.body.as_deref().unwrap_or(""); format!("## Task\nResolve GitHub Issue #{number} in repository `{repo}` by writing code and opening a pull request.\n\n## Issue\n**{title}**\n\n{body}\n\n## Workflow\nFollow these steps in order.\n\n1. **Explore** -- Use tool_search to find available tools. Use file-reading and search tools to understand the repo structure and find relevant code. Read at least 3-5 files before planning changes.\n\n2. **Plan** -- Identify which files need to change and what the fix or feature looks like. Think through edge cases.\n\n3. **Branch** -- Create a feature branch (e.g. `cairn/issue-{number}`).\n\n4. **Implement** -- Write the code. Make minimal, focused changes. Follow existing code style and conventions in the repo.\n\n5. **Verify** -- If the project has tests, run them. Fix any failures.\n\n6. **Deliver** -- Commit your changes, push the branch, and open a PR that references issue #{number} in the title or body.\n\n7. **Complete** -- After the PR is open, call escalate_to_operator for review, then complete_run with a summary.\n\n## Tips\n- Start by exploring. Do not write code until you understand the codebase.\n- If a tool call fails, read the error and try a different approach. A command that failed once will fail again unless you change something.\n- Write real, working code -- not pseudocode or TODO comments.\n- Keep changes focused on this issue only.\n- All tool calls targeting this repo need: repo=\"{repo}\".\n- Do not call complete_run until you have opened a PR.", number = issue.number, repo = entry.repo, title = issue.title, body = body) }
        Err(_) => format!("Resolve GitHub Issue #{} in repository `{}`. Explore the codebase, write a fix, and open a pull request. Use tool_search to discover available tools.", entry.issue_number, entry.repo),
    };
    let run_id = cairn_domain::RunId::new(&entry.run_id);
    let run = cairn_store::projections::RunReadModel::get(state.runtime.store.as_ref(), &run_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("run {} not found", entry.run_id))?;
    let work_item = cairn_integrations::WorkItem {
        integration_id: "github".into(),
        source_id: entry.installation_id.to_string(),
        external_id: entry.issue_number.to_string(),
        repo: entry.repo.clone(),
        title: entry.title.clone(),
        body: String::new(),
        run_id: entry.run_id.clone(),
        session_id: entry.session_id.clone(),
        status: cairn_integrations::WorkItemStatus::Processing,
    };
    let repo_id = cairn_workspace::RepoId::parse(&entry.repo)
        .map_err(|e| format!("invalid repo id '{}': {}", entry.repo, e.reason()))?;
    let repo_ctx = cairn_domain::RepoAccessContext {
        project: run.project.clone(),
    };
    state
        .project_repo_access
        .allow(
            &repo_ctx,
            &repo_id,
            cairn_domain::ActorRef::Operator {
                operator_id: cairn_domain::OperatorId::new("webhook-pipeline"),
            },
        )
        .await
        .map_err(|e| format!("repo allowlist failed: {}", e.client_message()))?;
    webhook_trigger_orchestration(
        state,
        &run,
        &goal,
        Some(entry.installation_id),
        Some(&work_item),
    )
    .await?;
    let final_state =
        cairn_store::projections::RunReadModel::get(state.runtime.store.as_ref(), &run_id)
            .await
            .ok()
            .flatten()
            .map(|r| r.state)
            .unwrap_or(cairn_domain::RunState::Failed);
    let (status, comment) = match final_state {
        cairn_domain::RunState::Completed => (
            IssueQueueStatus::Completed,
            "Cairn completed work on this issue. Please review the PR.".to_owned(),
        ),
        cairn_domain::RunState::WaitingApproval => (
            IssueQueueStatus::WaitingApproval,
            "Cairn created a PR -- waiting for operator approval in the dashboard.".to_owned(),
        ),
        other => (
            IssueQueueStatus::Failed(format!("run ended in {other:?}")),
            format!("Cairn hit an issue (state: {other:?}). Operator may retry."),
        ),
    };
    let _ = client
        .create_comment(owner, repo_name, entry.issue_number, &comment)
        .await;
    Ok(status)
}

pub(crate) async fn update_queue_status(
    github: &GitHubIntegration,
    issue_number: u64,
    status: IssueQueueStatus,
) {
    let mut queue = github.issue_queue.write().await;
    if let Some(entry) = queue.iter_mut().find(|e| e.issue_number == issue_number) {
        entry.status = status;
    }
}

pub(crate) async fn github_queue_pause_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    if let Some(ref gh) = state.github {
        gh.queue_paused
            .store(true, std::sync::atomic::Ordering::SeqCst);
        emit_github_progress(&state, serde_json::json!({"action": "queue_paused"}));
        Json(serde_json::json!({"status": "paused"})).into_response()
    } else {
        AppApiError::new(StatusCode::SERVICE_UNAVAILABLE, "github_not_configured", "")
            .into_response()
    }
}

pub(crate) async fn github_queue_resume_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let github = match &state.github {
        Some(gh) => gh.clone(),
        None => {
            return AppApiError::new(StatusCode::SERVICE_UNAVAILABLE, "github_not_configured", "")
                .into_response();
        }
    };
    let model_id = {
        let brain = state.runtime.runtime_config.default_brain_model().await;
        if brain.trim().is_empty() || brain == "default" {
            state.runtime.runtime_config.default_generate_model().await
        } else {
            brain
        }
    };
    let model_id = model_id.trim().to_owned();
    if model_id.is_empty() || model_id == "default" {
        return AppApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "no_brain_model",
            "No LLM model configured.",
        )
        .into_response();
    }
    github
        .queue_paused
        .store(false, std::sync::atomic::Ordering::SeqCst);
    emit_github_progress(&state, serde_json::json!({"action": "queue_resumed"}));
    if !github
        .queue_running
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        let state_clone = state.clone();
        let github_clone = github.clone();
        tokio::spawn(async move {
            process_issue_queue(state_clone, github_clone).await;
        });
    }
    Json(serde_json::json!({"status": "resumed"})).into_response()
}

pub(crate) async fn github_queue_skip_handler(
    State(state): State<Arc<AppState>>,
    Path(issue_str): Path<String>,
) -> impl IntoResponse {
    let github = match &state.github {
        Some(gh) => gh,
        None => {
            return AppApiError::new(StatusCode::SERVICE_UNAVAILABLE, "github_not_configured", "")
                .into_response();
        }
    };
    let issue_number: u64 = match issue_str.parse() {
        Ok(n) => n,
        Err(_) => {
            return AppApiError::new(StatusCode::BAD_REQUEST, "invalid_issue", "not a number")
                .into_response();
        }
    };
    let mut queue = github.issue_queue.write().await;
    if let Some(entry) = queue.iter_mut().find(|e| e.issue_number == issue_number) {
        entry.status = IssueQueueStatus::Failed("skipped by operator".into());
        emit_github_progress(
            &state,
            serde_json::json!({"action": "issue_skipped", "issue": issue_number}),
        );
        Json(serde_json::json!({"status": "skipped", "issue": issue_number})).into_response()
    } else {
        AppApiError::new(StatusCode::NOT_FOUND, "not_found", "issue not in queue").into_response()
    }
}

pub(crate) async fn github_queue_retry_handler(
    State(state): State<Arc<AppState>>,
    Path(issue_str): Path<String>,
) -> impl IntoResponse {
    let github = match &state.github {
        Some(gh) => gh.clone(),
        None => {
            return AppApiError::new(StatusCode::SERVICE_UNAVAILABLE, "github_not_configured", "")
                .into_response();
        }
    };
    let issue_number: u64 = match issue_str.parse() {
        Ok(n) => n,
        Err(_) => {
            return AppApiError::new(StatusCode::BAD_REQUEST, "invalid_issue", "not a number")
                .into_response();
        }
    };
    {
        let mut queue = github.issue_queue.write().await;
        if let Some(entry) = queue.iter_mut().find(|e| e.issue_number == issue_number) {
            entry.status = IssueQueueStatus::Pending;
            emit_github_progress(
                &state,
                serde_json::json!({"action": "issue_retried", "issue": issue_number}),
            );
        } else {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "issue not in queue")
                .into_response();
        }
    }
    if !github
        .queue_running
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        let state_clone = state.clone();
        let github_clone = github.clone();
        tokio::spawn(async move {
            process_issue_queue(state_clone, github_clone).await;
        });
    }
    Json(serde_json::json!({"status": "retried", "issue": issue_number})).into_response()
}

pub(crate) async fn github_installations_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let github = match &state.github {
        Some(gh) => gh,
        None => {
            return Json(serde_json::json!({"installations": [], "configured": false}))
                .into_response();
        }
    };
    match github.credentials.list_installations(&github.http).await {
        Ok(installations) => {
            let items: Vec<serde_json::Value> = installations.iter().map(|i| serde_json::json!({"id": i.id, "account": i.account.login, "repository_selection": i.repository_selection})).collect();
            Json(serde_json::json!({"installations": items, "configured": true})).into_response()
        }
        Err(e) => AppApiError::new(
            StatusCode::BAD_GATEWAY,
            "github_api_error",
            format!("Failed to list installations: {e}"),
        )
        .into_response(),
    }
}

pub(crate) async fn set_queue_concurrency_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let github = match &state.github {
        Some(gh) => gh,
        None => {
            return AppApiError::new(StatusCode::SERVICE_UNAVAILABLE, "github_not_configured", "")
                .into_response();
        }
    };
    let max = body["max_concurrent"].as_u64().unwrap_or(3).clamp(1, 20) as u32;
    let old = github
        .max_concurrent
        .swap(max, std::sync::atomic::Ordering::SeqCst);
    if max > old {
        github.run_semaphore.add_permits((max - old) as usize);
    }
    tracing::info!(old, new = max, "Queue concurrency updated");
    Json(serde_json::json!({"max_concurrent": max, "previous": old})).into_response()
}

/// GET /v1/webhooks/github/queue
pub(crate) async fn github_queue_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let github = match &state.github {
        Some(gh) => gh,
        None => {
            return Json(serde_json::json!({
                "queue": [],
                "total": 0,
                "max_concurrent": 0,
                "dispatcher_running": false,
                "github_configured": false,
            }))
            .into_response();
        }
    };

    let queue = github.issue_queue.read().await;
    let items: Vec<serde_json::Value> = queue
        .iter()
        .map(|entry| {
            serde_json::json!({
                "repo": entry.repo,
                "issue_number": entry.issue_number,
                "title": entry.title,
                "session_id": entry.session_id,
                "run_id": entry.run_id,
                "status": format!("{:?}", entry.status),
            })
        })
        .collect();

    let max_concurrent = github
        .max_concurrent
        .load(std::sync::atomic::Ordering::SeqCst);
    let paused = github
        .queue_paused
        .load(std::sync::atomic::Ordering::SeqCst);
    let running = github
        .queue_running
        .load(std::sync::atomic::Ordering::SeqCst);
    let dispatcher_running = running && !paused;

    Json(serde_json::json!({
        "queue": items,
        "total": items.len(),
        "max_concurrent": max_concurrent,
        "dispatcher_running": dispatcher_running,
    }))
    .into_response()
}

// ── POST /v1/integrations/github/verify-installation ───────────────────────
//
// Lets an operator prove a GitHub App installation works without mutating
// server-side state: they paste `app_id`, PEM private key, and
// `installation_id`, and cairn mints a JWT → exchanges it for an
// installation access token → fetches the installation's repo count.
// Success returns `{verified: true, owner, repo_count, expires_at}`.
// Any GitHub-side failure surfaces as 502 `github_api_error` so the UI
// can show the operator exactly why the paste didn't land.

#[derive(serde::Deserialize)]
pub(crate) struct VerifyInstallationRequest {
    pub app_id: u64,
    /// PEM-encoded RSA private key downloaded from the GitHub App page.
    pub private_key: String,
    pub installation_id: u64,
}

#[derive(serde::Serialize)]
struct VerifyInstallationResponse {
    verified: bool,
    owner: String,
    repo_count: u64,
    expires_at: String,
}

#[derive(serde::Deserialize)]
struct InstallationRepositoriesResponse {
    total_count: u64,
}

#[derive(serde::Deserialize)]
struct InstallationLookupResponse {
    #[serde(default)]
    account: Option<InstallationLookupAccount>,
}

#[derive(serde::Deserialize)]
struct InstallationLookupAccount {
    login: String,
}

pub(crate) async fn verify_github_installation_handler(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<VerifyInstallationRequest>,
) -> impl IntoResponse {
    // Build a short-lived HTTP client per verify. The endpoint is
    // operator-triggered (not hot-path) and the GitHub calls only take
    // a few hundred ms, so a dedicated client keeps this path self-
    // contained without depending on AppState plumbing. Bound the
    // total time we'll wait on api.github.com so a stalled TLS/DNS
    // handshake can't pin an axum worker indefinitely.
    let http = match reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "http_client_build_failed",
                format!("could not build HTTP client: {e}"),
            )
            .into_response();
        }
    };
    if body.private_key.trim().is_empty() {
        return AppApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "private_key must not be empty",
        )
        .into_response();
    }

    // 1. Build credentials from the pasted PEM (validates RSA key shape).
    let credentials =
        match cairn_github::AppCredentials::new(body.app_id, body.private_key.as_bytes()) {
            Ok(c) => c,
            Err(e) => {
                return AppApiError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_private_key",
                    format!("could not parse RSA private key: {e}"),
                )
                .into_response();
            }
        };

    // 2. Mint an installation access token.
    let token_manager = cairn_github::InstallationToken::new(
        credentials.clone(),
        body.installation_id,
        http.clone(),
    );
    let (access_token, expires_at) = match token_manager.refresh_with_metadata().await {
        Ok(pair) => pair,
        Err(e) => {
            return AppApiError::new(
                StatusCode::BAD_GATEWAY,
                "github_api_error",
                format!("token exchange failed: {e}"),
            )
            .into_response();
        }
    };

    // 3. Fetch installation metadata (owner login) with the App JWT.
    let jwt = match credentials.generate_jwt() {
        Ok(j) => j,
        Err(e) => {
            return AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "jwt_signing_failed",
                format!("could not sign JWT: {e}"),
            )
            .into_response();
        }
    };
    let installation_url = format!(
        "https://api.github.com/app/installations/{}",
        body.installation_id
    );
    let owner = match http
        .get(&installation_url)
        .header("Authorization", format!("Bearer {jwt}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "cairn-app/verify-installation")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<InstallationLookupResponse>().await {
                Ok(parsed) => parsed
                    .account
                    .map(|a| a.login)
                    .unwrap_or_else(|| "unknown".into()),
                Err(e) => {
                    return AppApiError::new(
                        StatusCode::BAD_GATEWAY,
                        "github_api_error",
                        format!("could not parse installation lookup: {e}"),
                    )
                    .into_response();
                }
            }
        }
        Ok(resp) => {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return AppApiError::new(
                StatusCode::BAD_GATEWAY,
                "github_api_error",
                format!("installation lookup returned {status}: {body_text}"),
            )
            .into_response();
        }
        Err(e) => {
            return AppApiError::new(
                StatusCode::BAD_GATEWAY,
                "github_api_error",
                format!("installation lookup request failed: {e}"),
            )
            .into_response();
        }
    };

    // 4. Fetch accessible repository count using the installation token.
    let repo_count = match http
        .get("https://api.github.com/installation/repositories?per_page=1")
        .header("Authorization", format!("token {access_token}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "cairn-app/verify-installation")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<InstallationRepositoriesResponse>().await {
                Ok(parsed) => parsed.total_count,
                Err(e) => {
                    return AppApiError::new(
                        StatusCode::BAD_GATEWAY,
                        "github_api_error",
                        format!("could not parse repo list: {e}"),
                    )
                    .into_response();
                }
            }
        }
        Ok(resp) => {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return AppApiError::new(
                StatusCode::BAD_GATEWAY,
                "github_api_error",
                format!("repositories lookup returned {status}: {body_text}"),
            )
            .into_response();
        }
        Err(e) => {
            return AppApiError::new(
                StatusCode::BAD_GATEWAY,
                "github_api_error",
                format!("repositories lookup request failed: {e}"),
            )
            .into_response();
        }
    };

    // `expires_at` came back alongside the access token from the first
    // refresh — no need for a redundant second token mint.
    Json(VerifyInstallationResponse {
        verified: true,
        owner,
        repo_count,
        expires_at,
    })
    .into_response()
}
