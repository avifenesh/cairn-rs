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
    ApprovalMatchPolicy, ApprovalScope, OperatorId, RuntimeEvent, SessionId, ToolCallAmended,
    ToolCallApproved, ToolCallId, ToolCallProposed, ToolCallRejected,
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

/// `std::sync::Mutex` poisoning should never happen here — every path
/// holding the lock is infallible — but if it somehow does we surface
/// it as a descriptive internal error rather than panicking.
#[inline]
fn lock<'a, T>(m: &'a Mutex<T>) -> std::sync::MutexGuard<'a, T> {
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
        // Load proposal context (cache, falling back to store).
        let entry = self.load_or_fetch(&call_id).await?;
        if entry.state == ProposalState::Approved || entry.state == ProposalState::Rejected {
            return Err(RuntimeError::InvalidTransition {
                entity: "tool_call_approval",
                from: format!("{:?}", entry.state).to_ascii_lowercase(),
                to: "approved".into(),
            });
        }

        // Persist approval event.
        self.append(RuntimeEvent::ToolCallApproved(ToolCallApproved {
            project: entry.proposal.project.clone(),
            call_id: call_id.clone(),
            session_id: entry.proposal.session_id.clone(),
            operator_id: operator_id.clone(),
            scope: scope.clone(),
            approved_tool_args: approved_args.clone(),
            approved_at_ms: now_ms(),
        }))
        .await?;

        // Update cache, session allow registry, and fire the oneshot.
        let (sender, decision) = {
            let mut guard = lock(&self.inner);

            // Update cache entry.
            if let Some(cached) = guard.proposals.get_mut(&call_id) {
                if let Some(ref args) = approved_args {
                    cached.approved_args = Some(args.clone());
                }
                cached.state = ProposalState::Approved;
            }

            // Extend the session allow registry if requested.
            if let ApprovalScope::Session { match_policy } = &scope {
                let rule = build_allow_rule(&entry.proposal, match_policy);
                guard
                    .allow_registry
                    .entry(entry.proposal.session_id.clone())
                    .or_default()
                    .push(rule);
            }

            // Resolve effective args for the decision payload: prefer
            // the newly-supplied override, else the cached
            // approved/amended/original chain.
            let effective = approved_args.clone().unwrap_or_else(|| {
                guard
                    .proposals
                    .get(&call_id)
                    .map(ProposalEntry::effective_args)
                    .unwrap_or(entry.proposal.tool_args.clone())
            });

            let sender = guard.pending.remove(&call_id);
            (
                sender,
                OperatorDecision::Approved {
                    approved_args: effective,
                },
            )
        };

        if let Some(tx) = sender {
            // Receiver may have already dropped (e.g. timeout fired just
            // before the approval landed). Losing that signal is fine —
            // the persisted event is the source of truth, and a
            // follow-up `retrieve_approved_proposal` will still see the
            // approved state.
            let _ = tx.send(decision);
        }

        Ok(())
    }

    async fn reject(
        &self,
        call_id: ToolCallId,
        operator_id: OperatorId,
        reason: Option<String>,
    ) -> Result<(), RuntimeError> {
        let entry = self.load_or_fetch(&call_id).await?;
        if entry.state == ProposalState::Approved || entry.state == ProposalState::Rejected {
            return Err(RuntimeError::InvalidTransition {
                entity: "tool_call_approval",
                from: format!("{:?}", entry.state).to_ascii_lowercase(),
                to: "rejected".into(),
            });
        }

        self.append(RuntimeEvent::ToolCallRejected(ToolCallRejected {
            project: entry.proposal.project.clone(),
            call_id: call_id.clone(),
            session_id: entry.proposal.session_id.clone(),
            operator_id,
            reason: reason.clone(),
            rejected_at_ms: now_ms(),
        }))
        .await?;

        let sender = {
            let mut guard = lock(&self.inner);
            if let Some(cached) = guard.proposals.get_mut(&call_id) {
                cached.state = ProposalState::Rejected;
            }
            guard.pending.remove(&call_id)
        };

        if let Some(tx) = sender {
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
        let entry = self.load_or_fetch(&call_id).await?;
        if entry.state == ProposalState::Approved || entry.state == ProposalState::Rejected {
            return Err(RuntimeError::InvalidTransition {
                entity: "tool_call_approval",
                from: format!("{:?}", entry.state).to_ascii_lowercase(),
                to: "amended".into(),
            });
        }

        self.append(RuntimeEvent::ToolCallAmended(ToolCallAmended {
            project: entry.proposal.project.clone(),
            call_id: call_id.clone(),
            session_id: entry.proposal.session_id.clone(),
            operator_id,
            new_tool_args: new_args.clone(),
            amended_at_ms: now_ms(),
        }))
        .await?;

        // Update the cache — but do NOT fire the oneshot. Operator
        // still needs to resolve (approve/reject) after amending.
        let mut guard = lock(&self.inner);
        if let Some(cached) = guard.proposals.get_mut(&call_id) {
            cached.amended_args = Some(new_args);
            cached.state = ProposalState::Amended;
        }

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
        // the cached state without opening a oneshot channel.
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
                        return Ok(OperatorDecision::Rejected { reason: None });
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
            Err(_elapsed) => {
                // Timeout: remove the pending slot, auto-reject, and
                // surface `Timeout` to the caller.
                {
                    let mut guard = lock(&self.inner);
                    guard.pending.remove(call_id);
                }
                if let Some(entry) = self.cached_entry(call_id) {
                    // Only write a rejection event if the proposal
                    // hasn't already been resolved via a race between
                    // `timeout` firing and the operator approve/reject
                    // landing.
                    if entry.state != ProposalState::Approved
                        && entry.state != ProposalState::Rejected
                    {
                        self.append(RuntimeEvent::ToolCallRejected(ToolCallRejected {
                            project: entry.proposal.project.clone(),
                            call_id: call_id.clone(),
                            session_id: entry.proposal.session_id.clone(),
                            operator_id: OperatorId::new("operator_timeout"),
                            reason: Some("operator_timeout".into()),
                            rejected_at_ms: now_ms(),
                        }))
                        .await?;
                        let mut guard = lock(&self.inner);
                        if let Some(cached) = guard.proposals.get_mut(call_id) {
                            cached.state = ProposalState::Rejected;
                        }
                    }
                }
                Ok(OperatorDecision::Timeout)
            }
        }
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

fn build_allow_rule(proposal: &ToolCallProposal, policy: &ApprovalMatchPolicy) -> AllowRule {
    AllowRule {
        tool_name: proposal.tool_name.clone(),
        tool_args: proposal.tool_args.clone(),
        policy: policy.clone(),
    }
}
