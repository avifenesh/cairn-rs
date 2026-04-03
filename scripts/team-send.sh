#!/usr/bin/env bash
# scripts/team-send.sh  <to>  <from>  <message>
#
# Atomically drops a JSON message into an agent's inbox.
# The inbox watcher picks it up and wakes the agent automatically.
#
# Examples:
#   ./scripts/team-send.sh worker-1 manager "Implement the executor module in crates/cairn-agent/src/executor.rs"
#   ./scripts/team-send.sh manager worker-1 "Done. executor.rs written and cargo test passes."

set -euo pipefail

TO="${1:?Usage: team-send.sh <to> <from> <message>}"
FROM="${2:?Usage: team-send.sh <to> <from> <message>}"
BODY="${3:?Usage: team-send.sh <to> <from> <message>}"

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
INBOX="$REPO_ROOT/.coordination/mailbox/inbox/$TO"
mkdir -p "$INBOX"

TIMESTAMP="$(date -Iseconds)"
NANO="$(date +%s%N)"
TMP="$INBOX/.tmp-$NANO.json"
FINAL="$INBOX/msg-$NANO.json"

# Escape body for JSON (handle quotes and backslashes)
ESCAPED_BODY="${BODY//\\/\\\\}"
ESCAPED_BODY="${ESCAPED_BODY//\"/\\\"}"

printf '{"from":"%s","to":"%s","ts":"%s","body":"%s"}\n' \
  "$FROM" "$TO" "$TIMESTAMP" "$ESCAPED_BODY" > "$TMP"

# Atomic rename — prevents watcher from reading a partial file
mv "$TMP" "$FINAL"

echo "[team-send] $FROM → $TO: $BODY"
