#!/usr/bin/env bash
# The browser decodes .worldview frames with this crate, so it has to build
# for wasm32 — which the simulator cannot, and which nothing else in the
# workspace checks. Run this before changing worldview's dependencies.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

if ! rustup target list --installed | grep -qx wasm32-unknown-unknown; then
    echo "wasm32-unknown-unknown is not installed: rustup target add wasm32-unknown-unknown" >&2
    exit 1
fi

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"
cargo check -p worldview --target wasm32-unknown-unknown --no-default-features
echo "worldview builds for wasm32-unknown-unknown"
