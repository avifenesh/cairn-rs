use ff_script::ScriptError;

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
