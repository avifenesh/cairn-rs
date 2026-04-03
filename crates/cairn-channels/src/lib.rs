//! Channel delivery and notification routing boundaries.

/// Notification routing boundaries.
pub mod router {}

/// Outbound channel adapter boundaries.
pub mod adapters {}

/// Delivery policy boundaries.
pub mod policies {}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        assert!(true);
    }
}
