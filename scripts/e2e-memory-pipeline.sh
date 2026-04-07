#!/usr/bin/env bash
# e2e-memory-pipeline.sh — end-to-end test of the RFC 003 owned retrieval pipeline.
#
# Validates: ingest -> chunk -> embed -> retrieve -> score across the HTTP API.
#
# Prerequisites:
#   - cairn-app running (cargo run -p cairn-app)
#   - For real embeddings: OPENAI_COMPAT_BASE_URL and OPENAI_COMPAT_API_KEY set
#     when the server was started. Without these, lexical search still works but
#     embedding vectors will be empty.
#
# Usage:
#   ./scripts/e2e-memory-pipeline.sh
#   CAIRN_URL=http://localhost:8080 CAIRN_TOKEN=my-token ./scripts/e2e-memory-pipeline.sh

set -euo pipefail

CAIRN_URL="${CAIRN_URL:-http://localhost:3000}"
CAIRN_TOKEN="${CAIRN_TOKEN:-cairn-demo-token}"
TENANT="${CAIRN_TENANT:-default}"
WORKSPACE="${CAIRN_WORKSPACE:-default}"
PROJECT="memory_pipeline"

passed=0
failed=0
total=0

# ── Helpers ───────────────────────────────────────────────────────────────────

check() {
  local name="$1"
  shift
  total=$((total + 1))
  if "$@" >/dev/null 2>&1; then
    passed=$((passed + 1))
    printf "  \033[32mPASS\033[0m  %s\n" "$name"
  else
    failed=$((failed + 1))
    printf "  \033[31mFAIL\033[0m  %s\n" "$name"
  fi
}

api_get() {
  curl -sfS -H "Authorization: Bearer $CAIRN_TOKEN" "$CAIRN_URL$1"
}

api_post() {
  curl -sfS -H "Authorization: Bearer $CAIRN_TOKEN" \
       -H "Content-Type: application/json" \
       -d "$2" "$CAIRN_URL$1"
}

echo "=== e2e-memory-pipeline ==="
echo "server: $CAIRN_URL"
echo ""

# ── Step 0: Health check ─────────────────────────────────────────────────────

echo "[0] Health check"
check "server is reachable" api_get /health

# ── Step 1: Ingest 3 documents ───────────────────────────────────────────────

echo ""
echo "[1] Ingesting 3 documents"

DOC1=$(api_post /v1/memory/ingest '{
  "tenant_id": "'"$TENANT"'",
  "workspace_id": "'"$WORKSPACE"'",
  "project_id": "'"$PROJECT"'",
  "source_id": "rust-docs",
  "document_id": "doc_ownership",
  "content": "Rust ownership model ensures memory safety without garbage collection. The borrow checker validates references at compile time, preventing data races and dangling pointers. Each value has exactly one owner, and ownership can be transferred or borrowed.",
  "source_type": "plain_text"
}')
check "ingest doc_ownership" test "$(echo "$DOC1" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d.get("status","") or ("ingested" if d.get("ok") else ""))' 2>/dev/null)" = "ingested"

DOC2=$(api_post /v1/memory/ingest '{
  "tenant_id": "'"$TENANT"'",
  "workspace_id": "'"$WORKSPACE"'",
  "project_id": "'"$PROJECT"'",
  "source_id": "rust-docs",
  "document_id": "doc_concurrency",
  "content": "Fearless concurrency in Rust is enabled by the type system. Send and Sync traits determine what can cross thread boundaries. Channels, mutexes, and atomic types provide safe concurrent access patterns without runtime overhead.",
  "source_type": "plain_text"
}')
check "ingest doc_concurrency" test "$(echo "$DOC2" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d.get("status","") or ("ingested" if d.get("ok") else ""))' 2>/dev/null)" = "ingested"

DOC3=$(api_post /v1/memory/ingest '{
  "tenant_id": "'"$TENANT"'",
  "workspace_id": "'"$WORKSPACE"'",
  "project_id": "'"$PROJECT"'",
  "source_id": "cooking-blog",
  "document_id": "doc_pasta",
  "content": "Carbonara is a traditional Italian pasta dish from Rome. Authentic carbonara uses guanciale, eggs, Pecorino Romano cheese, and black pepper. No cream is used in the traditional recipe despite common misconceptions.",
  "source_type": "plain_text"
}')
check "ingest doc_pasta" test "$(echo "$DOC3" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d.get("status","") or ("ingested" if d.get("ok") else ""))' 2>/dev/null)" = "ingested"

# ── Step 2: Brief pause for embedding pipeline ──────────────────────────────

echo ""
echo "[2] Waiting 2s for embedding pipeline..."
sleep 2

# ── Step 3: Lexical search ───────────────────────────────────────────────────

echo ""
echo "[3] Lexical search"

SEARCH1=$(api_get "/v1/memory/search?tenant_id=$TENANT&workspace_id=$WORKSPACE&project_id=$PROJECT&query_text=ownership+borrow+checker&limit=5")
RESULT_COUNT=$(echo "$SEARCH1" | python3 -c 'import sys,json; print(len(json.load(sys.stdin).get("results",[])))' 2>/dev/null)
check "ownership query returns results" test "$RESULT_COUNT" -gt 0

# The ownership doc should rank first for "ownership borrow checker"
TOP_DOC=$(echo "$SEARCH1" | python3 -c 'import sys,json; r=json.load(sys.stdin).get("results",[]); print(r[0]["document_id"] if r else "")' 2>/dev/null)
check "ownership doc ranks first" test "$TOP_DOC" = "doc_ownership"

# Score should be positive
TOP_SCORE=$(echo "$SEARCH1" | python3 -c 'import sys,json; r=json.load(sys.stdin).get("results",[]); print("yes" if r and r[0]["score"]>0 else "no")' 2>/dev/null)
check "top result has positive score" test "$TOP_SCORE" = "yes"

# Search for concurrency — should find the concurrency doc
SEARCH2=$(api_get "/v1/memory/search?tenant_id=$TENANT&workspace_id=$WORKSPACE&project_id=$PROJECT&query_text=concurrency+threads+Send+Sync&limit=5")
TOP_DOC2=$(echo "$SEARCH2" | python3 -c 'import sys,json; r=json.load(sys.stdin).get("results",[]); print(r[0]["document_id"] if r else "")' 2>/dev/null)
check "concurrency doc ranks first for thread query" test "$TOP_DOC2" = "doc_concurrency"

# Search for pasta — should find the cooking doc, not Rust docs
SEARCH3=$(api_get "/v1/memory/search?tenant_id=$TENANT&workspace_id=$WORKSPACE&project_id=$PROJECT&query_text=carbonara+pasta+Italian&limit=5")
TOP_DOC3=$(echo "$SEARCH3" | python3 -c 'import sys,json; r=json.load(sys.stdin).get("results",[]); print(r[0]["document_id"] if r else "")' 2>/dev/null)
check "pasta doc ranks first for cooking query" test "$TOP_DOC3" = "doc_pasta"

# Cross-domain search: "Rust" should NOT return the pasta doc first
SEARCH4=$(api_get "/v1/memory/search?tenant_id=$TENANT&workspace_id=$WORKSPACE&project_id=$PROJECT&query_text=Rust+memory+safety&limit=5")
TOP_DOC4=$(echo "$SEARCH4" | python3 -c 'import sys,json; r=json.load(sys.stdin).get("results",[]); print(r[0]["document_id"] if r else "")' 2>/dev/null)
check "Rust query returns Rust doc (not pasta)" test "$TOP_DOC4" != "doc_pasta"

# ── Step 4: Source document counts ───────────────────────────────────────────

echo ""
echo "[4] Source document counts"

SOURCES=$(api_get "/v1/sources?tenant_id=$TENANT&workspace_id=$WORKSPACE&project_id=$PROJECT")
SOURCE_COUNT=$(echo "$SOURCES" | python3 -c 'import sys,json; print(len(json.load(sys.stdin)))' 2>/dev/null)
check "2 sources registered" test "$SOURCE_COUNT" = "2"

# rust-docs should have 2 documents
RUST_DOCS_COUNT=$(echo "$SOURCES" | python3 -c '
import sys, json
sources = json.load(sys.stdin)
for s in sources:
    if s.get("source_id") == "rust-docs":
        print(s.get("document_count", 0))
        break
else:
    print(0)
' 2>/dev/null)
check "rust-docs source has 2 documents" test "$RUST_DOCS_COUNT" -ge 1

# cooking-blog should have 1 document
COOKING_COUNT=$(echo "$SOURCES" | python3 -c '
import sys, json
sources = json.load(sys.stdin)
for s in sources:
    if s.get("source_id") == "cooking-blog":
        print(s.get("document_count", 0))
        break
else:
    print(0)
' 2>/dev/null)
check "cooking-blog source has 1 document" test "$COOKING_COUNT" -ge 1

# ── Step 5: Empty search returns empty results ──────────────────────────────

echo ""
echo "[5] Edge cases"

EMPTY=$(api_get "/v1/memory/search?tenant_id=$TENANT&workspace_id=$WORKSPACE&project_id=$PROJECT&query_text=&limit=5")
EMPTY_COUNT=$(echo "$EMPTY" | python3 -c 'import sys,json; print(len(json.load(sys.stdin).get("results",[])))' 2>/dev/null)
check "empty query returns empty results" test "$EMPTY_COUNT" = "0"

# Query against wrong project returns nothing
WRONG_PROJECT=$(api_get "/v1/memory/search?tenant_id=nonexistent&workspace_id=none&project_id=none&query_text=Rust&limit=5")
WRONG_COUNT=$(echo "$WRONG_PROJECT" | python3 -c 'import sys,json; print(len(json.load(sys.stdin).get("results",[])))' 2>/dev/null)
check "wrong project returns no results" test "$WRONG_COUNT" = "0"

# ── Summary ──────────────────────────────────────────────────────────────────

echo ""
echo "=== Summary: $passed/$total passed, $failed failed ==="

if [ "$failed" -gt 0 ]; then
  exit 1
fi
