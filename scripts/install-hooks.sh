#!/usr/bin/env bash
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
install -m 755 "$root/scripts/pre-push" "$root/.git/hooks/pre-push"
echo "installed pre-push hook"
