#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

ensure_queue_layout
echo "initialized queue bus under $QUEUE_ROOT"
