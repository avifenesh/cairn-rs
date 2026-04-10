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

type_text() {
  # Atomically deliver text to a tmux pane via paste-buffer with bracketed
  # paste (-p), then submit with C-m.
  #
  # Why not `send-keys -l $text; sleep; Enter`:
  #   - `send-keys -l` types character-by-character at the TUI's paste speed.
  #     Long messages exceed the fixed post-sleep, so the Enter arrives before
  #     the TUI has finished buffering and gets absorbed as text. This is the
  #     bug this function exists to fix.
  #   - Bracketed paste (ESC [200~ ... ESC [201~) wraps the content so modern
  #     TUIs (including Claude Code's) recognize it as one atomic chunk. The
  #     TUI knows the paste is complete when it sees the end marker; the
  #     subsequent Enter is guaranteed to land after.
  local pane="$1"
  local text="$2"
  local buf="team-msg-$$-$RANDOM"

  # Load text into a named tmux paste buffer. `-` reads from stdin, which
  # avoids any argv-length limits and handles embedded special characters.
  if ! tmux load-buffer -b "$buf" - <<<"$text"; then
    echo "[watch] load-buffer failed for pane $pane" >&2
    return 1
  fi

  # Paste with -p (bracketed paste). Single atomic operation; no per-char
  # timing; no sleep-tuning required.
  tmux paste-buffer -b "$buf" -t "$pane" -p

  # Release the named buffer so we don't leak them on long-running watchers.
  tmux delete-buffer -b "$buf" 2>/dev/null || true

  # Short, bounded pause so the TUI finishes processing the paste end marker
  # before we submit. 300ms is plenty — the paste is already delivered, we
  # only need the TUI to flush its internal buffer.
  sleep 0.3

  # Submit. C-m (carriage return) is more reliable than the "Enter" keyword
  # across terminal modes — Enter can get remapped to LF in some readline
  # configs and fail to submit.
  tmux send-keys -t "$pane" C-m
}

inject() {
  local file="$1"
  local from body

  # Single python3 call: prints from on line 1, body on remaining lines.
  # On parse failure (malformed JSON from a broken sender) we loudly flag it
  # instead of silently cat-ing the raw file under from=? — the silent fallback
  # masked a team-send.sh LF-escape bug that dropped every multi-line message.
  local parsed rc
  if parsed="$(python3 -c "
import json, sys
try:
    d = json.load(open(sys.argv[1]))
    print(d.get('from','?'))
    print(d.get('body',''), end='')
except Exception as e:
    print('?(json-parse-failed: ' + str(e) + ')')
    print(open(sys.argv[1]).read(), end='')
" "$file" 2>&1)"; then
    from="${parsed%%$'\n'*}"
    body="${parsed#*$'\n'}"
    rc=0
  else
    from="?(python-crashed)"
    body="$(cat "$file")"
    rc=1
  fi

  # Park malformed payloads in a dead-letter dir instead of losing them, so
  # the sender can inspect what actually landed on disk.
  if [[ "$from" == \?* ]]; then
    local dlq="$REPO_ROOT/.coordination/mailbox/dead-letter/$AGENT"
    mkdir -p "$dlq"
    cp "$file" "$dlq/$(basename "$file")" 2>/dev/null || true
    echo "[watch:$AGENT] MALFORMED MESSAGE parked in $dlq ($(basename "$file")) — from=$from rc=$rc" >&2
  fi

  rm -f "$file"

  # On first message: send role brief as a separate submission first
  if [[ ! -f "$INIT_FLAG" ]]; then
    touch "$INIT_FLAG"
    local brief_file="$PROMPTS/$AGENT.md"
    if [[ -f "$brief_file" ]]; then
      local brief
      brief="$(tr '\n' ' ' < "$brief_file" | sed 's/  */ /g; s/# //g; s/## //g')"
      echo "[watch:$AGENT] sending role brief"
      type_text "$PANE" "$brief"
      sleep 2
    fi
  fi

  echo "[watch:$AGENT] injecting from=$from"
  type_text "$PANE" "Message from ${from}: ${body}"
}

# ── inotifywait (instant) — skip on /mnt/ (WSL Windows FS doesn't support inotify)
if command -v inotifywait &>/dev/null && [[ "$INBOX" != /mnt/* ]]; then
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
