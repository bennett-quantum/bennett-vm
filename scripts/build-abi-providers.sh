#!/usr/bin/env bash
# Separately selectable C providers. Never share a linker search path between families.
set -euo pipefail
unset DISPLAY WAYLAND_DISPLAY DBUS_SESSION_BUS_ADDRESS BROWSER
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="${1:-$root/.tmp/abi-providers}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
export TMPDIR="$root/.tmp/abi-providers/tmp"
mkdir -p "$TMPDIR"
cargo build --offline -j3 --lib --no-default-features --manifest-path "$root/Cargo.toml" --target-dir "$out/0.8"
cargo build --offline -j3 --lib --no-default-features --features abi-0-9 --manifest-path "$root/Cargo.toml" --target-dir "$out/0.9"
