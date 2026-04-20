#!/usr/bin/env bash
# =============================================================================
# run-fabric-integration-tests.sh — live-Valkey integration runner for cairn-fabric
#
# Boots a local Valkey 8 container, builds the FlowFabric Lua library,
# preloads it via FUNCTION LOAD REPLACE, then runs the cairn-fabric
# integration suite against the live instance.
#
# Usage:
#   ./scripts/run-fabric-integration-tests.sh                # normal run
#   ./scripts/run-fabric-integration-tests.sh --ci           # fail-fast, quiet
#   ./scripts/run-fabric-integration-tests.sh --keep-valkey  # leave container up
#
# Environment overrides:
#   CAIRN_TEST_VALKEY_URL   test connection URL (default: redis://localhost:6379)
#   VALKEY_IMAGE            docker image (default: valkey/valkey:8-alpine)
#   VALKEY_PORT             host port (default: 6379)
#   VALKEY_CONTAINER        container name (default: cairn-fabric-integ-valkey)
#   FF_PATH                 FlowFabric checkout path (default: /tmp/FlowFabric)
#   FF_BRANCH               FlowFabric branch (default: main)
#   FF_REPO                 clone URL if FF_PATH missing (default: https://github.com/avifenesh/FlowFabric.git)
#   FF_REV                  FlowFabric SHA for the Lua bundle loaded into Valkey.
#                           Must match the crates.io release used in
#                           crates/cairn-fabric/Cargo.toml (`ff-* = "0.1"` →
#                           FF v0.1.1 tag). Default tracks that tag.
#
# Exit codes: 0 = tests passed, non-zero = setup or test failure.
# =============================================================================

set -euo pipefail

# ── Flags ────────────────────────────────────────────────────────────────────
CI_MODE=0
KEEP_VALKEY=0
for arg in "$@"; do
  case "$arg" in
    --ci)          CI_MODE=1 ;;
    --keep-valkey) KEEP_VALKEY=1 ;;
    -h|--help)
      sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unknown arg: $arg" >&2
      exit 2
      ;;
  esac
done

# ── Configuration ────────────────────────────────────────────────────────────
VALKEY_IMAGE="${VALKEY_IMAGE:-valkey/valkey:8-alpine}"
VALKEY_PORT="${VALKEY_PORT:-6379}"
VALKEY_CONTAINER="${VALKEY_CONTAINER:-cairn-fabric-integ-valkey}"
FF_PATH="${FF_PATH:-/tmp/FlowFabric}"
FF_BRANCH="${FF_BRANCH:-main}"
FF_REPO="${FF_REPO:-https://github.com/avifenesh/FlowFabric.git}"
# Checked out to load FF's Lua functions into the test Valkey. The
# Rust crates in crates/cairn-fabric/Cargo.toml are now pinned to
# crates.io (`ff-core = "0.1"` etc.), so this ref only controls
# which Lua version lives in the test instance. Track the published
# v0.1.1 tag so the Lua matches the Rust.
FF_REV="${FF_REV:-00608ef62c0bdd1c4f6fed6b41be93f4d64af41a}"  # v0.1.1
TEST_URL="${CAIRN_TEST_VALKEY_URL:-redis://localhost:${VALKEY_PORT}}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# ── Output helpers ───────────────────────────────────────────────────────────
if [ -t 1 ] && [ "$CI_MODE" -eq 0 ]; then
  GRN='\033[0;32m'; RED='\033[0;31m'; YLW='\033[0;33m'; CYN='\033[0;36m'; BLD='\033[1m'; RST='\033[0m'
else
  GRN=''; RED=''; YLW=''; CYN=''; BLD=''; RST=''
fi

say()  { printf '%b==>%b %s\n' "$CYN" "$RST" "$1"; }
ok()   { printf '%b  ok%b %s\n' "$GRN" "$RST" "$1"; }
warn() { printf '%b warn%b %s\n' "$YLW" "$RST" "$1"; }
die()  { printf '%b fail%b %s\n' "$RED" "$RST" "$1" >&2; exit 1; }

# ── Cleanup trap ─────────────────────────────────────────────────────────────
STARTED_CONTAINER=0
cleanup() {
  local ec=$?
  if [ "$KEEP_VALKEY" -eq 1 ]; then
    warn "--keep-valkey set; leaving container ${VALKEY_CONTAINER} running"
  elif [ "$STARTED_CONTAINER" -eq 1 ]; then
    say "stopping valkey container"
    docker rm -f "$VALKEY_CONTAINER" >/dev/null 2>&1 || true
  fi
  exit "$ec"
}
trap cleanup EXIT INT TERM

# ── Pre-flight ───────────────────────────────────────────────────────────────
say "checking prerequisites"
command -v docker >/dev/null 2>&1 || die "docker not found on PATH"
command -v cargo  >/dev/null 2>&1 || die "cargo not found on PATH"
command -v git    >/dev/null 2>&1 || die "git not found on PATH"
if ! docker info >/dev/null 2>&1; then
  die "docker daemon not reachable — is Docker Desktop running?"
fi
ok "docker, cargo, git available"

# ── FlowFabric checkout ──────────────────────────────────────────────────────
say "ensuring FlowFabric checkout at ${FF_PATH} pinned to ${FF_REV:0:10}"
if [ ! -d "$FF_PATH/.git" ]; then
  say "cloning ${FF_REPO} (branch ${FF_BRANCH}) to ${FF_PATH}"
  git clone --branch "$FF_BRANCH" "$FF_REPO" "$FF_PATH"
  ok "cloned FlowFabric"
fi
say "checking out pinned rev ${FF_REV:0:10}"
git -C "$FF_PATH" fetch --quiet origin "$FF_BRANCH"
git -C "$FF_PATH" checkout --quiet --detach "$FF_REV" || die "could not check out FF_REV=${FF_REV} in ${FF_PATH} (try: rm -rf ${FF_PATH})"
ok "FF at ${FF_REV:0:10}"

# ── Valkey container (idempotent) ────────────────────────────────────────────
say "starting valkey (${VALKEY_IMAGE})"
# docker 29 prints a stray stdout line on missing containers before erroring,
# so use `ps -a` lookup instead — stable across versions.
if docker ps -a --format '{{.Names}}' | grep -qx "$VALKEY_CONTAINER"; then
  existing_state="$(docker inspect -f '{{.State.Status}}' "$VALKEY_CONTAINER" 2>/dev/null)"
else
  existing_state="absent"
fi
case "$existing_state" in
  running)
    ok "reusing running container ${VALKEY_CONTAINER}"
    ;;
  exited|created)
    say "starting existing container ${VALKEY_CONTAINER}"
    docker start "$VALKEY_CONTAINER" >/dev/null
    STARTED_CONTAINER=1
    ok "container started"
    ;;
  absent)
    say "creating container ${VALKEY_CONTAINER} on port ${VALKEY_PORT}"
    docker run -d \
      --name "$VALKEY_CONTAINER" \
      -p "${VALKEY_PORT}:6379" \
      "$VALKEY_IMAGE" >/dev/null
    STARTED_CONTAINER=1
    ok "container created"
    ;;
  *)
    die "container ${VALKEY_CONTAINER} in unexpected state: ${existing_state}"
    ;;
esac

# Wait for PING
say "waiting for valkey to accept connections"
attempts=0
max_attempts=30
until docker exec "$VALKEY_CONTAINER" valkey-cli ping >/dev/null 2>&1; do
  attempts=$((attempts + 1))
  if [ "$attempts" -ge "$max_attempts" ]; then
    docker logs --tail 50 "$VALKEY_CONTAINER" >&2 || true
    die "valkey did not respond to PING after ${max_attempts} attempts"
  fi
  sleep 1
done
ok "valkey PONG received"

# ── Build FlowFabric Lua library ─────────────────────────────────────────────
say "building ff-script (generates bundled flowfabric.lua)"
(
  cd "$FF_PATH"
  if [ "$CI_MODE" -eq 1 ]; then
    cargo build --release -p ff-script --quiet
  else
    cargo build --release -p ff-script
  fi
)
ok "ff-script built"

# Find the generated lua bundle in OUT_DIR (target/release/build/ff-script-*/out/flowfabric.lua)
LUA_BUNDLE="$(find "$FF_PATH/target/release/build" -maxdepth 3 -type f -name 'flowfabric.lua' 2>/dev/null | head -n 1)"
if [ -z "$LUA_BUNDLE" ] || [ ! -f "$LUA_BUNDLE" ]; then
  die "could not locate generated flowfabric.lua under ${FF_PATH}/target/release/build"
fi
ok "bundle at ${LUA_BUNDLE}"

# ── FUNCTION LOAD REPLACE ────────────────────────────────────────────────────
say "loading flowfabric library into valkey (FUNCTION LOAD REPLACE)"
# Copy the bundle into the container to sidestep quoting / size limits on the CLI.
docker cp "$LUA_BUNDLE" "$VALKEY_CONTAINER:/tmp/flowfabric.lua" >/dev/null
load_out="$(docker exec "$VALKEY_CONTAINER" sh -c 'valkey-cli -x FUNCTION LOAD REPLACE < /tmp/flowfabric.lua' 2>&1)" || {
  printf '%s\n' "$load_out" >&2
  die "FUNCTION LOAD failed — usually means the lua bundle is malformed or a stale lib blocks load; try 'docker exec ${VALKEY_CONTAINER} valkey-cli FUNCTION FLUSH'"
}
ok "library loaded: ${load_out}"

# Verify by calling ff_version (the library exposes it)
ff_ver="$(docker exec "$VALKEY_CONTAINER" valkey-cli FCALL ff_version 0 2>&1 || true)"
if [ -z "$ff_ver" ]; then
  die "ff_version FCALL returned empty — library did not register properly"
fi
ok "ff_version = ${ff_ver}"

# ── Run cairn-fabric integration suite ───────────────────────────────────────
say "running cairn-fabric integration tests"
export CAIRN_TEST_VALKEY_URL="$TEST_URL"
printf '    CAIRN_TEST_VALKEY_URL=%s\n' "$TEST_URL"

test_flags="--ignored --nocapture"
if [ "$CI_MODE" -eq 1 ]; then
  test_flags="--ignored"
fi

set +e
(
  cd "$REPO_ROOT"
  # shellcheck disable=SC2086
  cargo test -p cairn-fabric --test integration -- $test_flags
)
test_ec=$?
set -e

if [ "$test_ec" -eq 0 ]; then
  ok "integration tests passed"
else
  warn "integration tests exited with code ${test_ec}"
  if [ "$CI_MODE" -eq 1 ]; then
    exit "$test_ec"
  fi
fi

say "done"
exit "$test_ec"
