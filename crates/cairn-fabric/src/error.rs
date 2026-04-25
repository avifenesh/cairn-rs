use flowfabric::script::ScriptError;

#[derive(Debug, thiserror::Error)]
pub enum FabricError {
    #[error("valkey: {0}")]
    Valkey(String),
    #[error("script: {0}")]
    Script(#[from] ScriptError),
    #[error("config: {0}")]
    Config(String),
    #[error("bridge: {0}")]
    Bridge(String),
    #[error("{entity} not found: {id}")]
    NotFound { entity: &'static str, id: String },
    #[error("validation: {reason}")]
    Validation { reason: String },
    /// Re-declaring a dependency edge with a different `dependency_kind`
    /// or `data_passing_ref` than the staged edge. Carries both the
    /// existing and requested values for diagnostics; the adapter maps
    /// this to `RuntimeError::DependencyConflict` and the HTTP layer
    /// responds 409. Boxed to keep `FabricError` small (clippy
    /// `result_large_err`): every conflict is rare compared to the
    /// hot-path error variants, so one allocation on the error path
    /// is cheaper than inflating `size_of::<FabricError>()`.
    #[error(
        "dependency edge {} <- {} already staged with kind={} ref={:?}; re-declare specified \
         kind={} ref={:?}",
        .0.dependent_task_id,
        .0.prerequisite_task_id,
        .0.existing_kind,
        .0.existing_data_passing_ref,
        .0.requested_kind,
        .0.requested_data_passing_ref
    )]
    DependencyConflict(Box<DependencyConflictDetail>),
    #[error("internal: {0}")]
    Internal(String),
    #[error(
        "valkey version too low: detected {detected}, required >= {required}. \
         FlowFabric uses the Valkey Functions API (FCALL / FUNCTION LOAD), \
         which landed in Valkey 7.0 — older versions lack the API entirely. \
         During a rolling upgrade this check retries for 60s; if it still \
         fails, the node we reconnect to is still pre-7.0. \
         Note: operators are strongly encouraged to run 8.0.x+ for the Lua \
         sandbox CVE patches (CVE-2024-46981, -31449, CVE-2025-49844, \
         -46817/18/19) and because FlowFabric's CI only validates against \
         8.0 — a boot-time WARN is emitted on 7.x."
    )]
    ValkeyVersionTooLow { detected: String, required: String },
}

/// Payload for [`FabricError::DependencyConflict`]. Boxed in the
/// enum so overall `size_of::<FabricError>()` stays small.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependencyConflictDetail {
    pub dependent_task_id: String,
    pub prerequisite_task_id: String,
    pub existing_kind: String,
    pub existing_data_passing_ref: Option<String>,
    pub requested_kind: String,
    pub requested_data_passing_ref: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fabric_error_display() {
        let err = FabricError::Valkey("connection refused".into());
        assert!(err.to_string().contains("connection refused"));

        let err = FabricError::Config("missing host".into());
        assert!(err.to_string().contains("missing host"));

        let err = FabricError::Bridge("channel closed".into());
        assert!(err.to_string().contains("channel closed"));
    }

    #[test]
    fn script_error_converts() {
        let script_err = ScriptError::ExecutionNotFound;
        let fabric_err = FabricError::from(script_err);
        assert!(matches!(fabric_err, FabricError::Script(_)));
        assert!(fabric_err.to_string().contains("execution"));
    }
}
