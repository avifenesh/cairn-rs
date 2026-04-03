//! Product-owned ingest, retrieval, and memory service boundaries.

/// Document ingest boundaries.
pub mod ingest {}

/// Retrieval query boundaries.
pub mod retrieval {}

/// Ranking and diagnostics boundaries.
pub mod diagnostics {}

/// Deep-search boundaries.
pub mod deep_search {}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        assert!(true);
    }
}
