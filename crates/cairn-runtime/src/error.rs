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
    /// Re-declaring a task dependency with a different edge kind or
    /// `data_passing_ref` than the already-staged edge. Cairn surfaces
    /// this as HTTP 409 with both existing and requested values so
    /// operators can see the divergence.
    ///
    /// Boxed to keep `RuntimeError` at a small `size_of` (clippy
    /// `result_large_err`): this variant is rare compared to the
    /// lifecycle variants, so paying one allocation on the error
    /// path beats inflating every `Result<_, RuntimeError>` return.
    DependencyConflict(Box<DependencyConflictDetail>),
    /// Lease has expired.
    LeaseExpired { task_id: String },
    /// Store error.
    Store(cairn_store::StoreError),
    /// Internal error.
    Internal(String),
    /// Tenant quota exceeded.
    QuotaExceeded {
        tenant_id: String,
        quota_type: String,
        current: u32,
        limit: u32,
    },
    /// Validation failure.
    Validation { reason: String },
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeError::NotFound { entity, id } => write!(f, "{entity} not found: {id}"),
            RuntimeError::InvalidTransition { entity, from, to } => {
                // F41: when the `from` field carries a known FF state
                // code (injected by `fabric_err_to_runtime`'s terminal /
                // suspend classifiers), surface an operator-actionable
                // summary instead of the raw `execution_not_active ->
                // completed` jargon that the dogfood v6 runs hit.
                //
                // The original code is preserved verbatim at the end
                // (`code={from}`) so the operator can still grep logs
                // and correlate with FF's ScriptError taxonomy, but the
                // leading prose tells them what to do right now.
                if let Some(hint) = invalid_transition_hint(from.as_str(), to.as_str(), entity) {
                    write!(
                        f,
                        "invalid {entity} transition to {to}: {hint} (code={from})"
                    )
                } else {
                    write!(f, "invalid {entity} transition: {from} -> {to}")
                }
            }
            RuntimeError::PolicyDenied { reason } => write!(f, "policy denied: {reason}"),
            RuntimeError::Conflict { entity, id } => {
                write!(f, "{entity} conflict: {id}")
            }
            RuntimeError::LeaseExpired { task_id } => write!(f, "lease expired: {task_id}"),
            RuntimeError::Store(e) => write!(f, "store error: {e}"),
            RuntimeError::Internal(msg) => write!(f, "internal runtime error: {msg}"),
            RuntimeError::QuotaExceeded {
                tenant_id,
                quota_type,
                current,
                limit,
            } => {
                write!(
                    f,
                    "quota exceeded for tenant {tenant_id}: {quota_type} ({current}/{limit})"
                )
            }
            RuntimeError::Validation { reason } => write!(f, "validation error: {reason}"),
            RuntimeError::DependencyConflict(d) => write!(
                f,
                "dependency edge {} <- {} already exists with \
                 kind={} ref={:?}; re-declare requested kind={} ref={:?}",
                d.dependent_task_id,
                d.prerequisite_task_id,
                d.existing_kind,
                d.existing_data_passing_ref,
                d.requested_kind,
                d.requested_data_passing_ref,
            ),
        }
    }
}

impl std::error::Error for RuntimeError {}

/// Map a FF / cairn state-transition rejection code to an operator-
/// actionable prose hint.
///
/// Returns `None` for codes we don't recognise — callers fall back to
/// the raw `{from} -> {to}` format so unknown codes aren't silently
/// swallowed behind a generic message.
///
/// Scope per F41: the terminal and suspend classifiers in
/// `cairn_app::fabric_adapter` are the two known call sites that
/// inject FF codes into this field. Keep the table in sync with
/// `is_terminal_state_conflict` and `is_suspend_state_conflict`.
fn invalid_transition_hint(from: &str, to: &str, entity: &str) -> Option<&'static str> {
    match from {
        "execution_not_active" => Some(match to {
            // Terminal target — run / task never reached `active`
            // lifecycle, or it already transitioned to a terminal
            // state before the caller's request landed.
            "completed" | "failed" | "cancelled" => {
                "the run's execution is no longer in an active lease. \
                 The lease may have expired mid-loop, the run may \
                 already be terminal (check GET /v1/runs/:id), or the \
                 run was never claimed. Re-activate via POST \
                 /v1/runs/:id/claim and retry"
            }
            // Suspend / resume target — run is in a terminal phase
            // so pause / resume cannot apply.
            "suspended" | "active" => {
                "cannot pause or resume: the run is already terminal \
                 or its lease has expired. Check the run's current \
                 state via GET /v1/runs/:id"
            }
            _ => {
                "the execution is not in an active lease (check GET \
                 /v1/runs/:id for current state)"
            }
        }),
        "lease_expired" => Some(
            "the execution's lease expired before cairn could write \
             the terminal outcome. Extend the lease TTL for long-\
             running runs, or retry after re-claiming via POST \
             /v1/runs/:id/claim",
        ),
        "lease_revoked" => Some(
            "the execution's lease was revoked by an operator or \
             scanner before this request completed. Check the run's \
             current state via GET /v1/runs/:id",
        ),
        "stale_lease" | "invalid_lease_for_suspend" => Some(
            "the lease token supplied does not match the execution's \
             current lease. Re-read the run (GET /v1/runs/:id) and \
             retry with the current lease fence",
        ),
        "fence_required" => Some(
            "this operation requires either an active lease fence or \
             an explicit operator override. Claim the run via POST \
             /v1/runs/:id/claim first",
        ),
        "partial_fence_triple" => Some(
            "internal error: cairn sent an inconsistent lease fence to \
             the fabric. This is a cairn bug (F37) — file an issue \
             with the run id",
        ),
        "already_suspended" => Some(
            "a suspension is already open for this run — resume or \
             cancel the existing suspension before starting a new one",
        ),
        _ => {
            // Avoid unused-warning on entity for callers that don't
            // need the generic hint; keep the parameter for future
            // entity-specific messages.
            let _ = entity;
            None
        }
    }
}

/// Payload for [`RuntimeError::DependencyConflict`]. Boxed in the
/// enum so the overall `RuntimeError` size stays small.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyConflictDetail {
    pub dependent_task_id: String,
    pub prerequisite_task_id: String,
    pub existing_kind: String,
    pub existing_data_passing_ref: Option<String>,
    pub requested_kind: String,
    pub requested_data_passing_ref: Option<String>,
}

impl From<cairn_store::StoreError> for RuntimeError {
    fn from(e: cairn_store::StoreError) -> Self {
        RuntimeError::Store(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F41: the operator-facing message for an `execution_not_active`
    /// rejection on a terminal FCALL must NOT be raw FF jargon. It
    /// must name the failure in plain prose and tell the operator how
    /// to recover (re-activate via `/claim`, or check current state).
    #[test]
    fn invalid_transition_execution_not_active_completed_is_operator_actionable() {
        let err = RuntimeError::InvalidTransition {
            entity: "run",
            from: "execution_not_active".to_owned(),
            to: "completed".to_owned(),
        };
        let msg = err.to_string();
        // Must still mention the entity and target state for
        // dashboards / log greps.
        assert!(msg.contains("run"), "missing entity: {msg}");
        assert!(msg.contains("completed"), "missing target: {msg}");
        // Must carry an actionable verb.
        assert!(
            msg.contains("POST /v1/runs/:id/claim") || msg.contains("GET /v1/runs/:id"),
            "missing operator action hint: {msg}"
        );
        // Raw code preserved for correlation.
        assert!(
            msg.contains("execution_not_active"),
            "missing FF code for correlation: {msg}"
        );
        // Must NOT be the raw pre-F41 format.
        assert_ne!(
            msg, "invalid run transition: execution_not_active -> completed",
            "F41 regression: error reverted to raw FF jargon",
        );
    }

    #[test]
    fn invalid_transition_lease_expired_gets_hint() {
        let err = RuntimeError::InvalidTransition {
            entity: "run",
            from: "lease_expired".to_owned(),
            to: "completed".to_owned(),
        };
        let msg = err.to_string();
        assert!(msg.contains("lease"), "missing lease keyword: {msg}");
        assert!(msg.contains("lease_expired"), "missing code: {msg}");
    }

    #[test]
    fn invalid_transition_unknown_code_falls_back_to_raw_format() {
        // Unknown code → preserve the raw `{from} -> {to}` format so
        // operators can still see what happened. The hint table is
        // intentionally closed-world.
        let err = RuntimeError::InvalidTransition {
            entity: "task",
            from: "totally_novel_code".to_owned(),
            to: "completed".to_owned(),
        };
        let msg = err.to_string();
        assert_eq!(
            msg,
            "invalid task transition: totally_novel_code -> completed"
        );
    }

    #[test]
    fn invalid_transition_execution_not_active_suspend_distinct_hint() {
        // Suspend target emits a different hint than terminal target
        // (different recovery path).
        let err = RuntimeError::InvalidTransition {
            entity: "run",
            from: "execution_not_active".to_owned(),
            to: "suspended".to_owned(),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("pause or resume") || msg.contains("already terminal"),
            "suspend hint should differ from terminal: {msg}"
        );
    }
}
