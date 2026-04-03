//! SQLite-backed persistence and retrieval for local-mode.

mod documents;
mod retrieval;

pub use documents::SqliteDocumentStore;
pub use retrieval::SqliteRetrievalService;
