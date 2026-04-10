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

# Build JSON via python3's json.dumps — handles ALL escapes required by RFC 8259
# (quotes, backslashes, line-feeds, carriage-returns, tabs, other control chars,
# and unicode). The earlier printf+manual-escape approach only handled \ and ",
# so multi-line bodies produced invalid JSON that the watcher silently dropped.
python3 -c '
import json, sys
msg = {
    "from": sys.argv[1],
    "to":   sys.argv[2],
    "ts":   sys.argv[3],
    "body": sys.argv[4],
}
with open(sys.argv[5], "w", encoding="utf-8") as f:
    json.dump(msg, f, ensure_ascii=False)
' "$FROM" "$TO" "$TIMESTAMP" "$BODY" "$TMP"

# Atomic rename — prevents watcher from reading a partial file
mv "$TMP" "$FINAL"

# Log confirmation with a one-line preview of the body so long messages
# don't flood the sender's terminal.
PREVIEW="${BODY%%$'\n'*}"
echo "[team-send] $FROM → $TO: $PREVIEW"
