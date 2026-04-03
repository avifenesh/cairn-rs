//! Storage, migrations, event log, and synchronous projection boundaries.

/// Migration orchestration.
pub mod migrations {}

/// Durable event-log storage.
pub mod event_log {}

/// Synchronous projections for correctness-critical state.
pub mod projections {}

/// Repository boundaries for persisted entities.
pub mod repos {}

/// Database adapter boundaries.
pub mod db {}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        assert!(true);
    }
}
