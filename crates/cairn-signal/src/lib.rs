//! Signal ingestion, scheduling, and digest generation boundaries.

/// Scheduler boundaries.
pub mod scheduler {}

/// Source polling boundaries.
pub mod pollers {}

/// Webhook ingestion boundaries.
pub mod webhooks {}

/// Digest generation boundaries.
pub mod digests {}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        assert!(true);
    }
}
