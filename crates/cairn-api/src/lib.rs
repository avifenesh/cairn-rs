//! HTTP, SSE, auth, bootstrap, and operator-facing API boundaries.

/// HTTP route boundaries.
pub mod http {}

/// SSE boundaries.
pub mod sse {}

/// Authentication boundaries.
pub mod auth {}

/// Operator-facing read model boundaries.
pub mod read_models {}

/// Bootstrap and onboarding boundaries.
pub mod bootstrap {}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        assert!(true);
    }
}
