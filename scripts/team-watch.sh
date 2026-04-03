#!/usr/bin/env bash
# scripts/team-watch.sh  <agent>  <tmux-pane-id>  <repo-root>
# Watches inbox and injects messages into the agent's pane.
# On first message, prepends the role brief so timing doesn't matter.

set -euo pipefail

AGENT="${1:?}"
PANE="${2:?}"
REPO_ROOT="${3:-$HOME}"

INBOX="$REPO_ROOT/.coordination/mailbox/inbox/$AGENT"
INIT_FLAG="$REPO_ROOT/.coordination/initialized/$AGENT"
PROMPTS="$REPO_ROOT/.coordination/prompts"
mkdir -p "$INBOX" "$(dirname "$INIT_FLAG")"

inject() {
  local file="$1"
  local from body prefix=""

  from="$(python3 -c "import json,sys; d=json.load(open(sys.argv[1])); print(d.get('from','?'))" "$file" 2>/dev/null || echo "?")"
  body="$(python3 -c "import json,sys; d=json.load(open(sys.argv[1])); print(d.get('body',''))" "$file" 2>/dev/null || cat "$file")"
  rm -f "$file"

  # On first message: send role brief as a separate submission first
  if [[ ! -f "$INIT_FLAG" ]]; then
    touch "$INIT_FLAG"
    local brief_file="$PROMPTS/$AGENT.md"
    if [[ -f "$brief_file" ]]; then
      # Flatten to one line — newlines cause premature Enter over tmux send-keys
      local brief
      brief="$(tr '\n' ' ' < "$brief_file" | sed 's/  */ /g; s/# //g; s/## //g')"
      echo "[watch:$AGENT] sending role brief"
      tmux send-keys -t "$PANE" "$brief" Enter
      sleep 2
    fi
  fi

  echo "[watch:$AGENT] injecting from=$from"
  tmux send-keys -t "$PANE" "Message from ${from}: ${body}" Enter
}

# ── inotifywait (instant) ────────────────────────────────────────────────────
if command -v inotifywait &>/dev/null; then
  echo "[watch:$AGENT] ready (inotifywait)"
  inotifywait -m -q -e close_write,moved_to --format '%f' "$INBOX" |
  while IFS= read -r fname; do
    [[ "$fname" == msg-*.json ]] || continue
    inject "$INBOX/$fname"
  done
  exit 0
fi

# ── Poll fallback (1s) ───────────────────────────────────────────────────────
echo "[watch:$AGENT] ready (poll)"
while true; do
  for f in "$INBOX"/msg-*.json; do
    [[ -f "$f" ]] || continue
    inject "$f"
  done
  sleep 1
done
