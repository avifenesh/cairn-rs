//! Unified approval surface (F45).
//!
//! One public HTTP family — `/v1/approvals/*` — covers **both** approval
//! kinds the platform supports:
//!
//!   1. **plan approvals** (`cairn_store::projections::ApprovalRecord`):
//!      plan review, release gates, run-level pauses. Resolved via
//!      `approve` / `reject`.
//!   2. **tool-call approvals** (`cairn_store::projections::ToolCallApprovalRecord`):
//!      per-tool-call gating with operator amendment, session widening,
//!      and match policies. Resolved via `approve` / `reject`; can be
//!      `amend`ed before resolving.
//!
//! Wire responses use a discriminator:
//!
//! ```json
//! { "kind": "plan",      "approval_id": "...", ... }
//! { "kind": "tool_call", "call_id": "...",     ... }
//! ```
//!
//! Endpoints:
//!
//! | Method | Path                               | Behaviour                                   |
//! |--------|------------------------------------|---------------------------------------------|
//! | GET    | `/v1/approvals`                    | List both kinds, merged, newest first       |
//! | GET    | `/v1/approvals/:id`                | Fetch by id (tool-call first, then plan)    |
//! | POST   | `/v1/approvals/:id/approve`        | Approve (any kind) with kind-aware body     |
//! | POST   | `/v1/approvals/:id/reject`         | Reject (any kind)                           |
//! | POST   | `/v1/approvals/:id/deny`           | Alias of reject (legacy)                    |
//! | PATCH  | `/v1/approvals/:id/amend`          | Amend tool-call args (404 if plan approval) |
//!
//! The pre-F45 `/v1/tool-call-approvals/*` family is still served for
//! zero-downtime migration but returns **308 Permanent Redirect** to the
//! unified path.

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
    ApprovalDecision, ApprovalId, ApprovalMatchPolicy, ApprovalRequirement, ApprovalScope,
    AuditOutcome, OperatorId, ProjectKey, RunId, SessionId, TaskId, TenantId, ToolCallId,
    WorkspaceRole,
};
use cairn_runtime::{ApprovalPolicyService, ApprovalService, AuditService};
use cairn_store::projections::{
    ApprovalRecord, ToolCallApprovalReadModel, ToolCallApprovalRecord, ToolCallApprovalState,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::{runtime_error_response, store_error_response, AppApiError};
use crate::extractors::TenantScope;
use crate::handlers::admin::audit_actor_id;
use crate::state::AppState;

const DEFAULT_TENANT_ID: &str = "default_tenant";

/// Upper bound on the candidate set pulled from the tool-call projection
/// before the handler applies tenant + state filters and slices down to
/// `limit`/`offset`. Mirrors the constant previously used by the
/// standalone tool-call list handler. Pick is 10x the max page size.
const MAX_LIST_FETCH: usize = 5_000;

// ── Wire DTO: unified discriminated-union record ────────────────────────────

/// Kind filter used by `GET /v1/approvals` and the unified row envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ApprovalKind {
    Plan,
    ToolCall,
}

/// Unified wire shape. The `kind` discriminator disambiguates the two
/// payloads for consumers. The inner record is flattened so existing
/// field names stay stable for clients migrating off
/// `/v1/tool-call-approvals/*`.
///
/// Both variants are boxed to keep the enum compact and equal-sized —
/// the records are ~224 and ~488 bytes respectively, which triggers
/// `clippy::large_enum_variant` if either is stored inline. `Box<T>`
/// serializes transparently via serde, so the wire shape is unchanged.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum UnifiedApproval {
    Plan(Box<ApprovalRecord>),
    ToolCall(Box<ToolCallApprovalRecord>),
}

impl UnifiedApproval {
    fn created_at_ms(&self) -> u64 {
        match self {
            Self::Plan(r) => r.created_at,
            Self::ToolCall(r) => r.proposed_at_ms,
        }
    }
}

// ── Query DTOs ──────────────────────────────────────────────────────────────

/// Query string for the unified list endpoint.
///
/// `kind` narrows the merge (`plan` | `tool_call` | absent = both).
/// `state` is applied uniformly: `pending`, `approved`, `rejected`,
/// `timeout`. For plan approvals the `timeout` state collapses to
/// `rejected` semantically (plan approvals don't time out on their own).
///
/// Scope fields fall back to the authenticated principal's tenant when
/// omitted. A non-admin caller supplying a `tenant_id` different from
/// their principal is rejected up front.
/// F44 carry-forward: unknown query keys surface as 400 rather than
/// being silently dropped. The dogfood bug used `?status=pending`
/// (typo of `state=pending`); without `deny_unknown_fields` the handler
/// would happily return a full unfiltered list while the operator
/// thinks they filtered.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListApprovalsQuery {
    pub tenant_id: Option<String>,
    pub workspace_id: Option<String>,
    pub project_id: Option<String>,
    pub run_id: Option<String>,
    pub session_id: Option<String>,
    /// `plan | tool_call` — narrows merge. Absent means both.
    pub kind: Option<String>,
    /// `pending | approved | rejected | timeout` — applied to both kinds.
    pub state: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

impl ListApprovalsQuery {
    fn limit(&self) -> usize {
        self.limit.unwrap_or(100).min(500)
    }
    fn offset(&self) -> usize {
        self.offset.unwrap_or(0)
    }
    fn kind(&self) -> Result<Option<ApprovalKind>, AppApiError> {
        match self.kind.as_deref() {
            None => Ok(None),
            Some("plan") => Ok(Some(ApprovalKind::Plan)),
            Some("tool_call") => Ok(Some(ApprovalKind::ToolCall)),
            Some(other) => Err(AppApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_error",
                format!("unknown approval kind: {other}; expected plan | tool_call"),
            )),
        }
    }
    fn state(&self) -> Result<Option<ToolCallApprovalState>, AppApiError> {
        match self.state.as_deref() {
            None => Ok(None),
            Some(raw) => ToolCallApprovalState::parse(raw).map(Some).map_err(|_| {
                AppApiError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "validation_error",
                    format!("unknown approval state: {raw}"),
                )
            }),
        }
    }
}

// Approval-policy DTOs (unchanged from pre-F45 — policies are a separate
// resource, not part of the unification scope).

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CreateApprovalPolicyRequest {
    pub tenant_id: Option<String>,
    pub name: String,
    pub required_approvers: u32,
    pub allowed_approver_roles: Vec<WorkspaceRole>,
    pub auto_approve_after_ms: Option<u64>,
    pub auto_reject_after_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct ApprovalPolicyListQuery {
    pub tenant_id: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

impl ApprovalPolicyListQuery {
    pub(crate) fn tenant_id(&self) -> TenantId {
        TenantId::new(self.tenant_id.as_deref().unwrap_or(DEFAULT_TENANT_ID))
    }
    pub(crate) fn limit(&self) -> usize {
        self.limit.unwrap_or(100).min(500)
    }
    pub(crate) fn offset(&self) -> usize {
        self.offset.unwrap_or(0)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[allow(dead_code)]
pub(crate) struct RequestApprovalRequest {
    pub tenant_id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub approval_id: String,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub requirement: Option<ApprovalRequirement>,
    pub policy_id: Option<String>,
}

impl RequestApprovalRequest {
    pub(crate) fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[allow(dead_code)]
pub(crate) struct DelegateApprovalRequest {
    pub delegated_to: String,
}

// ── Approve / Reject / Amend bodies ─────────────────────────────────────────

/// Optional per-kind fields on the unified `approve` endpoint. Legacy
/// plan approvals ignore every field; tool-call approvals read `scope`
/// and `approved_tool_args` as in the pre-F45 BP-6 surface.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct UnifiedApproveBody {
    /// Principal-override echo — must match the authenticated principal
    /// when set, else 400 `identity_mismatch`. Never trusted otherwise.
    #[serde(default)]
    pub operator_id: Option<String>,
    /// Required for tool-call approvals; ignored for plan approvals.
    #[serde(default)]
    pub scope: Option<ApproveScope>,
    /// Tool-call only — override the final arguments at resolution time.
    #[serde(default)]
    pub approved_tool_args: Option<Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct UnifiedRejectBody {
    #[serde(default)]
    pub operator_id: Option<String>,
    /// Optional free-text reason logged on the audit entry and, for
    /// tool-call approvals, surfaced to the agent.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct AmendBody {
    #[serde(default)]
    pub operator_id: Option<String>,
    pub new_tool_args: Value,
}

/// Operator-facing scope DTO. Same shape as the pre-F45 tool-call
/// handler; `match_policy` is optional on `session` and inherited from
/// the proposal when omitted.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ApproveScope {
    Once,
    Session {
        #[serde(default)]
        match_policy: Option<ApprovalMatchPolicy>,
    },
}

// ── Shared helpers ──────────────────────────────────────────────────────────

async fn load_plan_approval_visible_to_tenant(
    state: &AppState,
    tenant_scope: &TenantScope,
    approval_id: &ApprovalId,
) -> Result<Option<ApprovalRecord>, axum::response::Response> {
    match state.runtime.approvals.get(approval_id).await {
        Ok(Some(record))
            if tenant_scope.is_admin || record.project.tenant_id == *tenant_scope.tenant_id() =>
        {
            Ok(Some(record))
        }
        // Cross-tenant mismatches and plain misses both collapse to
        // `None` here so the caller can try the tool-call store without
        // returning a 404 prematurely.
        Ok(_) => Ok(None),
        Err(err) => Err(runtime_error_response(err)),
    }
}

async fn load_tool_call_visible_to_tenant(
    state: &AppState,
    tenant_scope: &TenantScope,
    call_id: &ToolCallId,
) -> Result<Option<ToolCallApprovalRecord>, axum::response::Response> {
    let reader: &dyn ToolCallApprovalReadModel = state.runtime.store.as_ref();
    match reader.get(call_id).await {
        Ok(Some(record))
            if tenant_scope.is_admin || record.project.tenant_id == *tenant_scope.tenant_id() =>
        {
            Ok(Some(record))
        }
        Ok(_) => Ok(None),
        Err(err) => Err(store_error_response(err)),
    }
}

/// Resolve the incoming `:id` path parameter to whichever approval kind
/// actually exists. Tool-call records are checked first: they are more
/// numerous in practice and carry richer state. A miss in both stores
/// yields 404.
async fn resolve_approval_by_id(
    state: &AppState,
    tenant_scope: &TenantScope,
    raw_id: &str,
) -> Result<UnifiedApproval, axum::response::Response> {
    let call_id = ToolCallId::new(raw_id);
    match load_tool_call_visible_to_tenant(state, tenant_scope, &call_id).await {
        Ok(Some(record)) => return Ok(UnifiedApproval::ToolCall(Box::new(record))),
        Ok(None) => {}
        Err(resp) => return Err(resp),
    }
    let approval_id = ApprovalId::new(raw_id);
    match load_plan_approval_visible_to_tenant(state, tenant_scope, &approval_id).await {
        Ok(Some(record)) => Ok(UnifiedApproval::Plan(Box::new(record))),
        Ok(None) => Err(
            AppApiError::new(StatusCode::NOT_FOUND, "not_found", "approval not found")
                .into_response(),
        ),
        Err(resp) => Err(resp),
    }
}

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

// ── Handlers — unified surface ──────────────────────────────────────────────

/// `GET /v1/approvals` — merged list of plan + tool-call approvals.
///
/// Filter precedence:
/// 1. `kind=plan|tool_call` — narrows to one source.
/// 2. `run_id` / `session_id` — tool-call-native filters. Plan
///    approvals carry `run_id` only; the `session_id` filter excludes
///    them entirely.
/// 3. `state` — applied to both sources. Plan approvals map: `pending`
///    = no decision; `approved`/`rejected` = matching decision; `timeout`
///    = empty for plan (plan records don't time out on their own).
/// 4. Scope (tenant/workspace/project) defaults to the principal's
///    tenant; a non-admin caller passing a different `tenant_id` is
///    rejected.
pub(crate) async fn list_approvals_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Query(query): Query<ListApprovalsQuery>,
) -> impl IntoResponse {
    let kind_filter = match query.kind() {
        Ok(k) => k,
        Err(err) => return err.into_response(),
    };
    let state_filter = match query.state() {
        Ok(s) => s,
        Err(err) => return err.into_response(),
    };

    let limit = query.limit();
    let offset = query.offset();

    // F44 carry-forward: an admin caller with NO scope hints at all is
    // asking for the cross-tenant super-admin inbox. Without this path
    // the tenant fallback below would point at the admin service
    // account's own tenant (typically `default`) — never the canonical
    // `default_tenant` cell where orchestrate + UI write — and the
    // inbox would render empty while by-id still returned live records.
    let admin_cross_tenant_inbox = tenant_scope.is_admin
        && query.run_id.is_none()
        && query.session_id.is_none()
        && query.tenant_id.is_none()
        && query.workspace_id.is_none()
        && query.project_id.is_none();

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

    // F44 carry-forward: listings that fall back to the project-inbox
    // / admin-inbox paths (no run_id, no session_id) only ever return
    // pending tool-call records from the projection. A non-pending
    // `state` filter on those paths silently produced `[]` pre-F44 and
    // surprised operators — reject up front with 422 so the caller
    // knows to scope by run_id / session_id instead.
    let is_inbox_path = query.run_id.is_none() && query.session_id.is_none();
    let inbox_state_is_pending_only =
        matches!(state_filter, None | Some(ToolCallApprovalState::Pending));
    let want_tool = matches!(kind_filter, None | Some(ApprovalKind::ToolCall));
    if want_tool && is_inbox_path && !inbox_state_is_pending_only {
        return AppApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unsupported_filter",
            "inbox listings only return pending records; \
             use run_id or session_id to list resolved state",
        )
        .into_response();
    }

    // ── Gather plan approvals (unless narrowed out or session_id set) ──
    let mut merged: Vec<UnifiedApproval> = Vec::new();
    let want_plan = matches!(kind_filter, None | Some(ApprovalKind::Plan));
    // Plan approvals have no session_id dimension; a session_id filter
    // excludes them outright rather than returning a confusing empty
    // set after a wasted fetch.
    let want_plan = want_plan && query.session_id.is_none();
    if want_plan {
        // Ask for a generous slab so post-filter pagination isn't
        // silently truncated by state filtering.
        let plan_fetch_limit = offset
            .saturating_add(limit)
            .saturating_add(1)
            .min(MAX_LIST_FETCH);
        match state
            .runtime
            .approvals
            .list_all(&project, plan_fetch_limit, 0)
            .await
        {
            Ok(items) => {
                for record in items {
                    if let Some(run_filter) = query.run_id.as_deref() {
                        let matches_run = record
                            .run_id
                            .as_ref()
                            .map(|r| r.as_str() == run_filter)
                            .unwrap_or(false);
                        if !matches_run {
                            continue;
                        }
                    }
                    if let Some(state_want) = state_filter {
                        let matches_state = match state_want {
                            ToolCallApprovalState::Pending => record.decision.is_none(),
                            ToolCallApprovalState::Approved => {
                                matches!(record.decision, Some(ApprovalDecision::Approved))
                            }
                            ToolCallApprovalState::Rejected => {
                                matches!(record.decision, Some(ApprovalDecision::Rejected))
                            }
                            // Plan approvals don't time out on their own;
                            // a `timeout` filter excludes them entirely.
                            ToolCallApprovalState::Timeout => false,
                        };
                        if !matches_state {
                            continue;
                        }
                    }
                    merged.push(UnifiedApproval::Plan(Box::new(record)));
                }
            }
            Err(err) => return runtime_error_response(err),
        }
    }

    // ── Gather tool-call approvals ────────────────────────────────────
    if want_tool {
        let reader: &dyn ToolCallApprovalReadModel = state.runtime.store.as_ref();
        let fetch_limit = offset
            .saturating_add(limit)
            .saturating_add(1)
            .min(MAX_LIST_FETCH);
        let records: Result<Vec<ToolCallApprovalRecord>, _> =
            if let Some(rid) = query.run_id.as_deref() {
                reader.list_for_run(&RunId::new(rid)).await
            } else if let Some(sid) = query.session_id.as_deref() {
                reader.list_for_session(&SessionId::new(sid)).await
            } else if admin_cross_tenant_inbox {
                // F44: admin super-inbox spans every tenant. Pagination
                // is pushed down to the projection.
                reader.list_all_pending(fetch_limit, 0).await
            } else {
                reader
                    .list_pending_for_project(&project, fetch_limit, 0)
                    .await
            };
        match records {
            Ok(items) => {
                for record in items {
                    if !tenant_scope.is_admin
                        && record.project.tenant_id != *tenant_scope.tenant_id()
                    {
                        continue;
                    }
                    if let Some(state_want) = state_filter {
                        if record.state != state_want {
                            continue;
                        }
                    }
                    merged.push(UnifiedApproval::ToolCall(Box::new(record)));
                }
            }
            Err(err) => return store_error_response(err),
        }
    }

    // Newest first by created/proposed timestamp.
    merged.sort_by_key(|r| std::cmp::Reverse(r.created_at_ms()));

    let total = merged.len();
    let end = offset.saturating_add(limit).min(total);
    let start = offset.min(total);
    let page: Vec<_> = merged.drain(start..end).collect();
    let has_more = end < total;

    // Borrow only needs `items` here, but `state` discriminator cannot
    // be mixed with `flatten` in serde so we use a dedicated response
    // envelope that re-uses `ListResponse`.
    (
        StatusCode::OK,
        Json(ListResponse {
            items: page,
            has_more,
        }),
    )
        .into_response()
}

/// `GET /v1/approvals/:id` — fetch by id across both stores.
pub(crate) async fn get_approval_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match resolve_approval_by_id(state.as_ref(), &tenant_scope, &id).await {
        Ok(record) => (StatusCode::OK, Json(record)).into_response(),
        Err(resp) => resp,
    }
}

/// `POST /v1/approvals/:id/approve` — kind-aware approve.
pub(crate) async fn approve_approval_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<String>,
    body: Option<Json<UnifiedApproveBody>>,
) -> impl IntoResponse {
    let body = body.map(|Json(b)| b).unwrap_or_default();

    let record = match resolve_approval_by_id(state.as_ref(), &tenant_scope, &id).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match record {
        UnifiedApproval::Plan(record) => {
            // Plan approval: ignore `scope` / `approved_tool_args`; they
            // have no meaning on this kind. `operator_id` is still
            // validated.
            if let Err(err) = derive_operator_id(&principal, body.operator_id.as_deref()) {
                return err.into_response();
            }
            resolve_plan_approval(
                &state,
                &principal,
                &record,
                ApprovalDecision::Approved,
                "resolve_approval",
                serde_json::json!({ "decision": "approved" }),
            )
            .await
        }
        UnifiedApproval::ToolCall(record) => {
            let operator_id = match derive_operator_id(&principal, body.operator_id.as_deref()) {
                Ok(o) => o,
                Err(err) => return err.into_response(),
            };
            let scope_body = match body.scope {
                Some(s) => s,
                None => {
                    return AppApiError::new(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "validation_error",
                        "tool-call approval requires `scope` (once | session)",
                    )
                    .into_response()
                }
            };
            approve_tool_call(
                &state,
                &principal,
                *record,
                operator_id,
                scope_body,
                body.approved_tool_args,
            )
            .await
        }
    }
}

/// `POST /v1/approvals/:id/reject` (and `/deny` alias).
pub(crate) async fn reject_approval_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<String>,
    body: Option<Json<UnifiedRejectBody>>,
) -> impl IntoResponse {
    let body = body.map(|Json(b)| b).unwrap_or_default();

    let record = match resolve_approval_by_id(state.as_ref(), &tenant_scope, &id).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match record {
        UnifiedApproval::Plan(record) => {
            if let Err(err) = derive_operator_id(&principal, body.operator_id.as_deref()) {
                return err.into_response();
            }
            let audit_payload = serde_json::json!({
                "decision": "rejected",
                "reason":   body.reason,
            });
            resolve_plan_approval(
                &state,
                &principal,
                &record,
                ApprovalDecision::Rejected,
                "resolve_approval",
                audit_payload,
            )
            .await
        }
        UnifiedApproval::ToolCall(record) => {
            let operator_id = match derive_operator_id(&principal, body.operator_id.as_deref()) {
                Ok(o) => o,
                Err(err) => return err.into_response(),
            };
            reject_tool_call(&state, &principal, *record, operator_id, body.reason).await
        }
    }
}

/// `POST /v1/approvals/:id/deny` — legacy alias of reject. Thin wrapper
/// so the `/deny` audit action string stays distinct where the caller
/// used it historically.
pub(crate) async fn deny_approval_handler(
    state: State<Arc<AppState>>,
    tenant_scope: TenantScope,
    principal: Extension<AuthPrincipal>,
    id: Path<String>,
    body: Option<Json<UnifiedRejectBody>>,
) -> impl IntoResponse {
    // Identical semantics to reject — plan + tool-call resolve to the
    // same `Rejected` state. Kept as an alias so existing smoke tests
    // and scripts keep working.
    reject_approval_handler(state, tenant_scope, principal, id, body).await
}

/// `PATCH /v1/approvals/:id/amend` — tool-call only. 404 for plan.
pub(crate) async fn amend_approval_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<String>,
    Json(body): Json<AmendBody>,
) -> impl IntoResponse {
    let call_id = ToolCallId::new(id.clone());
    let record = match load_tool_call_visible_to_tenant(&state, &tenant_scope, &call_id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            // Give a clearer error when the id *does* exist as a plan
            // approval — amend doesn't apply there.
            let approval_id = ApprovalId::new(id);
            match load_plan_approval_visible_to_tenant(&state, &tenant_scope, &approval_id).await {
                Ok(Some(_)) => {
                    return AppApiError::new(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "unsupported_on_plan_approval",
                        "amend is only supported on tool-call approvals",
                    )
                    .into_response();
                }
                Ok(None) => {
                    return AppApiError::new(
                        StatusCode::NOT_FOUND,
                        "not_found",
                        "approval not found",
                    )
                    .into_response();
                }
                Err(resp) => return resp,
            }
        }
        Err(resp) => return resp,
    };

    // Self-amend guard: preserve the pre-F45 confused-deputy block.
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
                Ok(Some(updated)) => (
                    StatusCode::OK,
                    Json(UnifiedApproval::ToolCall(Box::new(updated))),
                )
                    .into_response(),
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

/// `POST /v1/approvals/:id/delegate` — stub retained from pre-F45.
pub(crate) async fn delegate_approval_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<DelegateApprovalRequest>,
) -> impl IntoResponse {
    let _ = (state, id, body);
    AppApiError::new(
        StatusCode::NOT_IMPLEMENTED,
        "not_implemented",
        "approval delegation is not yet implemented",
    )
    .into_response()
}

// ── Internal: plan-approval resolution + audit ──────────────────────────────

async fn resolve_plan_approval(
    state: &Arc<AppState>,
    principal: &AuthPrincipal,
    record: &ApprovalRecord,
    decision: ApprovalDecision,
    action: &'static str,
    audit_payload: Value,
) -> axum::response::Response {
    let before = crate::handlers::sse::current_event_head(state).await;
    match state
        .runtime
        .approvals
        .resolve(&record.approval_id, decision)
        .await
    {
        Ok(updated) => match state
            .runtime
            .audits
            .record(
                updated.project.tenant_id.clone(),
                audit_actor_id(principal),
                action.to_owned(),
                "approval".to_owned(),
                updated.approval_id.to_string(),
                AuditOutcome::Success,
                audit_payload,
            )
            .await
        {
            Ok(_) => {
                crate::handlers::sse::publish_runtime_frames_since(state, before).await;
                (
                    StatusCode::OK,
                    Json(UnifiedApproval::Plan(Box::new(updated))),
                )
                    .into_response()
            }
            Err(err) => runtime_error_response(err),
        },
        Err(err) => runtime_error_response(err),
    }
}

// ── Internal: tool-call resolution + audit ─────────────────────────────────

async fn approve_tool_call(
    state: &Arc<AppState>,
    principal: &AuthPrincipal,
    record: ToolCallApprovalRecord,
    operator_id: OperatorId,
    scope_body: ApproveScope,
    approved_tool_args: Option<Value>,
) -> axum::response::Response {
    let scope = match scope_body {
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

    let call_id = record.call_id.clone();
    let before = crate::handlers::sse::current_event_head(state).await;
    match state
        .runtime
        .tool_call_approvals
        .approve(
            call_id.clone(),
            operator_id,
            scope.clone(),
            approved_tool_args.clone(),
        )
        .await
    {
        Ok(()) => {
            let audit = state
                .runtime
                .audits
                .record(
                    record.project.tenant_id.clone(),
                    audit_actor_id(principal),
                    "approve_tool_call".to_owned(),
                    "tool_call_approval".to_owned(),
                    call_id.to_string(),
                    AuditOutcome::Success,
                    serde_json::json!({
                        "call_id": call_id.to_string(),
                        "scope":   scope,
                        "had_args_override": approved_tool_args.is_some(),
                    }),
                )
                .await;
            if let Err(err) = audit {
                return runtime_error_response(err);
            }
            crate::handlers::sse::publish_runtime_frames_since(state, before).await;
            let reader: &dyn ToolCallApprovalReadModel = state.runtime.store.as_ref();
            match reader.get(&call_id).await {
                Ok(Some(updated)) => (
                    StatusCode::OK,
                    Json(UnifiedApproval::ToolCall(Box::new(updated))),
                )
                    .into_response(),
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

async fn reject_tool_call(
    state: &Arc<AppState>,
    principal: &AuthPrincipal,
    record: ToolCallApprovalRecord,
    operator_id: OperatorId,
    reason: Option<String>,
) -> axum::response::Response {
    let call_id = record.call_id.clone();
    let before = crate::handlers::sse::current_event_head(state).await;
    match state
        .runtime
        .tool_call_approvals
        .reject(call_id.clone(), operator_id, reason.clone())
        .await
    {
        Ok(()) => {
            let audit = state
                .runtime
                .audits
                .record(
                    record.project.tenant_id.clone(),
                    audit_actor_id(principal),
                    "reject_tool_call".to_owned(),
                    "tool_call_approval".to_owned(),
                    call_id.to_string(),
                    AuditOutcome::Success,
                    serde_json::json!({
                        "call_id": call_id.to_string(),
                        "reason":  reason,
                    }),
                )
                .await;
            if let Err(err) = audit {
                return runtime_error_response(err);
            }
            crate::handlers::sse::publish_runtime_frames_since(state, before).await;
            let reader: &dyn ToolCallApprovalReadModel = state.runtime.store.as_ref();
            match reader.get(&call_id).await {
                Ok(Some(updated)) => (
                    StatusCode::OK,
                    Json(UnifiedApproval::ToolCall(Box::new(updated))),
                )
                    .into_response(),
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

// ── Request-approval (unchanged — distinct semantics from resolve) ─────────

pub(crate) async fn request_approval_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Json(body): Json<RequestApprovalRequest>,
) -> impl IntoResponse {
    let project = body.project();
    if !tenant_scope.is_admin && project.tenant_id != *tenant_scope.tenant_id() {
        return crate::errors::tenant_scope_mismatch_error().into_response();
    }
    let before = crate::handlers::sse::current_event_head(&state).await;
    match state
        .runtime
        .approvals
        .request(
            &project,
            ApprovalId::new(body.approval_id),
            body.run_id.map(RunId::new),
            body.task_id.map(TaskId::new),
            body.requirement.unwrap_or(ApprovalRequirement::Required),
        )
        .await
    {
        Ok(record) => {
            crate::handlers::sse::publish_runtime_frames_since(&state, before).await;
            (StatusCode::CREATED, Json(record)).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

// ── Approval policies (unchanged) ──────────────────────────────────────────

pub(crate) async fn create_approval_policy_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateApprovalPolicyRequest>,
) -> impl IntoResponse {
    let tenant_id = TenantId::new(body.tenant_id.as_deref().unwrap_or(DEFAULT_TENANT_ID));
    match state
        .runtime
        .approval_policies
        .create(
            tenant_id,
            body.name,
            body.required_approvers,
            body.allowed_approver_roles,
            body.auto_approve_after_ms,
            body.auto_reject_after_ms,
        )
        .await
    {
        Ok(record) => (StatusCode::CREATED, Json(record)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn list_approval_policies_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ApprovalPolicyListQuery>,
) -> impl IntoResponse {
    match state
        .runtime
        .approval_policies
        .list(&query.tenant_id(), query.limit(), query.offset())
        .await
    {
        Ok(items) => (
            StatusCode::OK,
            Json(ListResponse {
                has_more: items.len() == query.limit(),
                items,
            }),
        )
            .into_response(),
        Err(err) => runtime_error_response(err),
    }
}
