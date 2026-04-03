//! Durable runtime services for sessions, runs, tasks, approvals, and recovery.

/// Session orchestration boundaries.
pub mod sessions {}

/// Run orchestration boundaries.
pub mod runs {}

/// Task orchestration boundaries.
pub mod tasks {}

/// Approval lifecycle boundaries.
pub mod approvals {}

/// Checkpoint persistence and recovery boundaries.
pub mod checkpoints {}

/// Durable mailbox boundaries.
pub mod mailbox {}

/// Replay, recovery, and pause/resume boundaries.
pub mod recovery {}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        assert!(true);
    }
}
