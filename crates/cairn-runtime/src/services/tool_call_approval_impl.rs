//! In-memory + event-log-backed implementation of [`ToolCallApprovalService`].
//!
//! Responsibilities:
//!
//! * Proposal cache keyed by [`ToolCallId`] — fast path for execute ->
//!   await -> retrieve without touching the store.
//! * Session allow registry keyed by [`SessionId`] — `Session`-scoped
//!   approvals widen to subsequent matching proposals.
//! * Pending oneshot senders keyed by `ToolCallId` — the execute phase
//!   parks on `await_decision`; operator `approve`/`reject` fires the
//!   sender. `amend` deliberately does NOT fire.
//! * Event-log emission of the four domain events (`ToolCallProposed`,
//!   `ToolCallApproved`, `ToolCallRejected`, `ToolCallAmended`).
//! * Store fallback via [`ToolCallApprovalReader`] when the cache misses
//!   (restart, eviction).
//!
//! Not yet wired:
//!
//! * PR-2 lands the `ToolCallApprovalReader` impl on `cairn-store`.
//! * PR-5 wires the orchestrator to call this service.
//! * PR-6 wires the operator HTTP handlers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use cairn_domain::{
    ApprovalScope, OperatorId, RuntimeEvent, SessionId, ToolCallAmended, ToolCallApproved,
    ToolCallId, ToolCallProposed, ToolCallRejected,
};
use cairn_store::EventLog;
use serde_json::Value;
use tokio::sync::oneshot;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::tool_call_approvals::{
    proposal_matches_rule, AllowRule, ApprovalDecision, ApprovedProposal, OperatorDecision,
    ToolCallApprovalReader, ToolCallApprovalService, ToolCallProposal,
};

/// Current lifecycle state of a proposal held in the cache.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ProposalState {
    Pending,
    Amended,
    Approved,
    Rejected,
}

/// Cache entry for one proposal.
#[derive(Clone, Debug)]
struct ProposalEntry {
    proposal: ToolCallProposal,
    state: ProposalState,
    /// Most recent `ToolCallAmended.new_tool_args`, if any.
    amended_args: Option<Value>,
    /// `ToolCallApproved.approved_tool_args`, if any.
    approved_args: Option<Value>,
    /// `ToolCallRejected.reason`, retained so the `await_decision`
    /// fast-path (fired when rejection landed before the caller parked)
    /// can surface the actual rejection reason — including the
    /// `"operator_timeout"` sentinel — rather than silently dropping it.
    rejection_reason: Option<String>,
}

impl ProposalEntry {
    /// Resolve the arguments the execute phase should actually run
    /// (per the domain precedence invariant).
    fn effective_args(&self) -> Value {
        self.approved_args
            .clone()
            .or_else(|| self.amended_args.clone())
            .unwrap_or_else(|| self.proposal.tool_args.clone())
    }
}

type PendingMap = HashMap<ToolCallId, oneshot::Sender<OperatorDecision>>;

/// Shared in-memory state. A single `Mutex` keeps invariants atomic
/// (cache update + pending-map update + allow-registry update must all
/// see the same world); contention is not a concern given typical
/// throughput (one submit per tool call per run per operator session).
struct Inner {
    proposals: HashMap<ToolCallId, ProposalEntry>,
    pending: PendingMap,
    allow_registry: HashMap<SessionId, Vec<AllowRule>>,
}

impl Inner {
    fn new() -> Self {
        Self {
            proposals: HashMap::new(),
            pending: HashMap::new(),
            allow_registry: HashMap::new(),
        }
    }
}

/// Implementation of [`ToolCallApprovalService`] backed by an event log
/// plus an in-memory cache.
///
/// `S` is an `EventLog` implementation (Postgres / SQLite / InMemory).
/// `R` is the store projection reader (supplied separately so tests can
/// stub the store without spinning up a real projection).
pub struct ToolCallApprovalServiceImpl<S, R>
where
    S: EventLog + 'static,
    R: ToolCallApprovalReader + 'static,
{
    store: Arc<S>,
    reader: Arc<R>,
    inner: Arc<Mutex<Inner>>,
}

impl<S, R> ToolCallApprovalServiceImpl<S, R>
where
    S: EventLog + 'static,
    R: ToolCallApprovalReader + 'static,
{
    pub fn new(store: Arc<S>, reader: Arc<R>) -> Self {
        Self {
            store,
            reader,
            inner: Arc::new(Mutex::new(Inner::new())),
        }
    }

    /// Append a single runtime event through the store.
    async fn append(&self, event: RuntimeEvent) -> Result<(), RuntimeError> {
        self.store.append(&[make_envelope(event)]).await?;
        Ok(())
    }

    /// Look up the caller-supplied proposal in the cache. Clone so we
    /// don't hold the lock across the await path.
    fn cached_entry(&self, call_id: &ToolCallId) -> Option<ProposalEntry> {
        let guard = lock(&self.inner);
        guard.proposals.get(call_id).cloned()
    }

    /// Evaluate a fresh proposal against the session allow registry.
    /// Returns `Some(rule)` when the proposal should auto-approve.
    fn find_matching_rule(&self, proposal: &ToolCallProposal) -> Option<AllowRule> {
        let guard = lock(&self.inner);
        let rules = guard.allow_registry.get(&proposal.session_id)?;
        for rule in rules {
            if proposal_matches_rule(proposal, rule) {
                return Some(rule.clone());
            }
        }
        None
    }
}

/// Acquire the shared-state mutex. Every path holding this lock is
/// infallible (no panic-capable calls inside critical sections), so if
/// the lock has somehow been poisoned we prefer pressing on with the
/// inner state over propagating a poison panic up the async stack: the
/// event log is the source of truth, not this cache, and refusing to
/// serve an approve/reject because of a background panic elsewhere
/// would strand runs waiting on operator decisions. This choice is
/// intentional; it is not a "surface the error" policy.
#[inline]
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[async_trait]
impl<S, R> ToolCallApprovalService for ToolCallApprovalServiceImpl<S, R>
where
    S: EventLog + 'static,
    R: ToolCallApprovalReader + 'static,
{
    async fn submit_proposal(
        &self,
        proposal: ToolCallProposal,
    ) -> Result<ApprovalDecision, RuntimeError> {
        // 1. Persist the proposal event.
        self.append(RuntimeEvent::ToolCallProposed(ToolCallProposed {
            project: proposal.project.clone(),
            call_id: proposal.call_id.clone(),
            session_id: proposal.session_id.clone(),
            run_id: proposal.run_id.clone(),
            tool_name: proposal.tool_name.clone(),
            tool_args: proposal.tool_args.clone(),
            display_summary: proposal.display_summary.clone().unwrap_or_default(),
            match_policy: proposal.match_policy.clone(),
            proposed_at_ms: now_ms(),
        }))
        .await?;

        // 2. Cache the proposal.
        {
            let mut guard = lock(&self.inner);
            guard.proposals.insert(
                proposal.call_id.clone(),
                ProposalEntry {
                    proposal: proposal.clone(),
                    state: ProposalState::Pending,
                    amended_args: None,
                    approved_args: None,
                    rejection_reason: None,
                },
            );
        }

        // 3. Evaluate against session allow registry.
        if let Some(_rule) = self.find_matching_rule(&proposal) {
            // Auto-approve path: synthesize an approval event so the
            // audit log is self-contained and projections treat this
            // call identically to an operator-driven approval.
            self.append(RuntimeEvent::ToolCallApproved(ToolCallApproved {
                project: proposal.project.clone(),
                call_id: proposal.call_id.clone(),
                session_id: proposal.session_id.clone(),
                operator_id: OperatorId::new("session_allow"),
                scope: ApprovalScope::Once,
                approved_tool_args: None,
                approved_at_ms: now_ms(),
            }))
            .await?;

            let mut guard = lock(&self.inner);
            if let Some(entry) = guard.proposals.get_mut(&proposal.call_id) {
                entry.state = ProposalState::Approved;
            }
            return Ok(ApprovalDecision::AutoApproved);
        }

        Ok(ApprovalDecision::PendingOperator)
    }

    async fn approve(
        &self,
        call_id: ToolCallId,
        operator_id: OperatorId,
        scope: ApprovalScope,
        approved_args: Option<Value>,
    ) -> Result<(), RuntimeError> {
        // Ensure the cache is populated for this call_id.
        let _ = self.load_or_fetch(&call_id).await?;

        // ── Phase 1: atomically claim the Pending→Approved transition ───
        //
        // Capture everything needed to emit the event and fire the
        // oneshot *while holding the mutex*, so no concurrent
        // approve/reject/timeout can also observe `Pending` and
        // produce a second resolution event.
        struct ClaimedContext {
            project_key: cairn_domain::ProjectKey,
            session_id: SessionId,
            previous_state: ProposalState,
            effective_args: Value,
            allow_rule: Option<AllowRule>,
            sender: Option<oneshot::Sender<OperatorDecision>>,
        }

        let ctx = {
            let mut guard = lock(&self.inner);
            let cached =
                guard
                    .proposals
                    .get_mut(&call_id)
                    .ok_or_else(|| RuntimeError::NotFound {
                        entity: "tool_call_approval",
                        id: call_id.to_string(),
                    })?;
            if cached.state == ProposalState::Approved || cached.state == ProposalState::Rejected {
                return Err(RuntimeError::InvalidTransition {
                    entity: "tool_call_approval",
                    from: format!("{:?}", cached.state).to_ascii_lowercase(),
                    to: "approved".into(),
                });
            }

            // Fold the operator-supplied override into the cache *before*
            // computing `effective_args` so the allow-rule below captures
            // the actual post-approval args, not the pre-override ones.
            if let Some(ref args) = approved_args {
                cached.approved_args = Some(args.clone());
            }
            let effective_args = cached.effective_args();
            let project_key = cached.proposal.project.clone();
            let session_id = cached.proposal.session_id.clone();
            let tool_name = cached.proposal.tool_name.clone();

            // Build allow rule from *effective* args so that a later
            // auto-approve matches what the operator actually sanctioned
            // — not an unsafe pre-amendment payload. This addresses the
            // Copilot review finding on line 280.
            let allow_rule = if let ApprovalScope::Session { match_policy } = &scope {
                Some(AllowRule {
                    tool_name,
                    tool_args: effective_args.clone(),
                    policy: match_policy.clone(),
                })
            } else {
                None
            };

            let previous_state = cached.state.clone();
            cached.state = ProposalState::Approved;
            let sender = guard.pending.remove(&call_id);

            ClaimedContext {
                project_key,
                session_id,
                previous_state,
                effective_args,
                allow_rule,
                sender,
            }
        };

        // ── Phase 2: persist the approval event ─────────────────────────
        let append_result = self
            .append(RuntimeEvent::ToolCallApproved(ToolCallApproved {
                project: ctx.project_key.clone(),
                call_id: call_id.clone(),
                session_id: ctx.session_id.clone(),
                operator_id,
                scope: scope.clone(),
                approved_tool_args: approved_args.clone(),
                approved_at_ms: now_ms(),
            }))
            .await;

        if let Err(err) = append_result {
            // Revert the claimed transition so a retry from the caller
            // or a concurrent decision path can re-enter cleanly.
            let mut guard = lock(&self.inner);
            if let Some(cached) = guard.proposals.get_mut(&call_id) {
                if cached.state == ProposalState::Approved {
                    cached.state = ctx.previous_state;
                    if approved_args.is_some() {
                        cached.approved_args = None;
                    }
                }
            }
            return Err(err);
        }

        // ── Phase 3: post-commit side effects ───────────────────────────
        if let Some(rule) = ctx.allow_rule {
            let mut guard = lock(&self.inner);
            guard
                .allow_registry
                .entry(ctx.session_id.clone())
                .or_default()
                .push(rule);
        }

        if let Some(tx) = ctx.sender {
            // Receiver may have already dropped (e.g. timeout fired
            // between Phase 1 and Phase 2). Losing that signal is fine
            // — the persisted event is the source of truth, and a
            // follow-up `retrieve_approved_proposal` / `await_decision`
            // fast-path will still see the approved state.
            let _ = tx.send(OperatorDecision::Approved {
                approved_args: ctx.effective_args,
            });
        }

        Ok(())
    }

    async fn reject(
        &self,
        call_id: ToolCallId,
        operator_id: OperatorId,
        reason: Option<String>,
    ) -> Result<(), RuntimeError> {
        let _ = self.load_or_fetch(&call_id).await?;

        struct RejectCtx {
            project_key: cairn_domain::ProjectKey,
            session_id: SessionId,
            previous_state: ProposalState,
            previous_reason: Option<String>,
            sender: Option<oneshot::Sender<OperatorDecision>>,
        }

        // ── Phase 1: atomically claim the transition. ──────────────────
        let ctx = {
            let mut guard = lock(&self.inner);
            let cached =
                guard
                    .proposals
                    .get_mut(&call_id)
                    .ok_or_else(|| RuntimeError::NotFound {
                        entity: "tool_call_approval",
                        id: call_id.to_string(),
                    })?;
            if cached.state == ProposalState::Approved || cached.state == ProposalState::Rejected {
                return Err(RuntimeError::InvalidTransition {
                    entity: "tool_call_approval",
                    from: format!("{:?}", cached.state).to_ascii_lowercase(),
                    to: "rejected".into(),
                });
            }
            let previous_state = cached.state.clone();
            let previous_reason = cached.rejection_reason.clone();
            cached.state = ProposalState::Rejected;
            cached.rejection_reason = reason.clone();
            let project_key = cached.proposal.project.clone();
            let session_id = cached.proposal.session_id.clone();
            let sender = guard.pending.remove(&call_id);
            RejectCtx {
                project_key,
                session_id,
                previous_state,
                previous_reason,
                sender,
            }
        };

        // ── Phase 2: persist the rejection event. ──────────────────────
        let append_result = self
            .append(RuntimeEvent::ToolCallRejected(ToolCallRejected {
                project: ctx.project_key,
                call_id: call_id.clone(),
                session_id: ctx.session_id,
                operator_id,
                reason: reason.clone(),
                rejected_at_ms: now_ms(),
            }))
            .await;

        if let Err(err) = append_result {
            let mut guard = lock(&self.inner);
            if let Some(cached) = guard.proposals.get_mut(&call_id) {
                if cached.state == ProposalState::Rejected {
                    cached.state = ctx.previous_state;
                    cached.rejection_reason = ctx.previous_reason;
                }
            }
            return Err(err);
        }

        if let Some(tx) = ctx.sender {
            let _ = tx.send(OperatorDecision::Rejected { reason });
        }
        Ok(())
    }

    async fn amend(
        &self,
        call_id: ToolCallId,
        operator_id: OperatorId,
        new_args: Value,
    ) -> Result<(), RuntimeError> {
        let _ = self.load_or_fetch(&call_id).await?;

        // Claim the state transition (Pending|Amended → Amended) under
        // the mutex. Unlike approve/reject, Amended is not terminal —
        // re-entry is fine — but we still want to refuse amendments
        // over an already-resolved proposal.
        let (project_key, session_id, previous_state, previous_amended) = {
            let mut guard = lock(&self.inner);
            let cached =
                guard
                    .proposals
                    .get_mut(&call_id)
                    .ok_or_else(|| RuntimeError::NotFound {
                        entity: "tool_call_approval",
                        id: call_id.to_string(),
                    })?;
            if cached.state == ProposalState::Approved || cached.state == ProposalState::Rejected {
                return Err(RuntimeError::InvalidTransition {
                    entity: "tool_call_approval",
                    from: format!("{:?}", cached.state).to_ascii_lowercase(),
                    to: "amended".into(),
                });
            }
            let previous_state = cached.state.clone();
            let previous_amended = cached.amended_args.clone();
            cached.amended_args = Some(new_args.clone());
            cached.state = ProposalState::Amended;
            (
                cached.proposal.project.clone(),
                cached.proposal.session_id.clone(),
                previous_state,
                previous_amended,
            )
        };

        let append_result = self
            .append(RuntimeEvent::ToolCallAmended(ToolCallAmended {
                project: project_key,
                call_id: call_id.clone(),
                session_id,
                operator_id,
                new_tool_args: new_args,
                amended_at_ms: now_ms(),
            }))
            .await;

        if let Err(err) = append_result {
            let mut guard = lock(&self.inner);
            if let Some(cached) = guard.proposals.get_mut(&call_id) {
                cached.state = previous_state;
                cached.amended_args = previous_amended;
            }
            return Err(err);
        }

        // No oneshot firing — operator still needs to resolve after amending.
        Ok(())
    }

    async fn retrieve_approved_proposal(
        &self,
        call_id: &ToolCallId,
    ) -> Result<ApprovedProposal, RuntimeError> {
        if let Some(entry) = self.cached_entry(call_id) {
            if entry.state != ProposalState::Approved {
                return Err(RuntimeError::InvalidTransition {
                    entity: "tool_call_approval",
                    from: format!("{:?}", entry.state).to_ascii_lowercase(),
                    to: "retrieved".into(),
                });
            }
            return Ok(ApprovedProposal {
                call_id: call_id.clone(),
                tool_name: entry.proposal.tool_name.clone(),
                tool_args: entry.effective_args(),
            });
        }

        // Cache miss — fall back to the store projection.
        self.reader
            .get_tool_call_approval(call_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "tool_call_approval",
                id: call_id.to_string(),
            })
    }

    async fn await_decision(
        &self,
        call_id: &ToolCallId,
        timeout: Duration,
    ) -> Result<OperatorDecision, RuntimeError> {
        // Fast path: if approve/reject already landed (race between
        // submit_proposal and the await), return immediately based on
        // the cached state without opening a oneshot channel. Rejection
        // reason is preserved so callers see the actual reason (incl.
        // the `"operator_timeout"` sentinel), not an ambiguous `None`.
        {
            let guard = lock(&self.inner);
            if let Some(entry) = guard.proposals.get(call_id) {
                match entry.state {
                    ProposalState::Approved => {
                        return Ok(OperatorDecision::Approved {
                            approved_args: entry.effective_args(),
                        });
                    }
                    ProposalState::Rejected => {
                        return Ok(OperatorDecision::Rejected {
                            reason: entry.rejection_reason.clone(),
                        });
                    }
                    _ => {}
                }
            }
        }

        let (tx, rx) = oneshot::channel();
        {
            let mut guard = lock(&self.inner);
            // If a pending sender already exists for this call_id, the
            // caller issued concurrent `await_decision`s — that's a
            // programming error (one executor owns the call), so refuse
            // loudly rather than silently dropping the older sender.
            if guard.pending.contains_key(call_id) {
                return Err(RuntimeError::Conflict {
                    entity: "tool_call_approval_await",
                    id: call_id.to_string(),
                });
            }
            guard.pending.insert(call_id.clone(), tx);
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(decision)) => Ok(decision),
            Ok(Err(_canceled)) => {
                // Sender was dropped without firing — treat as
                // internal inconsistency rather than silently returning
                // a fake decision.
                Err(RuntimeError::Internal(
                    "approval oneshot dropped without a decision".into(),
                ))
            }
            Err(_elapsed) => self.handle_timeout(call_id).await,
        }
    }
}

impl<S, R> ToolCallApprovalServiceImpl<S, R>
where
    S: EventLog + 'static,
    R: ToolCallApprovalReader + 'static,
{
    /// Handle an `await_decision` timeout by atomically claiming the
    /// Pending/Amended → Rejected transition, then appending the
    /// `ToolCallRejected` event.
    ///
    /// Phases mirror `approve`/`reject` so a concurrent operator call
    /// and a timeout cannot both emit resolution events: whichever
    /// acquires the mutex first claims the transition, and the other
    /// observes `Approved`/`Rejected` and bails out.
    async fn handle_timeout(&self, call_id: &ToolCallId) -> Result<OperatorDecision, RuntimeError> {
        let reason = "operator_timeout".to_owned();

        // ── Phase 1: atomic claim. ──────────────────────────────────────
        struct TimeoutCtx {
            project_key: cairn_domain::ProjectKey,
            session_id: SessionId,
            previous_state: ProposalState,
            previous_reason: Option<String>,
        }

        let claimed: Option<TimeoutCtx> = {
            let mut guard = lock(&self.inner);
            guard.pending.remove(call_id);
            match guard.proposals.get_mut(call_id) {
                Some(cached)
                    if cached.state != ProposalState::Approved
                        && cached.state != ProposalState::Rejected =>
                {
                    let previous_state = cached.state.clone();
                    let previous_reason = cached.rejection_reason.clone();
                    cached.state = ProposalState::Rejected;
                    cached.rejection_reason = Some(reason.clone());
                    Some(TimeoutCtx {
                        project_key: cached.proposal.project.clone(),
                        session_id: cached.proposal.session_id.clone(),
                        previous_state,
                        previous_reason,
                    })
                }
                // Already resolved by a concurrent approve/reject, or
                // the cache entry vanished (evicted). Nothing more to do.
                _ => None,
            }
        };

        // ── Phase 2: persist the rejection event (only if we claimed). ─
        if let Some(ctx) = claimed {
            let append_result = self
                .append(RuntimeEvent::ToolCallRejected(ToolCallRejected {
                    project: ctx.project_key,
                    call_id: call_id.clone(),
                    session_id: ctx.session_id,
                    operator_id: OperatorId::new("operator_timeout"),
                    reason: Some(reason),
                    rejected_at_ms: now_ms(),
                }))
                .await;
            if let Err(err) = append_result {
                // Revert so a caller who retries doesn't see a
                // ghost-rejected state with no persisted event.
                let mut guard = lock(&self.inner);
                if let Some(cached) = guard.proposals.get_mut(call_id) {
                    if cached.state == ProposalState::Rejected {
                        cached.state = ctx.previous_state;
                        cached.rejection_reason = ctx.previous_reason;
                    }
                }
                return Err(err);
            }
        }

        Ok(OperatorDecision::Timeout)
    }
}

impl<S, R> ToolCallApprovalServiceImpl<S, R>
where
    S: EventLog + 'static,
    R: ToolCallApprovalReader + 'static,
{
    /// Load a proposal from cache, falling back to a store re-hydration
    /// from the projection. Used by approve/reject/amend.
    async fn load_or_fetch(&self, call_id: &ToolCallId) -> Result<ProposalEntry, RuntimeError> {
        if let Some(entry) = self.cached_entry(call_id) {
            return Ok(entry);
        }
        // Cache miss — best-effort re-hydration from projection. Without
        // the projection we cannot reconstruct the ProjectKey / SessionId
        // required to emit the decision event, so a miss is fatal.
        //
        // PR-2 extends `ToolCallApprovalReader` so miss-path callers can
        // rebuild a `ProposalEntry`. For now, surface a clean NotFound.
        let _approved = self.reader.get_tool_call_approval(call_id).await?;
        Err(RuntimeError::NotFound {
            entity: "tool_call_approval",
            id: call_id.to_string(),
        })
    }
}
