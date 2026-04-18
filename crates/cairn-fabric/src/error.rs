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
