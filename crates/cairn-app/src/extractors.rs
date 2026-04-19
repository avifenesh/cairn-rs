//! HTTP request extractors and scope guards.

use axum::extract::{FromRequest, FromRequestParts, Query, Request};
use axum::http::request::Parts;
use axum::Json;
use serde::de::DeserializeOwned;

use cairn_api::auth::AuthPrincipal;
use cairn_api::endpoints::ListQuery;
use cairn_api::memory_api::MemorySearchQuery;
use cairn_domain::{ProjectKey, TenantId, WorkspaceRole};

use crate::errors::{
    forbidden_api_error, query_rejection_error, tenant_scope_mismatch_error,
    unauthorized_api_error, AppApiError,
};
use crate::{DEFAULT_PROJECT_ID, DEFAULT_TENANT_ID, DEFAULT_WORKSPACE_ID};

// ── Query structs ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct ProjectScopedQuery {
    pub(crate) tenant_id: String,
    pub(crate) workspace_id: String,
    pub(crate) project_id: String,
    pub(crate) limit: Option<usize>,
    pub(crate) offset: Option<usize>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct OptionalProjectScopedQuery {
    pub(crate) tenant_id: Option<String>,
    pub(crate) workspace_id: Option<String>,
    pub(crate) project_id: Option<String>,
    pub(crate) limit: Option<usize>,
    pub(crate) offset: Option<usize>,
}

impl OptionalProjectScopedQuery {
    pub(crate) fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_deref().unwrap_or(DEFAULT_TENANT_ID),
            self.workspace_id.as_deref().unwrap_or(DEFAULT_WORKSPACE_ID),
            self.project_id.as_deref().unwrap_or(DEFAULT_PROJECT_ID),
        )
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit.unwrap_or(100)
    }

    pub(crate) fn offset(&self) -> usize {
        self.offset.unwrap_or(0)
    }
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct PreservedMemoryListQuery {
    pub(crate) tenant_id: Option<String>,
    pub(crate) workspace_id: Option<String>,
    pub(crate) project_id: Option<String>,
    pub(crate) limit: Option<usize>,
    pub(crate) offset: Option<usize>,
    pub(crate) status: Option<String>,
    pub(crate) category: Option<String>,
}

impl PreservedMemoryListQuery {
    pub(crate) fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_deref().unwrap_or(DEFAULT_TENANT_ID),
            self.workspace_id.as_deref().unwrap_or(DEFAULT_WORKSPACE_ID),
            self.project_id.as_deref().unwrap_or(DEFAULT_PROJECT_ID),
        )
    }

    pub(crate) fn list_query(&self) -> ListQuery {
        ListQuery {
            limit: self.limit,
            offset: self.offset,
            status: self.status.clone(),
            category: self.category.clone(),
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct PreservedMemorySearchParams {
    pub(crate) q: String,
    pub(crate) limit: Option<usize>,
    pub(crate) tenant_id: Option<String>,
    pub(crate) workspace_id: Option<String>,
    pub(crate) project_id: Option<String>,
}

impl PreservedMemorySearchParams {
    pub(crate) fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_deref().unwrap_or(DEFAULT_TENANT_ID),
            self.workspace_id.as_deref().unwrap_or(DEFAULT_WORKSPACE_ID),
            self.project_id.as_deref().unwrap_or(DEFAULT_PROJECT_ID),
        )
    }

    pub(crate) fn search_query(&self) -> MemorySearchQuery {
        MemorySearchQuery {
            q: self.q.clone(),
            limit: self.limit,
        }
    }
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct TenantCostQuery {
    pub(crate) since_ms: Option<u64>,
}

impl ProjectScopedQuery {
    pub(crate) fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit.unwrap_or(100)
    }

    pub(crate) fn offset(&self) -> usize {
        self.offset.unwrap_or(0)
    }
}

// ── HasProjectScope trait ──────────────────────────────────────────────────

pub(crate) trait HasProjectScope {
    fn project(&self) -> ProjectKey;
}

impl HasProjectScope for OptionalProjectScopedQuery {
    fn project(&self) -> ProjectKey {
        Self::project(self)
    }
}

impl HasProjectScope for PreservedMemoryListQuery {
    fn project(&self) -> ProjectKey {
        Self::project(self)
    }
}

impl HasProjectScope for PreservedMemorySearchParams {
    fn project(&self) -> ProjectKey {
        Self::project(self)
    }
}

// ── Scope types ────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub(crate) struct TenantScope {
    pub(crate) tenant_id: TenantId,
    /// `true` when the request was authenticated with the admin service account.
    /// Admin tokens bypass per-tenant scope checks so they can access any tenant.
    pub(crate) is_admin: bool,
}

impl TenantScope {
    pub(crate) fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }
}

pub struct WorkspaceRoleGuard<const MIN_ROLE: u8>;
#[allow(dead_code)]
pub(crate) type MemberRoleGuard = WorkspaceRoleGuard<1>;
pub(crate) type ReviewerRoleGuard = WorkspaceRoleGuard<2>;
pub type AdminRoleGuard = WorkspaceRoleGuard<3>;

#[derive(Clone, Debug)]
pub(crate) struct ProjectScope<T> {
    pub(crate) tenant: TenantScope,
    #[allow(dead_code)]
    pub(crate) project: ProjectKey,
    pub(crate) value: T,
}

impl<T> ProjectScope<T> {
    #[allow(dead_code)]
    pub(crate) fn project(&self) -> &ProjectKey {
        &self.project
    }

    pub(crate) fn into_inner(self) -> T {
        self.value
    }

    #[allow(dead_code)]
    pub(crate) fn tenant_scope(&self) -> &TenantScope {
        &self.tenant
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ProjectJson<T> {
    pub(crate) tenant: TenantScope,
    #[allow(dead_code)]
    pub(crate) project: ProjectKey,
    pub(crate) value: T,
}

impl<T> ProjectJson<T> {
    #[allow(dead_code)]
    pub(crate) fn project(&self) -> &ProjectKey {
        &self.project
    }

    pub(crate) fn into_inner(self) -> T {
        self.value
    }

    #[allow(dead_code)]
    pub(crate) fn tenant_scope(&self) -> &TenantScope {
        &self.tenant
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

pub(crate) fn validate_project_scope<T: HasProjectScope>(
    tenant: TenantScope,
    value: T,
) -> Result<(TenantScope, ProjectKey, T), AppApiError> {
    let project = value.project();
    // Admin tokens have cross-tenant access — skip the scope check.
    if !tenant.is_admin && project.tenant_id != *tenant.tenant_id() {
        return Err(tenant_scope_mismatch_error());
    }

    Ok((tenant, project, value))
}

/// `true` for the bootstrap admin service account or the System principal.
/// T6b-C5 (shared helper): returns true when the path's project belongs
/// to the caller's tenant OR the caller is an admin. Used by
/// `marketplace_routes` and `repo_routes` to refuse cross-tenant
/// mutations; hoisted here to avoid divergent copies.
pub(crate) fn enforce_project_tenant(
    principal: &AuthPrincipal,
    project: &cairn_domain::tenancy::ProjectKey,
) -> bool {
    if is_admin_principal(principal) {
        return true;
    }
    principal
        .tenant()
        .map(|t| t.tenant_id == project.tenant_id)
        .unwrap_or(false)
}

pub fn is_admin_principal(principal: &AuthPrincipal) -> bool {
    match principal {
        AuthPrincipal::System => true,
        AuthPrincipal::ServiceAccount { name, .. } => name == "admin",
        AuthPrincipal::Operator { .. } => false,
    }
}

// ── FromRequestParts / FromRequest impls ───────────────────────────────────

#[axum::async_trait]
impl<S> FromRequestParts<S> for TenantScope
where
    S: Send + Sync,
{
    type Rejection = AppApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let tenant_id = parts
            .extensions
            .get::<TenantId>()
            .cloned()
            .ok_or_else(unauthorized_api_error)?;
        // Admin service account bypasses per-tenant scope checks.
        let is_admin = parts
            .extensions
            .get::<AuthPrincipal>()
            .map(is_admin_principal)
            .unwrap_or(false);
        Ok(Self {
            tenant_id,
            is_admin,
        })
    }
}

#[axum::async_trait]
impl<S, T> FromRequestParts<S> for ProjectScope<T>
where
    S: Send + Sync,
    T: HasProjectScope + DeserializeOwned + Send,
{
    type Rejection = AppApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let tenant = TenantScope::from_request_parts(parts, state).await?;
        let Query(value) = Query::<T>::from_request_parts(parts, state)
            .await
            .map_err(|err| query_rejection_error(err.to_string()))?;
        let (tenant, project, value) = validate_project_scope(tenant, value)?;
        Ok(Self {
            tenant,
            project,
            value,
        })
    }
}

#[axum::async_trait]
impl<S, T> FromRequest<S> for ProjectJson<T>
where
    S: Send + Sync,
    T: HasProjectScope + DeserializeOwned + Send,
{
    type Rejection = AppApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let is_admin = request
            .extensions()
            .get::<AuthPrincipal>()
            .map(is_admin_principal)
            .unwrap_or(false);
        let tenant = request
            .extensions()
            .get::<TenantId>()
            .cloned()
            .map(|tenant_id| TenantScope {
                tenant_id,
                is_admin,
            })
            .ok_or_else(unauthorized_api_error)?;
        let Json(value) = Json::<T>::from_request(request, state)
            .await
            .map_err(|err| query_rejection_error(err.body_text()))?;
        let (tenant, project, value) = validate_project_scope(tenant, value)?;
        Ok(Self {
            tenant,
            project,
            value,
        })
    }
}

#[axum::async_trait]
impl<S, const MIN_ROLE: u8> FromRequestParts<S> for WorkspaceRoleGuard<MIN_ROLE>
where
    S: Send + Sync,
{
    type Rejection = AppApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // T6b-C4 fail-closed: the prior "no workspace role attached →
        // treat as unrestricted" branch was a blanket bypass. Admin
        // endpoints must refuse requests from principals that haven't
        // had a role attached, UNLESS the principal is System / the
        // admin service account (these carry `is_admin_principal ==
        // true` and don't need a workspace-role binding to pass).
        if let Some(principal) = parts.extensions.get::<AuthPrincipal>() {
            if is_admin_principal(principal) {
                return Ok(Self);
            }
        }
        let Some(role) = parts.extensions.get::<WorkspaceRole>().copied() else {
            return Err(forbidden_api_error(
                "workspace role not attached; refusing privileged request",
            ));
        };
        if (role as u8) < MIN_ROLE {
            return Err(forbidden_api_error("insufficient workspace role"));
        }
        Ok(Self)
    }
}
