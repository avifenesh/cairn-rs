//! RFC 008: cross-workspace resource sharing domain types.

use crate::{TenantId, WorkspaceId};
use serde::{Deserialize, Serialize};

/// A shared resource grant linking a source workspace to a target workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedResource {
    pub share_id: String,
    pub tenant_id: TenantId,
    pub source_workspace_id: WorkspaceId,
    pub target_workspace_id: WorkspaceId,
    /// One of "prompt_asset", "corpus", or "source".
    pub resource_type: String,
    pub resource_id: String,
    pub permissions: Vec<String>,
    pub shared_at_ms: u64,
}
