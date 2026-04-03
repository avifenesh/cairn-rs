use async_trait::async_trait;

use crate::error::StoreError;

/// Marker for supported database backends.
///
/// V1 targets Postgres for production and SQLite for testing/embedded use.
/// Concrete adapters implement `DbAdapter` for their backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Postgres,
    Sqlite,
}

/// Database adapter boundary for pluggable storage backends.
///
/// Concrete implementations wrap a connection pool and provide the
/// transactional boundary that ties event-log appends to synchronous
/// projection updates.
///
/// The transaction lifecycle is intentionally not exposed as a trait
/// method in Week 1. Instead, implementations compose event-log and
/// projection writes internally within a single database transaction.
/// This avoids locking the trait surface to a specific transaction
/// handle type before we choose a connection library.
#[async_trait]
pub trait DbAdapter: Send + Sync {
    /// Which backend this adapter targets.
    fn backend(&self) -> Backend;

    /// Verify the database connection is alive.
    async fn health_check(&self) -> Result<(), StoreError>;

    /// Run pending migrations.
    async fn migrate(&self) -> Result<(), StoreError>;
}

#[cfg(test)]
mod tests {
    use super::Backend;

    #[test]
    fn backends_are_distinct() {
        assert_ne!(Backend::Postgres, Backend::Sqlite);
    }
}
