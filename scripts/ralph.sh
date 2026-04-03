#!/usr/bin/env bash
# scripts/ralph.sh — Ralph Loop for cairn-rs
#
# A while-true loop that feeds PROMPT.md into claude -p on every iteration.
# Each iteration gets a clean context window. The filesystem is the memory.
#
# Usage:
#   ./scripts/ralph.sh [max_iterations] [prompt_file]
#   ./scripts/ralph.sh 20                        # 20 iterations, default prompt
#   ./scripts/ralph.sh 10 PROMPT-refactor.md     # custom prompt file
#
# Stop early: create a file called .ralph-stop or the agent outputs <promise>COMPLETE</promise>

set -e

MAX_ITERATIONS="${1:-10}"
PROMPT_FILE="${2:-PROMPT.md}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LOG_DIR="$REPO_ROOT/.ralph"

mkdir -p "$LOG_DIR"

if [[ ! -f "$REPO_ROOT/$PROMPT_FILE" ]]; then
  echo "ERROR: $PROMPT_FILE not found in $REPO_ROOT" >&2
  echo "Create it first with your task description, verification steps, and completion criteria." >&2
  exit 1
fi

# ── Claude flags (model comes from settings.json) ────────────────────────────
CLAUDE_FLAGS="-p --dangerously-skip-permissions"
[[ -n "${RALPH_MODEL:-}" ]] && CLAUDE_FLAGS="$CLAUDE_FLAGS --model $RALPH_MODEL"

echo "╔══════════════════════════════════════════╗"
echo "║         Ralph Loop — cairn-rs            ║"
echo "╠══════════════════════════════════════════╣"
echo "║  Prompt:     $PROMPT_FILE"
echo "║  Max iters:  $MAX_ITERATIONS"
echo "║  Logs:       .ralph/"
echo "║  Stop:       touch .ralph-stop"
echo "╚══════════════════════════════════════════╝"
echo ""

iteration=0
while [ "$iteration" -lt "$MAX_ITERATIONS" ]; do
  iteration=$((iteration + 1))
  LOG_FILE="$LOG_DIR/iter-$(printf '%03d' $iteration).log"

  echo "═══ Iteration $iteration / $MAX_ITERATIONS ═══ $(date +%H:%M:%S)"

  # Safety valve: stop file
  if [[ -f "$REPO_ROOT/.ralph-stop" ]]; then
    echo "Stop file detected. Exiting."
    rm -f "$REPO_ROOT/.ralph-stop"
    exit 0
  fi

  # Run claude with clean context, pipe prompt from file
  cd "$REPO_ROOT"
  output=$(cat "$PROMPT_FILE" | claude $CLAUDE_FLAGS 2>&1) || true
  echo "$output" > "$LOG_FILE"

  # Check completion signal
  if echo "$output" | grep -q '<promise>COMPLETE</promise>'; then
    echo ""
    echo "Task complete at iteration $iteration"
    echo "$output" | tail -20
    exit 0
  fi

  # Show last few lines of output
  echo "$output" | tail -5
  echo ""
done

echo "Hit max iterations ($MAX_ITERATIONS) without completion."
exit 1
