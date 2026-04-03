use crate::error::StoreError;

/// A single schema migration.
///
/// Migrations follow the naming convention `V{NNN}__{description}.sql`
/// and live in `crates/cairn-store/migrations/`. The version number is
/// zero-padded to three digits.
///
/// Examples:
/// - `V001__create_event_log.sql`
/// - `V002__create_session_table.sql`
/// - `V003__create_run_table.sql`
#[derive(Clone, Debug)]
pub struct Migration {
    /// Monotonic version number (e.g., 1, 2, 3).
    pub version: u32,
    /// Human-readable name derived from the filename.
    pub name: String,
    /// SQL content of the migration.
    pub sql: String,
}

/// Tracks which migrations have been applied.
#[derive(Clone, Debug)]
pub struct AppliedMigration {
    pub version: u32,
    pub name: String,
    /// Unix milliseconds when the migration was applied.
    pub applied_at: u64,
}

/// Migration runner interface.
///
/// Concrete implementations read SQL files from the migrations directory
/// and apply them in version order within a transaction.
pub trait MigrationRunner: Send + Sync {
    /// List all available migrations from the migrations directory.
    fn available(&self) -> Result<Vec<Migration>, StoreError>;

    /// List migrations that have already been applied.
    fn applied(&self) -> Result<Vec<AppliedMigration>, StoreError>;

    /// Return migrations that are available but not yet applied.
    fn pending(&self) -> Result<Vec<Migration>, StoreError> {
        let available = self.available()?;
        let applied = self.applied()?;
        let max_applied = applied.iter().map(|m| m.version).max().unwrap_or(0);
        Ok(available
            .into_iter()
            .filter(|m| m.version > max_applied)
            .collect())
    }

    /// Apply all pending migrations in version order.
    fn run_pending(&self) -> Result<Vec<AppliedMigration>, StoreError>;
}

#[cfg(test)]
mod tests {
    use super::Migration;

    #[test]
    fn migration_carries_version_and_sql() {
        let m = Migration {
            version: 1,
            name: "create_event_log".to_owned(),
            sql: "CREATE TABLE event_log (...);".to_owned(),
        };
        assert_eq!(m.version, 1);
        assert!(m.sql.contains("CREATE TABLE"));
    }
}
