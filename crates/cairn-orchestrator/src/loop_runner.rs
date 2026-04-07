//! OrchestratorLoop — ties GatherPhase → DecidePhase → ExecutePhase together.
//!
//! The loop drives one run from start to a terminal state (or a suspension
//! point like `waiting_approval` or `waiting_dependency`).  It is the only
//! component that advances the run through the GATHER → DECIDE → EXECUTE
//! cycle, enforces the iteration cap and wall-clock timeout, and decides
//! when to save checkpoints.

use crate::context::{LoopConfig, LoopSignal, LoopTermination, OrchestrationContext};
use crate::decide::DecidePhase;
use crate::error::OrchestratorError;
use crate::execute::ExecutePhase;
use crate::gather::GatherPhase;

/// Drives the GATHER → DECIDE → EXECUTE loop for a single run.
///
/// # Type parameters
/// - `G`: a [`GatherPhase`] implementation
/// - `D`: a [`DecidePhase`] implementation
/// - `E`: an [`ExecutePhase`] implementation
///
/// # Loop contract (per RFC 005)
/// - One `OrchestratorLoop` instance owns exactly one run's execution.
/// - It is the only writer to run/task state for that run (via services).
/// - All state changes flow through the runtime event model (RFC 002).
/// - The loop may be suspended and resumed (approval / subagent wait).
pub struct OrchestratorLoop<G, D, E> {
    gather:  G,
    decide:  D,
    execute: E,
    config:  LoopConfig,
}

impl<G, D, E> OrchestratorLoop<G, D, E>
where
    G: GatherPhase,
    D: DecidePhase,
    E: ExecutePhase,
{
    /// Construct a new loop with the given phases and configuration.
    pub fn new(gather: G, decide: D, execute: E, config: LoopConfig) -> Self {
        Self { gather, decide, execute, config }
    }

    /// Drive the GATHER → DECIDE → EXECUTE cycle until a terminal state.
    ///
    /// Returns `Ok(LoopTermination)` for all expected stop conditions
    /// (done, failed, max-iterations, timeout, approval-wait, subagent-wait).
    /// Returns `Err(OrchestratorError)` only for unexpected infrastructure errors.
    ///
    /// # Resume
    /// To resume from a checkpoint, build the `OrchestrationContext` with the
    /// iteration counter and step_history restored from the checkpoint data,
    /// then call this method again.  The gather phase will see the current
    /// memory/event state; the loop picks up from where it left off.
    pub async fn run(
        &self,
        mut ctx: OrchestrationContext,
    ) -> Result<LoopTermination, OrchestratorError> {
        let deadline_ms = ctx.run_started_at_ms + self.config.timeout_ms;

        for _iter in 0..self.config.max_iterations {
            // ── Timeout check ─────────────────────────────────────────────────
            let now_ms = now_millis();
            if now_ms >= deadline_ms {
                return Ok(LoopTermination::TimedOut);
            }

            // ── GATHER ────────────────────────────────────────────────────────
            let gather_output = self.gather.gather(&ctx).await?;

            // ── DECIDE ────────────────────────────────────────────────────────
            let decide_output = self.decide.decide(&ctx, &gather_output).await?;

            // ── Approval pre-check ────────────────────────────────────────────
            // When the decide phase signals that approval is required, the
            // execute phase is skipped and the loop suspends.  The execute
            // phase is responsible for creating the ApprovalRequest event and
            // transitioning the run to `waiting_approval` when it detects that
            // `requires_approval` is true.
            if decide_output.requires_approval {
                let outcome = self.execute.execute(&ctx, &decide_output).await?;
                // Execute phase emits ApprovalRequested and returns WaitApproval.
                for result in &outcome.results {
                    if let crate::context::ActionStatus::AwaitingApproval { approval_id } =
                        &result.status
                    {
                        return Ok(LoopTermination::WaitingApproval {
                            approval_id: approval_id.clone(),
                        });
                    }
                }
                // Fallthrough: approval was somehow not set — treat as a failure.
                return Err(OrchestratorError::Execute(
                    "requires_approval was true but no AwaitingApproval status returned"
                        .to_owned(),
                ));
            }

            // ── EXECUTE ───────────────────────────────────────────────────────
            let execute_outcome = self.execute.execute(&ctx, &decide_output).await?;

            // ── Loop signal ───────────────────────────────────────────────────
            match execute_outcome.loop_signal {
                LoopSignal::Done => {
                    let summary = decide_output
                        .proposals
                        .iter()
                        .find(|p| p.action_type == cairn_domain::ActionType::CompleteRun)
                        .map(|p| p.description.clone())
                        .unwrap_or_else(|| "completed".to_owned());
                    return Ok(LoopTermination::Completed { summary });
                }
                LoopSignal::Failed { reason } => {
                    return Ok(LoopTermination::Failed { reason });
                }
                LoopSignal::WaitApproval { approval_id } => {
                    return Ok(LoopTermination::WaitingApproval { approval_id });
                }
                LoopSignal::WaitSubagent { child_task_id } => {
                    return Ok(LoopTermination::WaitingSubagent { child_task_id });
                }
                LoopSignal::Continue => {
                    ctx.iteration = ctx.iteration.saturating_add(1);
                }
            }
        }

        Ok(LoopTermination::MaxIterationsReached)
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
