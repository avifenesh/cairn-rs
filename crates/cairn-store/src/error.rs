use std::fmt;

/// Store-level errors for persistence, event-log, and projection operations.
#[derive(Debug)]
pub enum StoreError {
    /// Entity not found by ID.
    NotFound { entity: &'static str, id: String },
    /// Optimistic concurrency conflict.
    Conflict {
        entity: &'static str,
        expected_version: u64,
        actual_version: u64,
    },
    /// Database connection or pool error.
    Connection(String),
    /// Migration execution error.
    Migration(String),
    /// Serialization or deserialization error.
    Serialization(String),
    /// Unclassified internal error.
    Internal(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::NotFound { entity, id } => write!(f, "{entity} not found: {id}"),
            StoreError::Conflict {
                entity,
                expected_version,
                actual_version,
            } => write!(
                f,
                "{entity} version conflict: expected {expected_version}, got {actual_version}"
            ),
            StoreError::Connection(msg) => write!(f, "connection error: {msg}"),
            StoreError::Migration(msg) => write!(f, "migration error: {msg}"),
            StoreError::Serialization(msg) => write!(f, "serialization error: {msg}"),
            StoreError::Internal(msg) => write!(f, "internal store error: {msg}"),
        }
    }
}

impl std::error::Error for StoreError {}
