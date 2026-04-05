use std::fmt;

/// Runtime service errors.
#[derive(Debug)]
pub enum RuntimeError {
    /// Entity not found.
    NotFound { entity: &'static str, id: String },
    /// Invalid state transition.
    InvalidTransition {
        entity: &'static str,
        from: String,
        to: String,
    },
    /// Command rejected by policy.
    PolicyDenied { reason: String },
    /// Optimistic concurrency conflict.
    Conflict { entity: &'static str, id: String },
    /// Lease has expired.
    LeaseExpired { task_id: String },
    /// Store error.
    Store(cairn_store::StoreError),
    /// Internal error.
    Internal(String),
    /// Tenant quota exceeded.
    QuotaExceeded { tenant_id: String, quota_type: String, current: u32, limit: u32 },
    /// Validation failure.
    Validation { reason: String },
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeError::NotFound { entity, id } => write!(f, "{entity} not found: {id}"),
            RuntimeError::InvalidTransition { entity, from, to } => {
                write!(f, "invalid {entity} transition: {from} -> {to}")
            }
            RuntimeError::PolicyDenied { reason } => write!(f, "policy denied: {reason}"),
            RuntimeError::Conflict { entity, id } => {
                write!(f, "{entity} conflict: {id}")
            }
            RuntimeError::LeaseExpired { task_id } => write!(f, "lease expired: {task_id}"),
            RuntimeError::Store(e) => write!(f, "store error: {e}"),
            RuntimeError::Internal(msg) => write!(f, "internal runtime error: {msg}"),
            RuntimeError::QuotaExceeded { tenant_id, quota_type, current, limit } => {
                write!(f, "quota exceeded for tenant {tenant_id}: {quota_type} ({current}/{limit})")
            }
            RuntimeError::Validation { reason } => write!(f, "validation error: {reason}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<cairn_store::StoreError> for RuntimeError {
    fn from(e: cairn_store::StoreError) -> Self {
        RuntimeError::Store(e)
    }
}
