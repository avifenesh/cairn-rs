//! RFC 006 prompt release pipeline — version diffing, gradual rollout,
//! A/B routing, approval gating, and rollback.
//!
//! This module sits above the existing [`PromptReleaseService`] and
//! [`PromptVersionService`] traits, adding pipeline-level orchestration
//! that those CRUD boundaries don't own.
//!
//! ## Architecture
//!
//! ```text
//! PromptReleasePipeline
//!  ├─ diff_versions()       — line-by-line diff of two version templates
//!  ├─ start_rollout()       — set rollout % and persist state
//!  ├─ resolve_with_rollout()— A/B route: rand < percent → new, else stable
//!  ├─ request_full_release()— approval gate before going to 100%
//!  ├─ rollback()            — set rollout to 0%, mark rolled_back
//!  └─ get_rollout_state()   — inspect current rollout metadata
//! ```

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

// ── Gap 1: Version Diffing ───────────────────────────────────────────────────

/// A single line-level change in a version diff.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    /// 1-based line number in the old text (None for additions).
    pub old_line: Option<usize>,
    /// 1-based line number in the new text (None for deletions).
    pub new_line: Option<usize>,
    pub kind: DiffKind,
    pub content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffKind {
    Equal,
    Added,
    Removed,
}

/// Result of diffing two prompt version templates.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VersionDiff {
    pub old_version_id: String,
    pub new_version_id: String,
    pub lines: Vec<DiffLine>,
    pub additions: usize,
    pub deletions: usize,
}

/// Compute a line-by-line diff between two text strings.
///
/// Uses a simple LCS-based algorithm (O(n*m) worst case, fine for
/// prompt templates which are typically < 500 lines).
pub fn diff_texts(old: &str, new: &str) -> Vec<DiffLine> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let n = old_lines.len();
    let m = new_lines.len();

    // Build LCS table.
    let mut table = vec![vec![0u32; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            if old_lines[i - 1] == new_lines[j - 1] {
                table[i][j] = table[i - 1][j - 1] + 1;
            } else {
                table[i][j] = table[i - 1][j].max(table[i][j - 1]);
            }
        }
    }

    // Backtrack to produce diff.
    let mut result = Vec::new();
    let mut i = n;
    let mut j = m;

    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old_lines[i - 1] == new_lines[j - 1] {
            result.push(DiffLine {
                old_line: Some(i),
                new_line: Some(j),
                kind: DiffKind::Equal,
                content: old_lines[i - 1].to_owned(),
            });
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || table[i][j - 1] >= table[i - 1][j]) {
            result.push(DiffLine {
                old_line: None,
                new_line: Some(j),
                kind: DiffKind::Added,
                content: new_lines[j - 1].to_owned(),
            });
            j -= 1;
        } else {
            result.push(DiffLine {
                old_line: Some(i),
                new_line: None,
                kind: DiffKind::Removed,
                content: old_lines[i - 1].to_owned(),
            });
            i -= 1;
        }
    }

    result.reverse();
    result
}

/// Build a `VersionDiff` from two version IDs and their template content.
pub fn diff_versions(
    old_version_id: &str,
    old_content: &str,
    new_version_id: &str,
    new_content: &str,
) -> VersionDiff {
    let lines = diff_texts(old_content, new_content);
    let additions = lines.iter().filter(|l| l.kind == DiffKind::Added).count();
    let deletions = lines.iter().filter(|l| l.kind == DiffKind::Removed).count();
    VersionDiff {
        old_version_id: old_version_id.to_owned(),
        new_version_id: new_version_id.to_owned(),
        lines,
        additions,
        deletions,
    }
}

// ── Gap 2+3+5: Rollout State + A/B Routing + Rollback ───────────────────────

/// Rollout lifecycle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RolloutStatus {
    /// Gradual rollout in progress at some percent.
    Active,
    /// Rolled out to 100% (fully live).
    Complete,
    /// Rolled back to 0%.
    RolledBack,
    /// Awaiting approval before going to 100%.
    PendingApproval,
}

/// Per-release rollout metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RolloutState {
    pub release_id: String,
    pub percent: u8,
    pub status: RolloutStatus,
    /// The "stable" release that serves the remaining (100 - percent)% traffic.
    pub stable_release_id: Option<String>,
    /// Approval ID if approval was requested.
    pub approval_id: Option<String>,
    pub updated_at_ms: u64,
}

/// Outcome of an A/B routing decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoutingDecision {
    /// Serve the new (candidate) release.
    Candidate { release_id: String },
    /// Serve the stable (current) release.
    Stable { release_id: String },
    /// No rollout active for this release; use default resolution.
    NoRollout,
}

/// In-memory rollout state manager.
///
/// Manages rollout percentages, A/B routing decisions, approval gates,
/// and rollback. Designed to be shared via `Arc<PromptReleasePipeline>`.
pub struct PromptReleasePipeline {
    /// Rollout state keyed by release_id.
    rollouts: Mutex<HashMap<String, RolloutState>>,
    /// Version content cache keyed by version_id (for diffing).
    version_content: Mutex<HashMap<String, String>>,
}

impl PromptReleasePipeline {
    pub fn new() -> Self {
        Self {
            rollouts: Mutex::new(HashMap::new()),
            version_content: Mutex::new(HashMap::new()),
        }
    }

    // ── Version content management ───────────────────────────────────────

    /// Store version template content (needed for diffing).
    pub fn store_version_content(&self, version_id: &str, content: String) {
        self.version_content
            .lock()
            .unwrap()
            .insert(version_id.to_owned(), content);
    }

    /// Retrieve stored version content.
    pub fn get_version_content(&self, version_id: &str) -> Option<String> {
        self.version_content
            .lock()
            .unwrap()
            .get(version_id)
            .cloned()
    }

    /// Gap 1: Diff two prompt versions by their IDs.
    ///
    /// Returns `None` if either version's content hasn't been stored.
    pub fn diff_versions(
        &self,
        old_version_id: &str,
        new_version_id: &str,
    ) -> Option<VersionDiff> {
        let content = self.version_content.lock().unwrap();
        let old = content.get(old_version_id)?;
        let new = content.get(new_version_id)?;
        Some(diff_versions(old_version_id, old, new_version_id, new))
    }

    // ── Rollout management ───────────────────────────────────────────────

    /// Gap 2: Start a gradual rollout at the given percentage.
    ///
    /// `stable_release_id` is the currently active release that serves
    /// the remaining traffic.
    pub fn start_rollout(
        &self,
        release_id: &str,
        percent: u8,
        stable_release_id: Option<String>,
    ) -> RolloutState {
        let percent = percent.min(100);
        let state = RolloutState {
            release_id: release_id.to_owned(),
            percent,
            status: if percent >= 100 {
                RolloutStatus::Complete
            } else {
                RolloutStatus::Active
            },
            stable_release_id,
            approval_id: None,
            updated_at_ms: now_ms(),
        };
        self.rollouts
            .lock()
            .unwrap()
            .insert(release_id.to_owned(), state.clone());
        state
    }

    /// Update rollout percentage for an active rollout.
    pub fn update_rollout_percent(&self, release_id: &str, percent: u8) -> Option<RolloutState> {
        let mut map = self.rollouts.lock().unwrap();
        let state = map.get_mut(release_id)?;
        if state.status == RolloutStatus::RolledBack {
            return None; // can't update a rolled-back release
        }
        let percent = percent.min(100);
        state.percent = percent;
        state.status = if percent >= 100 {
            RolloutStatus::Complete
        } else {
            RolloutStatus::Active
        };
        state.updated_at_ms = now_ms();
        Some(state.clone())
    }

    /// Gap 3: A/B routing decision based on rollout percentage.
    ///
    /// `rand_value` should be a uniform random in [0.0, 1.0).
    /// Returns `Candidate` if rand < percent/100, `Stable` otherwise.
    pub fn resolve_with_rollout(
        &self,
        release_id: &str,
        rand_value: f64,
    ) -> RoutingDecision {
        let map = self.rollouts.lock().unwrap();
        let state = match map.get(release_id) {
            Some(s) => s,
            None => return RoutingDecision::NoRollout,
        };

        match state.status {
            RolloutStatus::RolledBack => {
                // All traffic goes to stable.
                match &state.stable_release_id {
                    Some(id) => RoutingDecision::Stable {
                        release_id: id.clone(),
                    },
                    None => RoutingDecision::NoRollout,
                }
            }
            RolloutStatus::Complete => RoutingDecision::Candidate {
                release_id: release_id.to_owned(),
            },
            RolloutStatus::Active | RolloutStatus::PendingApproval => {
                let threshold = state.percent as f64 / 100.0;
                if rand_value < threshold {
                    RoutingDecision::Candidate {
                        release_id: release_id.to_owned(),
                    }
                } else {
                    match &state.stable_release_id {
                        Some(id) => RoutingDecision::Stable {
                            release_id: id.clone(),
                        },
                        None => RoutingDecision::NoRollout,
                    }
                }
            }
        }
    }

    /// Gap 4: Request approval before promoting to 100%.
    ///
    /// Sets the rollout status to `PendingApproval` and stores the
    /// approval ID for later checking. The caller is responsible for
    /// actually creating the approval record via [`ApprovalService`].
    pub fn request_full_release(
        &self,
        release_id: &str,
        approval_id: String,
    ) -> Option<RolloutState> {
        let mut map = self.rollouts.lock().unwrap();
        let state = map.get_mut(release_id)?;
        state.status = RolloutStatus::PendingApproval;
        state.approval_id = Some(approval_id);
        state.updated_at_ms = now_ms();
        Some(state.clone())
    }

    /// Gap 4: Complete the approval gate — promote to 100%.
    ///
    /// Only succeeds if the rollout is in `PendingApproval` status.
    pub fn approve_full_release(&self, release_id: &str) -> Option<RolloutState> {
        let mut map = self.rollouts.lock().unwrap();
        let state = map.get_mut(release_id)?;
        if state.status != RolloutStatus::PendingApproval {
            return None;
        }
        state.percent = 100;
        state.status = RolloutStatus::Complete;
        state.updated_at_ms = now_ms();
        Some(state.clone())
    }

    /// Gap 5: Rollback — set rollout to 0% and mark as rolled_back.
    pub fn rollback(&self, release_id: &str) -> Option<RolloutState> {
        let mut map = self.rollouts.lock().unwrap();
        let state = map.get_mut(release_id)?;
        state.percent = 0;
        state.status = RolloutStatus::RolledBack;
        state.updated_at_ms = now_ms();
        Some(state.clone())
    }

    /// Get current rollout state for a release.
    pub fn get_rollout_state(&self, release_id: &str) -> Option<RolloutState> {
        self.rollouts.lock().unwrap().get(release_id).cloned()
    }

    /// List all active rollouts.
    pub fn active_rollouts(&self) -> Vec<RolloutState> {
        self.rollouts
            .lock()
            .unwrap()
            .values()
            .filter(|s| s.status == RolloutStatus::Active || s.status == RolloutStatus::PendingApproval)
            .cloned()
            .collect()
    }
}

impl Default for PromptReleasePipeline {
    fn default() -> Self {
        Self::new()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Gap 1: Version diffing tests ─────────────────────────────────────

    #[test]
    fn diff_identical_texts() {
        let lines = diff_texts("hello\nworld", "hello\nworld");
        assert!(lines.iter().all(|l| l.kind == DiffKind::Equal));
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn diff_added_line() {
        let lines = diff_texts("line1\nline2", "line1\ninserted\nline2");
        let added: Vec<_> = lines.iter().filter(|l| l.kind == DiffKind::Added).collect();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].content, "inserted");
    }

    #[test]
    fn diff_removed_line() {
        let lines = diff_texts("line1\nremoved\nline2", "line1\nline2");
        let removed: Vec<_> = lines.iter().filter(|l| l.kind == DiffKind::Removed).collect();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].content, "removed");
    }

    #[test]
    fn diff_changed_line() {
        let lines = diff_texts("hello world", "hello rust");
        let removed: Vec<_> = lines.iter().filter(|l| l.kind == DiffKind::Removed).collect();
        let added: Vec<_> = lines.iter().filter(|l| l.kind == DiffKind::Added).collect();
        assert_eq!(removed.len(), 1);
        assert_eq!(added.len(), 1);
        assert_eq!(removed[0].content, "hello world");
        assert_eq!(added[0].content, "hello rust");
    }

    #[test]
    fn diff_empty_to_content() {
        let lines = diff_texts("", "new line");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].kind, DiffKind::Added);
    }

    #[test]
    fn diff_content_to_empty() {
        let lines = diff_texts("old line", "");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].kind, DiffKind::Removed);
    }

    #[test]
    fn diff_versions_computes_stats() {
        let diff = diff_versions(
            "v1",
            "You are a helpful assistant.\nBe concise.",
            "v2",
            "You are a helpful coding assistant.\nBe concise.\nUse examples.",
        );
        assert_eq!(diff.old_version_id, "v1");
        assert_eq!(diff.new_version_id, "v2");
        assert_eq!(diff.additions, 2); // changed line + new line
        assert_eq!(diff.deletions, 1); // old changed line removed
    }

    #[test]
    fn pipeline_diff_versions_by_id() {
        let pipeline = PromptReleasePipeline::new();
        pipeline.store_version_content("v1", "line one\nline two".into());
        pipeline.store_version_content("v2", "line one\nline three".into());

        let diff = pipeline.diff_versions("v1", "v2").unwrap();
        assert_eq!(diff.deletions, 1);
        assert_eq!(diff.additions, 1);
    }

    #[test]
    fn pipeline_diff_missing_version_returns_none() {
        let pipeline = PromptReleasePipeline::new();
        pipeline.store_version_content("v1", "content".into());
        assert!(pipeline.diff_versions("v1", "v_missing").is_none());
    }

    // ── Gap 2: Rollout state tests ───────────────────────────────────────

    #[test]
    fn start_rollout_stores_state() {
        let pipeline = PromptReleasePipeline::new();
        let state = pipeline.start_rollout("rel_1", 25, Some("rel_stable".into()));
        assert_eq!(state.percent, 25);
        assert_eq!(state.status, RolloutStatus::Active);
        assert_eq!(state.stable_release_id.as_deref(), Some("rel_stable"));
    }

    #[test]
    fn start_rollout_at_100_is_complete() {
        let pipeline = PromptReleasePipeline::new();
        let state = pipeline.start_rollout("rel_1", 100, None);
        assert_eq!(state.status, RolloutStatus::Complete);
        assert_eq!(state.percent, 100);
    }

    #[test]
    fn update_rollout_percent() {
        let pipeline = PromptReleasePipeline::new();
        pipeline.start_rollout("rel_1", 10, None);
        let state = pipeline.update_rollout_percent("rel_1", 50).unwrap();
        assert_eq!(state.percent, 50);
        assert_eq!(state.status, RolloutStatus::Active);
    }

    #[test]
    fn update_rollout_percent_clamps_to_100() {
        let pipeline = PromptReleasePipeline::new();
        pipeline.start_rollout("rel_1", 10, None);
        let state = pipeline.update_rollout_percent("rel_1", 200).unwrap();
        assert_eq!(state.percent, 100);
        assert_eq!(state.status, RolloutStatus::Complete);
    }

    // ── Gap 3: A/B routing tests ─────────────────────────────────────────

    #[test]
    fn routing_no_rollout_returns_no_rollout() {
        let pipeline = PromptReleasePipeline::new();
        let decision = pipeline.resolve_with_rollout("nonexistent", 0.5);
        assert_eq!(decision, RoutingDecision::NoRollout);
    }

    #[test]
    fn routing_50_percent_candidate() {
        let pipeline = PromptReleasePipeline::new();
        pipeline.start_rollout("rel_new", 50, Some("rel_stable".into()));

        // rand = 0.3 < 0.5 → candidate
        let decision = pipeline.resolve_with_rollout("rel_new", 0.3);
        assert_eq!(
            decision,
            RoutingDecision::Candidate {
                release_id: "rel_new".into()
            }
        );
    }

    #[test]
    fn routing_50_percent_stable() {
        let pipeline = PromptReleasePipeline::new();
        pipeline.start_rollout("rel_new", 50, Some("rel_stable".into()));

        // rand = 0.7 >= 0.5 → stable
        let decision = pipeline.resolve_with_rollout("rel_new", 0.7);
        assert_eq!(
            decision,
            RoutingDecision::Stable {
                release_id: "rel_stable".into()
            }
        );
    }

    #[test]
    fn routing_100_percent_always_candidate() {
        let pipeline = PromptReleasePipeline::new();
        pipeline.start_rollout("rel_new", 100, Some("rel_stable".into()));

        let decision = pipeline.resolve_with_rollout("rel_new", 0.99);
        assert_eq!(
            decision,
            RoutingDecision::Candidate {
                release_id: "rel_new".into()
            }
        );
    }

    #[test]
    fn routing_0_percent_always_stable() {
        let pipeline = PromptReleasePipeline::new();
        pipeline.start_rollout("rel_new", 0, Some("rel_stable".into()));

        let decision = pipeline.resolve_with_rollout("rel_new", 0.01);
        assert_eq!(
            decision,
            RoutingDecision::Stable {
                release_id: "rel_stable".into()
            }
        );
    }

    // ── Gap 4: Approval gate tests ───────────────────────────────────────

    #[test]
    fn request_full_release_sets_pending_approval() {
        let pipeline = PromptReleasePipeline::new();
        pipeline.start_rollout("rel_1", 50, None);

        let state = pipeline
            .request_full_release("rel_1", "apr_123".into())
            .unwrap();
        assert_eq!(state.status, RolloutStatus::PendingApproval);
        assert_eq!(state.approval_id.as_deref(), Some("apr_123"));
        // Percent stays at 50 during approval.
        assert_eq!(state.percent, 50);
    }

    #[test]
    fn approve_full_release_promotes_to_100() {
        let pipeline = PromptReleasePipeline::new();
        pipeline.start_rollout("rel_1", 50, None);
        pipeline.request_full_release("rel_1", "apr_123".into());

        let state = pipeline.approve_full_release("rel_1").unwrap();
        assert_eq!(state.percent, 100);
        assert_eq!(state.status, RolloutStatus::Complete);
    }

    #[test]
    fn approve_without_pending_returns_none() {
        let pipeline = PromptReleasePipeline::new();
        pipeline.start_rollout("rel_1", 50, None);
        // Not in PendingApproval state.
        assert!(pipeline.approve_full_release("rel_1").is_none());
    }

    #[test]
    fn routing_during_pending_approval_still_uses_percent() {
        let pipeline = PromptReleasePipeline::new();
        pipeline.start_rollout("rel_1", 50, Some("rel_stable".into()));
        pipeline.request_full_release("rel_1", "apr_1".into());

        // Traffic continues at 50% while awaiting approval.
        let low = pipeline.resolve_with_rollout("rel_1", 0.3);
        assert_eq!(
            low,
            RoutingDecision::Candidate {
                release_id: "rel_1".into()
            }
        );
        let high = pipeline.resolve_with_rollout("rel_1", 0.7);
        assert_eq!(
            high,
            RoutingDecision::Stable {
                release_id: "rel_stable".into()
            }
        );
    }

    // ── Gap 5: Rollback tests ────────────────────────────────────────────

    #[test]
    fn rollback_sets_zero_and_rolled_back() {
        let pipeline = PromptReleasePipeline::new();
        pipeline.start_rollout("rel_1", 50, Some("rel_stable".into()));

        let state = pipeline.rollback("rel_1").unwrap();
        assert_eq!(state.percent, 0);
        assert_eq!(state.status, RolloutStatus::RolledBack);
    }

    #[test]
    fn rollback_routes_all_traffic_to_stable() {
        let pipeline = PromptReleasePipeline::new();
        pipeline.start_rollout("rel_1", 50, Some("rel_stable".into()));
        pipeline.rollback("rel_1");

        let decision = pipeline.resolve_with_rollout("rel_1", 0.01);
        assert_eq!(
            decision,
            RoutingDecision::Stable {
                release_id: "rel_stable".into()
            }
        );
    }

    #[test]
    fn rollback_nonexistent_returns_none() {
        let pipeline = PromptReleasePipeline::new();
        assert!(pipeline.rollback("nonexistent").is_none());
    }

    #[test]
    fn cannot_update_rolled_back_rollout() {
        let pipeline = PromptReleasePipeline::new();
        pipeline.start_rollout("rel_1", 50, None);
        pipeline.rollback("rel_1");
        assert!(pipeline.update_rollout_percent("rel_1", 75).is_none());
    }

    // ── Integration: full pipeline flow ──────────────────────────────────

    #[test]
    fn full_pipeline_flow() {
        let pipeline = PromptReleasePipeline::new();

        // Store version content for diffing.
        pipeline.store_version_content("pv_1", "You are helpful.".into());
        pipeline.store_version_content("pv_2", "You are a helpful coding assistant.".into());

        // Diff the versions.
        let diff = pipeline.diff_versions("pv_1", "pv_2").unwrap();
        assert_eq!(diff.additions, 1);
        assert_eq!(diff.deletions, 1);

        // Start gradual rollout at 10%.
        pipeline.start_rollout("rel_new", 10, Some("rel_old".into()));
        assert_eq!(pipeline.active_rollouts().len(), 1);

        // Ramp to 50%.
        pipeline.update_rollout_percent("rel_new", 50);

        // A/B routing works.
        let low = pipeline.resolve_with_rollout("rel_new", 0.2);
        assert_eq!(
            low,
            RoutingDecision::Candidate {
                release_id: "rel_new".into()
            }
        );

        // Request approval for 100%.
        pipeline.request_full_release("rel_new", "apr_1".into());
        let state = pipeline.get_rollout_state("rel_new").unwrap();
        assert_eq!(state.status, RolloutStatus::PendingApproval);

        // Approve.
        let state = pipeline.approve_full_release("rel_new").unwrap();
        assert_eq!(state.percent, 100);
        assert_eq!(state.status, RolloutStatus::Complete);
        assert!(pipeline.active_rollouts().is_empty());
    }

    #[test]
    fn full_pipeline_flow_with_rollback() {
        let pipeline = PromptReleasePipeline::new();

        pipeline.start_rollout("rel_new", 50, Some("rel_old".into()));

        // Something goes wrong — rollback.
        let state = pipeline.rollback("rel_new").unwrap();
        assert_eq!(state.percent, 0);
        assert_eq!(state.status, RolloutStatus::RolledBack);

        // All traffic goes to stable.
        let decision = pipeline.resolve_with_rollout("rel_new", 0.0);
        assert_eq!(
            decision,
            RoutingDecision::Stable {
                release_id: "rel_old".into()
            }
        );

        // Active rollouts list is empty.
        assert!(pipeline.active_rollouts().is_empty());
    }
}
