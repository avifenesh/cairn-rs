//! Storage, migrations, event log, and synchronous projection boundaries.
//!
//! `cairn-store` owns durable persistence for all runtime entities:
//!
//! - **Event log**: append-only log for `full_history` entities (RFC 002)
//! - **Synchronous projections**: current-state tables updated transactionally
//!   with event persistence (session, run, task, approval, checkpoint, mailbox,
//!   tool invocation)
//! - **Migrations**: schema migration runner and naming conventions
//! - **DB adapter**: pluggable backend boundary for Postgres and SQLite

pub mod db;
pub mod error;
pub mod event_log;
pub mod in_memory;
pub mod migration_check;
pub mod migrations;
#[cfg(feature = "postgres")]
pub mod pg;
pub mod projections;
pub mod snapshot;
#[cfg(feature = "sqlite")]
pub mod sqlite;

pub use db::{Backend, DbAdapter};
pub use error::StoreError;
pub use event_log::{DurabilityClass, EntityRef, EventLog, EventPosition, StoredEvent};
pub use in_memory::InMemoryStore;
pub use migrations::{AppliedMigration, Migration, MigrationRunner};
pub use projections::SyncProjection;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_compiles_with_domain_dependency() {
        let id = cairn_domain::SessionId::new("test");
        assert_eq!(id.as_str(), "test");
    }
}
