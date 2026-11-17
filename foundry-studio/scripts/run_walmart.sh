#!/usr/bin/env bash
# Launch the complete delivered Walmart order file. Pass explicit desvis
# flags, such as --headless or --simtime 20h, for an internal bounded run.
set -euo pipefail

walmart_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
exec cargo run --release --manifest-path "$walmart_repo_dir/Cargo.toml" -p desvis -- \
  "$walmart_repo_dir/sim/dessim/tests/fixtures/walmart_dfw6_spec.json" --run "$@"
