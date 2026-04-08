#!/usr/bin/env bash
# =============================================================================
# docker-health-check.sh — verify Docker Compose stack is healthy
#
# Usage:
#   ./scripts/docker-health-check.sh                     # default compose
#   ./scripts/docker-health-check.sh postgres             # postgres compose
#   COMPOSE_FILE=docker-compose.postgres.yml ./scripts/docker-health-check.sh
# =============================================================================

set -euo pipefail

VARIANT="${1:-default}"
case "$VARIANT" in
  postgres|pg) COMPOSE_FILE="docker-compose.postgres.yml" ;;
  *)           COMPOSE_FILE="docker-compose.yml" ;;
esac

GRN='\033[0;32m'; RED='\033[0;31m'; CYN='\033[0;36m'; BLD='\033[1m'; RST='\033[0m'
PASS=0; FAIL=0

ok()   { echo -e "${GRN}  ✓${RST} $1"; PASS=$(( PASS + 1 )); }
fail() { echo -e "${RED}  ✗${RST} $1"; FAIL=$(( FAIL + 1 )); }

echo -e "${BLD}Docker health check (${CYN}${COMPOSE_FILE}${RST}${BLD})${RST}"

# ── Check services are running ───────────────────────────────────────────────

echo -e "\n${BLD}Services:${RST}"

for svc in $(docker compose -f "$COMPOSE_FILE" ps --services 2>/dev/null); do
  STATUS=$(docker compose -f "$COMPOSE_FILE" ps "$svc" --format '{{.Health}}' 2>/dev/null || echo "unknown")
  if [ "$STATUS" = "healthy" ]; then
    ok "$svc: healthy"
  else
    fail "$svc: $STATUS"
  fi
done

# ── Check cairn API ──────────────────────────────────────────────────────────

echo -e "\n${BLD}Cairn API:${RST}"

TOKEN="${CAIRN_ADMIN_TOKEN:-dev-admin-token}"
BASE="http://localhost:3000"

# Health endpoint
HTTP=$(curl -s -o /dev/null -w "%{http_code}" --max-time 5 "$BASE/health" 2>/dev/null || echo "000")
[ "$HTTP" = "200" ] && ok "GET /health ($HTTP)" || fail "GET /health ($HTTP)"

# Dashboard
HTTP=$(curl -s -o /dev/null -w "%{http_code}" --max-time 5 \
  -H "Authorization: Bearer $TOKEN" "$BASE/v1/dashboard?tenant_id=default&workspace_id=default&project_id=default" 2>/dev/null || echo "000")
[ "$HTTP" = "200" ] && ok "GET /v1/dashboard ($HTTP)" || fail "GET /v1/dashboard ($HTTP)"

# Status
HTTP=$(curl -s -o /dev/null -w "%{http_code}" --max-time 5 \
  -H "Authorization: Bearer $TOKEN" "$BASE/v1/status" 2>/dev/null || echo "000")
[ "$HTTP" = "200" ] && ok "GET /v1/status ($HTTP)" || fail "GET /v1/status ($HTTP)"

# Settings (check store backend)
BODY=$(curl -s --max-time 5 -H "Authorization: Bearer $TOKEN" "$BASE/v1/settings" 2>/dev/null || echo "{}")
BACKEND=$(echo "$BODY" | python3 -c "import sys,json;print(json.load(sys.stdin).get('store_backend','?'))" 2>/dev/null || echo "?")
MODE=$(echo "$BODY" | python3 -c "import sys,json;print(json.load(sys.stdin).get('deployment_mode','?'))" 2>/dev/null || echo "?")
echo -e "  Store backend: ${CYN}${BACKEND}${RST}"
echo -e "  Mode: ${CYN}${MODE}${RST}"

if [ "$VARIANT" = "postgres" ] || [ "$VARIANT" = "pg" ]; then
  [ "$BACKEND" = "postgres" ] && ok "backend is postgres" || fail "expected postgres, got $BACKEND"
  [ "$MODE" = "self_hosted_team" ] && ok "mode is team" || fail "expected team mode, got $MODE"
fi

# ── Postgres connectivity (if applicable) ────────────────────────────────────

if [ "$VARIANT" = "postgres" ] || [ "$VARIANT" = "pg" ]; then
  echo -e "\n${BLD}Postgres:${RST}"
  HTTP=$(curl -s -o /dev/null -w "%{http_code}" --max-time 5 \
    -H "Authorization: Bearer $TOKEN" "$BASE/v1/admin/db-status" 2>/dev/null || echo "000")
  [[ "$HTTP" =~ ^(200|404)$ ]] && ok "GET /v1/admin/db-status ($HTTP)" || fail "GET /v1/admin/db-status ($HTTP)"
fi

# ── Summary ──────────────────────────────────────────────────────────────────

echo ""
TOTAL=$(( PASS + FAIL ))
printf "  ${GRN}Passed${RST}  %d\n" "$PASS"
printf "  ${RED}Failed${RST}  %d\n" "$FAIL"
printf "  Total   %d\n" "$TOTAL"

[ "$FAIL" -eq 0 ] && echo -e "\n${GRN}${BLD}All checks passed.${RST}" || echo -e "\n${RED}${BLD}Some checks failed.${RST}"
exit "$FAIL"
