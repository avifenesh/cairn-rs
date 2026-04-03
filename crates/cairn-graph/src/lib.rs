//! Graph projections and graph-backed product query boundaries.

/// Graph projections built from runtime and provenance facts.
pub mod projections {}

/// Product-shaped graph queries.
pub mod queries {}

/// Provenance and execution graph boundaries.
pub mod provenance {}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        assert!(true);
    }
}
