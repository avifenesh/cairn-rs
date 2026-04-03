//! Language-neutral plugin protocol boundaries and shared types.

/// Protocol manifest boundaries.
pub mod manifest {}

/// Request/response envelope boundaries.
pub mod wire {}

/// Plugin capability declarations.
pub mod capabilities {}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        assert!(true);
    }
}
