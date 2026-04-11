#!/usr/bin/env bash
# =============================================================================
# docker-test.sh — build and verify the Docker Compose stack end-to-end
#
# This script:
#   1. Builds the Docker image
#   2. Starts Postgres + Cairn
#   3. Waits for healthy status
#   4. Runs health checks
#   5. Tears down
#
# Usage:
#   ./scripts/docker-test.sh
# =============================================================================

set -euo pipefail

GRN='\033[0;32m'; RED='\033[0;31m'; CYN='\033[0;36m'; BLD='\033[1m'; RST='\033[0m'
PASS=0; FAIL=0

ok()   { echo -e "${GRN}  ✓${RST} $1"; PASS=$(( PASS + 1 )); }
fail() { echo -e "${RED}  ✗${RST} $1"; FAIL=$(( FAIL + 1 )); }

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

TOKEN="${CAIRN_ADMIN_TOKEN:-dev-admin-token}"
BASE="http://localhost:3000"

cleanup() {
  echo -e "\n${BLD}Cleaning up...${RST}"
  docker compose down -v --remove-orphans 2>/dev/null || true
}
trap cleanup EXIT

# ── Step 1: Build ────────────────────────────────────────────────────────────
echo -e "${BLD}Step 1: Building Docker image...${RST}"
if docker compose build --no-cache 2>&1 | tail -5; then
  ok "Docker build succeeded"
else
  fail "Docker build failed"
  exit 1
fi

# ── Step 2: Start ────────────────────────────────────────────────────────────
echo -e "\n${BLD}Step 2: Starting services...${RST}"
docker compose up -d

# ── Step 3: Wait for healthy ─────────────────────────────────────────────────
echo -e "\n${BLD}Step 3: Waiting for services to become healthy...${RST}"
MAX_WAIT=120
ELAPSED=0
while [ "$ELAPSED" -lt "$MAX_WAIT" ]; do
  PG_HEALTH=$(docker compose ps postgres --format '{{.Health}}' 2>/dev/null || echo "starting")
  CAIRN_HEALTH=$(docker compose ps cairn --format '{{.Health}}' 2>/dev/null || echo "starting")
  if [ "$PG_HEALTH" = "healthy" ] && [ "$CAIRN_HEALTH" = "healthy" ]; then
    ok "All services healthy after ${ELAPSED}s"
    break
  fi
  sleep 2
  ELAPSED=$(( ELAPSED + 2 ))
done

if [ "$ELAPSED" -ge "$MAX_WAIT" ]; then
  fail "Services did not become healthy within ${MAX_WAIT}s"
  echo "  Postgres: $PG_HEALTH"
  echo "  Cairn: $CAIRN_HEALTH"
  docker compose logs --tail=30
  exit 1
fi

# ── Step 4: Health checks ────────────────────────────────────────────────────
echo -e "\n${BLD}Step 4: Running health checks...${RST}"

# /health
HTTP=$(curl -s -o /dev/null -w "%{http_code}" --max-time 5 "$BASE/health" 2>/dev/null || echo "000")
[ "$HTTP" = "200" ] && ok "GET /health ($HTTP)" || fail "GET /health ($HTTP)"

# /v1/status
HTTP=$(curl -s -o /dev/null -w "%{http_code}" --max-time 5 \
  -H "Authorization: Bearer $TOKEN" "$BASE/v1/status" 2>/dev/null || echo "000")
[ "$HTTP" = "200" ] && ok "GET /v1/status ($HTTP)" || fail "GET /v1/status ($HTTP)"

# /v1/settings — verify Postgres backend
BODY=$(curl -s --max-time 5 -H "Authorization: Bearer $TOKEN" "$BASE/v1/settings" 2>/dev/null || echo "{}")
BACKEND=$(echo "$BODY" | python3 -c "import sys,json;print(json.load(sys.stdin).get('store_backend','?'))" 2>/dev/null || echo "?")
[ "$BACKEND" = "postgres" ] && ok "Store backend: postgres" || fail "Expected postgres, got: $BACKEND"

# /v1/dashboard
HTTP=$(curl -s -o /dev/null -w "%{http_code}" --max-time 5 \
  -H "Authorization: Bearer $TOKEN" "$BASE/v1/dashboard?tenant_id=default_tenant&workspace_id=default_workspace&project_id=default_project" 2>/dev/null || echo "000")
[ "$HTTP" = "200" ] && ok "GET /v1/dashboard ($HTTP)" || fail "GET /v1/dashboard ($HTTP)"

# UI serves (static asset)
HTTP=$(curl -s -o /dev/null -w "%{http_code}" --max-time 5 "$BASE/" 2>/dev/null || echo "000")
[ "$HTTP" = "200" ] && ok "GET / (UI) ($HTTP)" || fail "GET / (UI) ($HTTP)"

# Create a session (write path)
HTTP=$(curl -s -o /dev/null -w "%{http_code}" --max-time 5 -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"tenant_id":"default_tenant","workspace_id":"default_workspace","project_id":"default_project","session_id":"docker-test-sess"}' \
  "$BASE/v1/sessions" 2>/dev/null || echo "000")
[[ "$HTTP" =~ ^(200|201|409)$ ]] && ok "POST /v1/sessions ($HTTP)" || fail "POST /v1/sessions ($HTTP)"

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
TOTAL=$(( PASS + FAIL ))
printf "  ${GRN}Passed${RST}  %d\n" "$PASS"
printf "  ${RED}Failed${RST}  %d\n" "$FAIL"
printf "  Total   %d\n" "$TOTAL"

[ "$FAIL" -eq 0 ] && echo -e "\n${GRN}${BLD}Docker E2E: all checks passed.${RST}" || echo -e "\n${RED}${BLD}Docker E2E: some checks failed.${RST}"
exit "$FAIL"
