#!/usr/bin/env bash
# scripts/team-start.sh
# Starts a 1-manager + 3-worker agent team in a single tmux split view.
#
# Local   — all claude, default model
# Bedrock — all claude, Opus (manager) + Sonnet (workers) via AWS Bedrock
#
# Usage:
#   ./scripts/team-start.sh            # local, default claude
#   ./scripts/team-start.sh --bedrock  # Bedrock models (dev)
#   ./scripts/team-start.sh --resume   # re-attach existing session
#
# Role briefs are injected by the watcher on first message — no timing issues.
#
# Prereq (one-time):
#   sudo apt install inotify-tools

set -euo pipefail

SESSION="cairn-team"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
INBOX_ROOT="$REPO_ROOT/.coordination/mailbox/inbox"

# ── Flags ────────────────────────────────────────────────────────────────────
USE_BEDROCK=0
for arg in "$@"; do
  [[ "$arg" == "--bedrock" ]] && USE_BEDROCK=1
  [[ "$arg" == "--resume"  ]] && tmux attach -t "$SESSION" 2>/dev/null && exit 0
done

MANAGER_MODEL="${MANAGER_MODEL:-us.anthropic.claude-opus-4-6-v1[1m]}"
WORKER_MODEL="${WORKER_MODEL:-us.anthropic.claude-sonnet-4-6[1m]}"
# ─────────────────────────────────────────────────────────────────────────────

# ── Guards ───────────────────────────────────────────────────────────────────
if ! command -v tmux &>/dev/null; then
  echo "ERROR: tmux not found.  sudo apt install tmux" >&2; exit 1
fi
if ! command -v inotifywait &>/dev/null; then
  echo "INFO: poll mode (1s). Install inotify-tools for instant wake-ups."
fi

# ── Reset ────────────────────────────────────────────────────────────────────
tmux kill-session -t "$SESSION" 2>/dev/null || true
for agent in manager worker-1 worker-2 worker-3; do
  mkdir -p "$INBOX_ROOT/$agent"
done
# Clear first-brief flags so role is re-injected on fresh start
rm -f "$REPO_ROOT/.coordination/initialized/"*
mkdir -p "$REPO_ROOT/.coordination/prompts" "$REPO_ROOT/.coordination/initialized"

# ── tmux: 2×2 tiled split ────────────────────────────────────────────────────
#   ┌──────────────────┬──────────────────┐
#   │  MANAGER (opus)  │    WORKER 1      │
#   ├──────────────────┼──────────────────┤
#   │    WORKER 2      │    WORKER 3      │
#   └──────────────────┴──────────────────┘
# Use pane IDs (%N) — immune to pane-base-index differences across machines
P0=$(tmux new-session  -d -s "$SESSION" -n team -x 220 -y 50 -P -F "#{pane_id}")
P1=$(tmux split-window -t "$SESSION:team" -P -F "#{pane_id}")
P2=$(tmux split-window -t "$SESSION:team" -P -F "#{pane_id}")
P3=$(tmux split-window -t "$SESSION:team" -P -F "#{pane_id}")
tmux select-layout -t "$SESSION:team" tiled
tmux set -t "$SESSION" mouse on
tmux set -t "$SESSION" set-clipboard external
tmux select-pane -t "$P0" -T "MANAGER"
tmux select-pane -t "$P1" -T "WORKER 1"
tmux select-pane -t "$P2" -T "WORKER 2"
tmux select-pane -t "$P3" -T "WORKER 3"
tmux set -t "$SESSION" pane-border-status top
tmux set -t "$SESSION" pane-border-format " #{pane_title} "

# ── Watchers (background window) ─────────────────────────────────────────────
tmux new-window -t "$SESSION" -n watchers
for entry in "manager:$P0" "worker-1:$P1" "worker-2:$P2" "worker-3:$P3"; do
  AGENT="${entry%%:*}"; PANE="${entry##*:}"
  tmux send-keys -t "$SESSION:watchers" \
    "\"$REPO_ROOT/scripts/team-watch.sh\" $AGENT \"$PANE\" \"$REPO_ROOT\" &" Enter
done
tmux send-keys -t "$SESSION:watchers" "wait" Enter

# ── Build claude command ──────────────────────────────────────────────────────
if [[ "$USE_BEDROCK" == "1" ]]; then
  MGR_CMD="CLAUDE_CODE_USE_BEDROCK=1 claude --dangerously-skip-permissions --model \"$MANAGER_MODEL\""
  WRK_CMD="CLAUDE_CODE_USE_BEDROCK=1 claude --dangerously-skip-permissions --model \"$WORKER_MODEL\""
else
  MGR_CMD="claude --dangerously-skip-permissions"
  WRK_CMD="claude --dangerously-skip-permissions"
fi

# ── Launch agents ─────────────────────────────────────────────────────────────
tmux send-keys -t "$P0" "cd \"$REPO_ROOT\" && $MGR_CMD" Enter
for i in 1 2 3; do
  PANE_ID="P$i"; PANE="${!PANE_ID}"
  # Generate per-worker prompt file
  sed "s/{{WORKER_ID}}/worker-$i/g; s/{{WORKER_NUM}}/$i/g" \
    "$REPO_ROOT/.coordination/prompts/worker-template.md" \
    > "$REPO_ROOT/.coordination/prompts/worker-$i.md" 2>/dev/null || true
  tmux send-keys -t "$PANE" "cd \"$REPO_ROOT\" && $WRK_CMD" Enter
done

# ── Switch to team view ───────────────────────────────────────────────────────
tmux select-window -t "$SESSION:team"
tmux select-pane   -t "$P0"

SEND="$REPO_ROOT/scripts/team-send.sh"
[[ "$REPO_ROOT" == "$HOME" ]] && SEND="~/scripts/team-send.sh"

echo ""
echo "Team started. Attach with:"
echo "  tmux attach -t $SESSION"
echo ""
echo "Send first task to manager:"
echo "  $SEND manager you 'your task here'"
