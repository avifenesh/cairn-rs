//! Tool invocation, permissions, plugin host integration, and execution isolation.

/// Builtin tool host boundaries.
pub mod builtin {}

/// Tool permission model boundaries.
pub mod permissions {}

/// Plugin host boundaries.
pub mod plugins {}

/// Supervised-process execution class boundaries.
pub mod supervised_process {}

/// Sandboxed-process execution class boundaries.
pub mod sandboxed_process {}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        assert!(true);
    }
}
