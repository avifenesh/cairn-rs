//! Fleet view — active agent sessions across the deployment.
//!
//! Mirrors `cairn/internal/server/routes_fleet.go`:
//! - `GET /v1/fleet` returns at most 200 active sessions with status, summary, and truncation flag.
//! - Status is derived from the session's latest run state.

use cairn_domain::lifecycle::{AgentStatus, RunState};
use cairn_store::projections::{RunRecord, SessionRecord};
use serde::{Deserialize, Serialize};

/// Maximum sessions returned by the fleet endpoint (mirrors `fleetSessionLimit` in Go).
pub const FLEET_SESSION_LIMIT: usize = 200;

/// One entry in the fleet view.
///
/// Corresponds to a single active agent session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentFleetEntry {
    /// Session ID.
    pub id: String,
    /// Derived operational status.
    pub status: AgentStatus,
    /// Unix ms timestamp of last activity.
    pub last_active_ms: u64,
    /// Title of the currently executing task, if any.
    #[serde(rename = "currentTask", skip_serializing_if = "Option::is_none")]
    pub current_task: Option<String>,
}

/// Response payload for `GET /v1/fleet`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentFleetView {
    pub agents: Vec<AgentFleetEntry>,
    /// Count of agents by status: `{ "busy": N, "idle": N, "offline": N }`.
    pub summary: FleetSummary,
    /// True when the session list was capped at `FLEET_SESSION_LIMIT`.
    pub truncated: bool,
}

/// Aggregated counts of agent statuses in the fleet.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetSummary {
    pub busy: u32,
    pub idle: u32,
    pub offline: u32,
}

impl FleetSummary {
    pub fn increment(&mut self, status: AgentStatus) {
        match status {
            AgentStatus::Busy => self.busy += 1,
            AgentStatus::Idle => self.idle += 1,
            AgentStatus::Offline => self.offline += 1,
        }
    }

    pub fn total(&self) -> u32 {
        self.busy + self.idle + self.offline
    }
}

/// Build a fleet view from session records and their latest run records.
///
/// `sessions` — all non-terminal sessions retrieved from the store (≤ FLEET_SESSION_LIMIT + 1)
/// `runs`     — the latest run for each session, keyed by session_id
///
/// This is a pure function to keep the fleet logic testable without a store.
pub fn build_fleet_view(
    sessions: Vec<SessionRecord>,
    runs: &std::collections::HashMap<String, RunRecord>,
    limit: usize,
) -> AgentFleetView {
    let truncated = sessions.len() > limit;
    let sessions = &sessions[..sessions.len().min(limit)];

    let mut summary = FleetSummary::default();
    let mut agents = Vec::with_capacity(sessions.len());

    for session in sessions {
        let latest_run = runs.get(session.session_id.as_str());
        let run_state = latest_run.map(|r| r.state);
        let status = AgentStatus::from_run_state(run_state);
        let current_task = latest_run
            .filter(|r| !r.state.is_terminal())
            .map(|_| "running".to_owned()); // placeholder; real title comes from TaskRecord

        summary.increment(status);
        agents.push(AgentFleetEntry {
            id: session.session_id.as_str().to_owned(),
            status,
            last_active_ms: session.updated_at,
            current_task,
        });
    }

    AgentFleetView {
        agents,
        summary,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::{
        lifecycle::{RunState, SessionState},
        ProjectKey, RunId, SessionId,
    };
    use cairn_store::projections::{RunRecord, SessionRecord};
    use std::collections::HashMap;

    fn session(id: &str, updated_at: u64) -> SessionRecord {
        SessionRecord {
            session_id: SessionId::new(id),
            project: ProjectKey::new("t", "w", "p"),
            state: SessionState::Open,
            version: 1,
            created_at: 1000,
            updated_at,
        }
    }

    fn run(session_id: &str, state: RunState) -> RunRecord {
        RunRecord {
            run_id: RunId::new(format!("run_{session_id}")),
            session_id: SessionId::new(session_id),
            parent_run_id: None,
            project: ProjectKey::new("t", "w", "p"),
            state,
            prompt_release_id: None,
            agent_role_id: None,
            failure_class: None,
            pause_reason: None,
            resume_trigger: None,
            version: 1,
            created_at: 1000,
            updated_at: 2000,
        }
    }

    #[test]
    fn fleet_view_busy_idle_offline_counts() {
        let sessions = vec![
            session("s1", 3000),
            session("s2", 2000),
            session("s3", 1000),
        ];
        let mut runs = HashMap::new();
        runs.insert("s1".to_owned(), run("s1", RunState::Running));       // busy
        runs.insert("s2".to_owned(), run("s2", RunState::WaitingApproval)); // idle
        // s3: no run → offline

        let view = build_fleet_view(sessions, &runs, FLEET_SESSION_LIMIT);

        assert_eq!(view.summary.busy, 1);
        assert_eq!(view.summary.idle, 1);
        assert_eq!(view.summary.offline, 1);
        assert!(!view.truncated);
    }

    #[test]
    fn fleet_view_truncates_at_limit() {
        // Create limit+1 sessions.
        let sessions: Vec<SessionRecord> = (0..=5).map(|i| session(&format!("s{i}"), i as u64)).collect();
        let runs = HashMap::new();

        let view = build_fleet_view(sessions, &runs, 5);
        assert_eq!(view.agents.len(), 5);
        assert!(view.truncated);
    }

    #[test]
    fn fleet_view_no_truncation_within_limit() {
        let sessions: Vec<SessionRecord> = (0..3).map(|i| session(&format!("s{i}"), i as u64)).collect();
        let view = build_fleet_view(sessions, &HashMap::new(), FLEET_SESSION_LIMIT);
        assert_eq!(view.agents.len(), 3);
        assert!(!view.truncated);
    }

    #[test]
    fn agent_status_from_run_states() {
        assert_eq!(AgentStatus::from_run_state(Some(RunState::Running)), AgentStatus::Busy);
        assert_eq!(AgentStatus::from_run_state(Some(RunState::WaitingApproval)), AgentStatus::Idle);
        assert_eq!(AgentStatus::from_run_state(Some(RunState::Paused)), AgentStatus::Idle);
        assert_eq!(AgentStatus::from_run_state(Some(RunState::Completed)), AgentStatus::Offline);
        assert_eq!(AgentStatus::from_run_state(Some(RunState::Failed)), AgentStatus::Offline);
        assert_eq!(AgentStatus::from_run_state(None), AgentStatus::Offline);
    }

    #[test]
    fn fleet_summary_counts_correctly() {
        let mut summary = FleetSummary::default();
        summary.increment(AgentStatus::Busy);
        summary.increment(AgentStatus::Busy);
        summary.increment(AgentStatus::Idle);
        summary.increment(AgentStatus::Offline);
        assert_eq!(summary.busy, 2);
        assert_eq!(summary.idle, 1);
        assert_eq!(summary.offline, 1);
        assert_eq!(summary.total(), 4);
    }

    #[test]
    fn fleet_summary_serializes_to_json() {
        let view = AgentFleetView {
            agents: vec![AgentFleetEntry {
                id: "sess_1".to_owned(),
                status: AgentStatus::Busy,
                last_active_ms: 9000,
                current_task: Some("Draft PR".to_owned()),
            }],
            summary: FleetSummary { busy: 1, idle: 0, offline: 0 },
            truncated: false,
        };
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["summary"]["busy"], 1);
        assert_eq!(json["agents"][0]["status"], "busy");
        assert_eq!(json["agents"][0]["currentTask"], "Draft PR");
        assert_eq!(json["truncated"], false);
    }
}
