//! Recovery, sandbox durability, and retry-safety domain types for Phase 0.

use crate::RunId;
use serde::{Deserialize, Serialize};

/// Dimensions on which sandbox policy can enforce a hard cap in v1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceDimension {
    DiskBytes,
    MemoryBytes,
    WallClockMs,
}

/// Policy action to take once a sandbox hits a configured resource limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnExhaustion {
    Destroy,
    PauseAwaitOperator,
    ReportOnly,
}

/// Durable checkpoint boundary captured around each orchestrator iteration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointKind {
    Intent,
    Result,
}

/// Recovery behavior for tool calls whose completion is ambiguous after a crash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrySafety {
    IdempotentSafe,
    DangerousPause,
    AuthorResponsible,
}

/// Why a sandbox was destroyed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestroyReason {
    Completed,
    Failed,
    Abandoned,
    Stale,
    ResourceLimitExceeded {
        dimension: ResourceDimension,
        limit: u64,
        observed: u64,
    },
}

/// Why a sandbox was preserved for later inspection or operator action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreservationReason {
    AgentCrashed,
    AgentPreempted,
    ControlPlaneRestart,
    AwaitingResourceRaise {
        dimension: ResourceDimension,
        limit: u64,
        observed: u64,
    },
    BaseRevisionDrift {
        expected: String,
        actual: String,
    },
    AllowlistRevoked {
        /// Canonical repo identifier, typically `owner/repo`.
        repo_id: String,
    },
}

/// Persistent record of a run that exceeded the recovery attempt threshold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryEscalation {
    pub run_id: RunId,
    pub attempt_count: u32,
    pub last_error: String,
    pub escalated_at_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::{
        CheckpointKind, DestroyReason, OnExhaustion, PreservationReason, ResourceDimension,
        RetrySafety,
    };

    #[test]
    fn phase_zero_enums_serialize_in_snake_case() {
        assert_eq!(
            serde_json::to_string(&ResourceDimension::WallClockMs).unwrap(),
            r#""wall_clock_ms""#
        );
        assert_eq!(
            serde_json::to_string(&OnExhaustion::PauseAwaitOperator).unwrap(),
            r#""pause_await_operator""#
        );
        assert_eq!(
            serde_json::to_string(&CheckpointKind::Intent).unwrap(),
            r#""intent""#
        );
        assert_eq!(
            serde_json::to_string(&RetrySafety::AuthorResponsible).unwrap(),
            r#""author_responsible""#
        );
    }

    #[test]
    fn destroy_reason_round_trips_resource_limit_fields() {
        let reason = DestroyReason::ResourceLimitExceeded {
            dimension: ResourceDimension::DiskBytes,
            limit: 1024,
            observed: 2048,
        };

        let json = serde_json::to_string(&reason).unwrap();
        let restored: DestroyReason = serde_json::from_str(&json).unwrap();

        assert_eq!(restored, reason);
    }

    #[test]
    fn preservation_reason_round_trips_all_phase_zero_variants() {
        let cases = [
            PreservationReason::AwaitingResourceRaise {
                dimension: ResourceDimension::MemoryBytes,
                limit: 256,
                observed: 300,
            },
            PreservationReason::BaseRevisionDrift {
                expected: "abc123".to_owned(),
                actual: "def456".to_owned(),
            },
            PreservationReason::AllowlistRevoked {
                repo_id: "org/repo".to_owned(),
            },
        ];

        for reason in cases {
            let json = serde_json::to_string(&reason).unwrap();
            let restored: PreservationReason = serde_json::from_str(&json).unwrap();
            assert_eq!(restored, reason);
        }
    }
}
