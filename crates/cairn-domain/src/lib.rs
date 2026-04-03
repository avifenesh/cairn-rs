//! Shared domain contracts for Cairn runtime, storage, and product services.

/// Stable identifier types.
pub mod ids {}

/// Canonical command shapes.
pub mod commands {}

/// Canonical event shapes.
pub mod events {}

/// Tenancy and ownership primitives.
pub mod tenancy {}

/// Lifecycle enums and state machine helpers.
pub mod lifecycle {}

/// Policy and permission-related shared types.
pub mod policy {}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        assert!(true);
    }
}
