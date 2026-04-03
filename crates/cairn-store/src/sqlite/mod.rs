//! SQLite-backed implementations of store traits for local-mode use.
//!
//! Gated behind the `sqlite` feature flag. Provides the same trait
//! implementations as the Postgres backend but targeting SQLite for
//! development, personal use, and small-scale evaluation (RFC 003).

mod adapter;
mod event_log;
mod projections;
mod schema;

pub use adapter::SqliteAdapter;
pub use event_log::SqliteEventLog;
pub use projections::SqliteSyncProjection;
