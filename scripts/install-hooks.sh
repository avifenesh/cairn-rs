#!/usr/bin/env bash
# Install git hooks for cairn-rs development.
# Run once after cloning: ./scripts/install-hooks.sh

set -euo pipefail
HOOK_DIR="$(git rev-parse --show-toplevel)/.git/hooks"

cp scripts/hooks/pre-push "$HOOK_DIR/pre-push"
chmod +x "$HOOK_DIR/pre-push"

echo "✓ Installed pre-push hook"
echo "  Runs: cargo fmt, cargo clippy (Rust) + tsc, build (frontend)"
echo "  Only checks what changed. Skip with: git push --no-verify"
