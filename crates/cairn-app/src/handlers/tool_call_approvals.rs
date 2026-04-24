//! HTTP handlers for the **tool-call** approval flow (PR BP-6).
//!
//! Distinct from the general [`crate::handlers::approvals`] surface,
//! which still owns plan review, prompt releases, and legacy run-level
//! pauses. Tool-call approvals have their own event family
//! (`ToolCallProposed` / `ToolCallApproved` / `ToolCallRejected` /
//! `ToolCallAmended`), their own projection on `cairn-store`
//! ([`cairn_store::projections::ToolCallApprovalReadModel`]), and their
//! own runtime service
//! ([`cairn_runtime::tool_call_approvals::ToolCallApprovalService`]).
//!
//! ## Endpoints
//!
//! | Method | Path                                              | Purpose                                |
//! |--------|---------------------------------------------------|----------------------------------------|
//! | GET    | `/v1/tool-call-approvals`                         | List (filter by run / session / state) |
//! | GET    | `/v1/tool-call-approvals/:call_id`                | Fetch a single record                  |
//! | POST   | `/v1/tool-call-approvals/:call_id/approve`        | Approve (Once or Session + policy)     |
//! | POST   | `/v1/tool-call-approvals/:call_id/reject`         | Reject with optional reason            |
//! | PATCH  | `/v1/tool-call-approvals/:call_id/amend`          | Preview-edit args (non-resolving)      |
//!
//! ## Identity & audit
//!
//! `OperatorId` is **always** derived from the authenticated principal
//! (see [`crate::handlers::admin::audit_actor_id`]). Bodies that smuggle
//! an `operator_id` field disagreeing with the principal are rejected
//! with 400 — the audit trail never trusts user-supplied identity.
//!
//! ## `amend` self-guard
//!
//! An operator cannot amend a proposal whose tool is the amend endpoint
//! itself. This blocks confused-deputy attacks via recursive amendment
//! of an approval mutation.

use std::sync::Arc;

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use cairn_api::auth::AuthPrincipal;
use cairn_api::http::ListResponse;
use cairn_domain::{
    ApprovalMatchPolicy, ApprovalScope, AuditOutcome, OperatorId, ProjectKey, RunId, SessionId,
    ToolCallId,
};
use cairn_runtime::AuditService;
use cairn_store::projections::{
    ToolCallApprovalReadModel, ToolCallApprovalRecord, ToolCallApprovalState,
};
use serde::Deserialize;
use serde_json::Value;

use crate::errors::{runtime_error_response, store_error_response, AppApiError};
use crate::extractors::TenantScope;
use crate::handlers::admin::audit_actor_id;
use crate::state::AppState;

// ── Query / DTO ────────────────────────────────────────────────────────────

/// Query string for the list endpoint.
///
/// At least one of the tenant-bound filters is expected in normal
/// operation (`run_id`, `session_id`, or project triple). If none is
/// supplied the handler falls back to the caller's tenant scope and
/// returns the pending inbox across the default project.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct ListToolCallApprovalsQuery {
    pub tenant_id: Option<String>,
    pub workspace_id: Option<String>,
    pub project_id: Option<String>,
    pub run_id: Option<String>,
    pub session_id: Option<String>,
    /// Filter by state. Accepts `pending | approved | rejected | timeout`.
    pub state: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

impl ListToolCallApprovalsQuery {
    fn limit(&self) -> usize {
        self.limit.unwrap_or(100).min(500)
    }
    fn offset(&self) -> usize {
        self.offset.unwrap_or(0)
    }
    fn state(&self) -> Result<Option<ToolCallApprovalState>, AppApiError> {
        match self.state.as_deref() {
            None => Ok(None),
            Some(raw) => ToolCallApprovalState::parse(raw).map(Some).map_err(|_| {
                AppApiError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "validation_error",
                    format!("unknown tool-call approval state: {raw}"),
                )
            }),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ApproveBody {
    /// Optional override: must match the principal's identity when set.
    /// Present for API symmetry but never trusted — always a tripwire for
    /// impersonation attempts.
    #[serde(default)]
    pub operator_id: Option<String>,
    pub scope: ApproveScope,
    /// If set, the operator-supplied final arguments. Overrides any
    /// prior `amend` payload and the original proposal arguments.
    #[serde(default)]
    pub approved_tool_args: Option<Value>,
}

/// Operator-facing scope DTO.
///
/// `match_policy` is optional on `session`: when omitted the handler
/// falls back to the match policy captured on the original
/// [`ToolCallProposed`] event (see [`ToolCallApprovalRecord::match_policy`]).
/// This keeps the client payload minimal for the common case while still
/// supporting overrides when an operator wants to widen / narrow at
/// approval time.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ApproveScope {
    Once,
    Session {
        #[serde(default)]
        match_policy: Option<ApprovalMatchPolicy>,
    },
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RejectBody {
    #[serde(default)]
    pub operator_id: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct AmendBody {
    #[serde(default)]
    pub operator_id: Option<String>,
    pub new_tool_args: Value,
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Pull the caller's `OperatorId` from the auth principal and reject
/// any request body that smuggles a different `operator_id`.
fn derive_operator_id(
    principal: &AuthPrincipal,
    body_override: Option<&str>,
) -> Result<OperatorId, AppApiError> {
    let actor = audit_actor_id(principal);
    if let Some(supplied) = body_override {
        if supplied != actor {
            return Err(AppApiError::new(
                StatusCode::BAD_REQUEST,
                "identity_mismatch",
                "operator_id in body does not match authenticated principal",
            ));
        }
    }
    Ok(OperatorId::new(actor))
}

/// Fetch the record and enforce tenant visibility. 404 (not 403) on a
/// cross-tenant mismatch so existence is not leaked across tenants.
async fn load_record_visible_to_tenant(
    state: &AppState,
    tenant_scope: &TenantScope,
    call_id: &ToolCallId,
) -> Result<ToolCallApprovalRecord, axum::response::Response> {
    let reader: &dyn ToolCallApprovalReadModel = state.runtime.store.as_ref();
    match reader.get(call_id).await {
        Ok(Some(record))
            if tenant_scope.is_admin || record.project.tenant_id == *tenant_scope.tenant_id() =>
        {
            Ok(record)
        }
        Ok(_) => Err(AppApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "tool-call approval not found",
        )
        .into_response()),
        Err(err) => Err(store_error_response(err)),
    }
}

// ── Handlers ───────────────────────────────────────────────────────────────

/// Upper bound on the candidate set pulled from the projection before
/// the handler applies tenant + state filters and slices down to
/// `limit`/`offset`. Prevents an operator from exfiltrating an
/// unbounded inbox via a single request while still leaving enough
/// headroom that filter-heavy pages aren't silently truncated. Pick
/// is 10x the max page size served out (`limit().min(500)`).
const MAX_LIST_FETCH: usize = 5_000;

/// List tool-call approvals.
///
/// Selection precedence (first match wins):
/// 1. `run_id` → `list_for_run`
/// 2. `session_id` → `list_for_session`
/// 3. project triple → `list_pending_for_project` (operator inbox — only
///    returns records in `pending`, so a `state=approved|rejected` filter
///    here is an empty-set noise request).
///
/// Tenant scope is enforced by filtering the candidate set, then the
/// `state` filter, *then* the `limit`/`offset` slice is applied — so
/// pages are never short or silently truncated by post-fetch retain()s.
pub(crate) async fn list_tool_call_approvals_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Query(query): Query<ListToolCallApprovalsQuery>,
) -> impl IntoResponse {
    let state_filter = match query.state() {
        Ok(s) => s,
        Err(err) => return err.into_response(),
    };
    let reader: &dyn ToolCallApprovalReadModel = state.runtime.store.as_ref();
    let limit = query.limit();
    let offset = query.offset();

    // Resolve the fallback project triple from the caller's tenant scope
    // (never the process-wide default). A non-admin caller sending a
    // `tenant_id` that disagrees with their principal is rejected up
    // front so the filter can never yield a silently-empty result
    // because the query hit a foreign tenant and the retain() dropped
    // everything.
    let project = ProjectKey::new(
        query
            .tenant_id
            .as_deref()
            .unwrap_or_else(|| tenant_scope.tenant_id().as_str()),
        query
            .workspace_id
            .as_deref()
            .unwrap_or(crate::state::DEFAULT_WORKSPACE_ID),
        query
            .project_id
            .as_deref()
            .unwrap_or(crate::state::DEFAULT_PROJECT_ID),
    );
    if !tenant_scope.is_admin && project.tenant_id != *tenant_scope.tenant_id() {
        return crate::errors::tenant_scope_mismatch_error().into_response();
    }

    // Branch selection:
    //
    // * run_id / session_id — the projection currently exposes no
    //   pagination for these paths, so we read the full list and slice
    //   in-memory. A single run's approval count is bounded in practice
    //   by the agent's tool-budget; refuse pagination requests that
    //   would reach past `MAX_LIST_FETCH` rather than silently produce
    //   an empty tail.
    //
    // * project inbox — the projection only returns `pending` records,
    //   *and* it accepts `(limit, offset)`. When the caller has no
    //   state filter (or filters to `pending`) we push `offset`/`limit`
    //   down to the projection, so pagination scales past
    //   `MAX_LIST_FETCH`. For any other state filter the inbox always
    //   yields an empty set (the projection doesn't return resolved
    //   records on this path); return `[]` up front instead of doing an
    //   expensive fetch + filter that we already know will be empty.
    let is_project_inbox = query.run_id.is_none() && query.session_id.is_none();
    let inbox_state_is_pending_only =
        matches!(state_filter, None | Some(ToolCallApprovalState::Pending));

    // Reject non-pending state filter on the project-inbox path up
    // front (projection has no `list_all_for_project` equivalent; a
    // caller wanting resolved records must scope by run_id or
    // session_id). Silently returning `[]` would surprise operators who
    // see approved records on the run view but an empty project inbox.
    if is_project_inbox && !inbox_state_is_pending_only {
        return AppApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unsupported_filter",
            "listing by project triple only returns pending records; \
             use run_id or session_id to list resolved state",
        )
        .into_response();
    }

    let push_down_inbox_pagination = is_project_inbox && inbox_state_is_pending_only;

    if offset.saturating_add(limit) > MAX_LIST_FETCH && !push_down_inbox_pagination {
        return AppApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "pagination_out_of_range",
            format!(
                "offset + limit must be <= {MAX_LIST_FETCH} when listing by run_id / session_id"
            ),
        )
        .into_response();
    }

    let records: Result<Vec<ToolCallApprovalRecord>, _> = if let Some(rid) = query.run_id.as_deref()
    {
        reader.list_for_run(&RunId::new(rid)).await
    } else if let Some(sid) = query.session_id.as_deref() {
        reader.list_for_session(&SessionId::new(sid)).await
    } else {
        // Inbox path: push `offset`/`limit` to the projection, fetch
        // `limit + 1` to detect has_more in a single round trip.
        reader
            .list_pending_for_project(&project, limit.saturating_add(1), offset)
            .await
    };

    match records {
        Ok(mut items) => {
            if !tenant_scope.is_admin {
                let caller = tenant_scope.tenant_id().clone();
                items.retain(|r| r.project.tenant_id == caller);
            }
            if let Some(st) = state_filter {
                items.retain(|r| r.state == st);
            }

            // Inbox push-down path: we fetched `limit + 1` starting at
            // `offset` already — trim the peek row and report has_more
            // directly.
            if push_down_inbox_pagination {
                let has_more = items.len() > limit;
                items.truncate(limit);
                return (StatusCode::OK, Json(ListResponse { items, has_more })).into_response();
            }

            // run_id / session_id path: apply `limit`/`offset` in memory
            // against the tenant- + state-filtered set.
            let total_after_filter = items.len();
            let end = offset.saturating_add(limit).min(total_after_filter);
            let start = offset.min(total_after_filter);
            let page: Vec<_> = items.drain(start..end).collect();
            let has_more = end < total_after_filter;
            (
                StatusCode::OK,
                Json(ListResponse {
                    items: page,
                    has_more,
                }),
            )
                .into_response()
        }
        Err(err) => store_error_response(err),
    }
}

pub(crate) async fn get_tool_call_approval_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(call_id): Path<String>,
) -> impl IntoResponse {
    let call_id = ToolCallId::new(call_id);
    match load_record_visible_to_tenant(&state, &tenant_scope, &call_id).await {
        Ok(record) => (StatusCode::OK, Json(record)).into_response(),
        Err(resp) => resp,
    }
}

pub(crate) async fn approve_tool_call_approval_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Extension(principal): Extension<AuthPrincipal>,
    Path(call_id): Path<String>,
    Json(body): Json<ApproveBody>,
) -> impl IntoResponse {
    let call_id = ToolCallId::new(call_id);

    // 1. Tenant scope + load the record so we can fall back on its
    //    `match_policy` when `Session` omits one.
    let record = match load_record_visible_to_tenant(&state, &tenant_scope, &call_id).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    // 2. Principal-derived operator id; body override is a no-op at
    //    best, a 400 at worst.
    let operator_id = match derive_operator_id(&principal, body.operator_id.as_deref()) {
        Ok(o) => o,
        Err(err) => return err.into_response(),
    };

    // 3. Resolve scope. `Session` without `match_policy` falls back to
    //    the proposal's captured default.
    let scope = match body.scope {
        ApproveScope::Once => ApprovalScope::Once,
        ApproveScope::Session { match_policy: None } => ApprovalScope::Session {
            match_policy: record.match_policy.clone(),
        },
        ApproveScope::Session {
            match_policy: Some(policy),
        } => ApprovalScope::Session {
            match_policy: policy,
        },
    };

    let before = crate::handlers::sse::current_event_head(&state).await;
    match state
        .runtime
        .tool_call_approvals
        .approve(
            call_id.clone(),
            operator_id,
            scope.clone(),
            body.approved_tool_args.clone(),
        )
        .await
    {
        Ok(()) => {
            let audit = state
                .runtime
                .audits
                .record(
                    record.project.tenant_id.clone(),
                    audit_actor_id(&principal),
                    "approve_tool_call".to_owned(),
                    "tool_call_approval".to_owned(),
                    call_id.to_string(),
                    AuditOutcome::Success,
                    serde_json::json!({
                        "call_id": call_id.to_string(),
                        "scope": scope,
                        "had_args_override": body.approved_tool_args.is_some(),
                    }),
                )
                .await;
            if let Err(err) = audit {
                return runtime_error_response(err);
            }
            crate::handlers::sse::publish_runtime_frames_since(&state, before).await;
            // Re-fetch the record to return the new state to the caller.
            let reader: &dyn ToolCallApprovalReadModel = state.runtime.store.as_ref();
            match reader.get(&call_id).await {
                Ok(Some(updated)) => (StatusCode::OK, Json(updated)).into_response(),
                Ok(None) => AppApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "approval record disappeared after approve",
                )
                .into_response(),
                Err(err) => store_error_response(err),
            }
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn reject_tool_call_approval_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Extension(principal): Extension<AuthPrincipal>,
    Path(call_id): Path<String>,
    Json(body): Json<RejectBody>,
) -> impl IntoResponse {
    let call_id = ToolCallId::new(call_id);
    let record = match load_record_visible_to_tenant(&state, &tenant_scope, &call_id).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    let operator_id = match derive_operator_id(&principal, body.operator_id.as_deref()) {
        Ok(o) => o,
        Err(err) => return err.into_response(),
    };

    let before = crate::handlers::sse::current_event_head(&state).await;
    match state
        .runtime
        .tool_call_approvals
        .reject(call_id.clone(), operator_id, body.reason.clone())
        .await
    {
        Ok(()) => {
            let audit = state
                .runtime
                .audits
                .record(
                    record.project.tenant_id.clone(),
                    audit_actor_id(&principal),
                    "reject_tool_call".to_owned(),
                    "tool_call_approval".to_owned(),
                    call_id.to_string(),
                    AuditOutcome::Success,
                    serde_json::json!({
                        "call_id": call_id.to_string(),
                        "reason": body.reason,
                    }),
                )
                .await;
            if let Err(err) = audit {
                return runtime_error_response(err);
            }
            crate::handlers::sse::publish_runtime_frames_since(&state, before).await;
            let reader: &dyn ToolCallApprovalReadModel = state.runtime.store.as_ref();
            match reader.get(&call_id).await {
                Ok(Some(updated)) => (StatusCode::OK, Json(updated)).into_response(),
                Ok(None) => AppApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "approval record disappeared after reject",
                )
                .into_response(),
                Err(err) => store_error_response(err),
            }
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn amend_tool_call_approval_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Extension(principal): Extension<AuthPrincipal>,
    Path(call_id): Path<String>,
    Json(body): Json<AmendBody>,
) -> impl IntoResponse {
    let call_id = ToolCallId::new(call_id);
    let record = match load_record_visible_to_tenant(&state, &tenant_scope, &call_id).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    // Self-amend guard: operators cannot amend a proposal whose tool
    // *is* the amend endpoint. Blocks confused-deputy via recursive
    // amendment of approval mutations.
    if record.tool_name == "amend_approval" {
        return AppApiError::new(
            StatusCode::FORBIDDEN,
            "self_amend_forbidden",
            "cannot amend a proposal targeting the amend_approval tool",
        )
        .into_response();
    }

    let operator_id = match derive_operator_id(&principal, body.operator_id.as_deref()) {
        Ok(o) => o,
        Err(err) => return err.into_response(),
    };

    let before = crate::handlers::sse::current_event_head(&state).await;
    match state
        .runtime
        .tool_call_approvals
        .amend(call_id.clone(), operator_id, body.new_tool_args.clone())
        .await
    {
        Ok(()) => {
            let audit = state
                .runtime
                .audits
                .record(
                    record.project.tenant_id.clone(),
                    audit_actor_id(&principal),
                    "amend_tool_call".to_owned(),
                    "tool_call_approval".to_owned(),
                    call_id.to_string(),
                    AuditOutcome::Success,
                    serde_json::json!({ "call_id": call_id.to_string() }),
                )
                .await;
            if let Err(err) = audit {
                return runtime_error_response(err);
            }
            crate::handlers::sse::publish_runtime_frames_since(&state, before).await;
            let reader: &dyn ToolCallApprovalReadModel = state.runtime.store.as_ref();
            match reader.get(&call_id).await {
                Ok(Some(updated)) => (StatusCode::OK, Json(updated)).into_response(),
                Ok(None) => AppApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "approval record disappeared after amend",
                )
                .into_response(),
                Err(err) => store_error_response(err),
            }
        }
        Err(err) => runtime_error_response(err),
    }
}
