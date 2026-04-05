//! Postgres-backed implementations of store traits.
//!
//! Gated behind the `postgres` feature flag.

mod adapter;
mod event_log;
mod migration_runner;
mod projections;
mod rebuild;

pub use adapter::PgAdapter;
pub use event_log::PgEventLog;
pub use migration_runner::{PgMigrationRunner, registered_migrations};
pub use projections::PgSyncProjection;
pub use rebuild::{ProjectionRebuilder, RebuildReport};
