//! Prompt registry, release controls, evaluations, and scorecard boundaries.

/// Prompt asset, version, and release boundaries.
pub mod prompts {}

/// Selector and rollout boundaries.
pub mod selectors {}

/// Eval matrix boundaries.
pub mod matrices {}

/// Scorecard boundaries.
pub mod scorecards {}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        assert!(true);
    }
}
