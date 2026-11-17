#!/bin/bash
# The browser viewer draws what desvis draws: for the ten-minute recording that
# ships with the viewer's tests, the rows the viewer would upload are compared
# byte for byte with the rows desvis uploads for the same instants.
#
# desvis's side is its own `--dump-render` of that recording. It is generated
# output, so it is made here rather than checked in, and the comparison
# (`studio/viewer/tests/dumped_rows.rs`) is run against it.
#
# usage: viewer_rows.sh [<dump-dir>]      (default: target/worldview-harness/viewer-rows)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
recording="$repo/studio/viewer/tests/fixtures/minimal_easyloop_flow/recording.worldview"
dump="${1:-$repo/target/worldview-harness/viewer-rows}"
# Instants inside slices rather than on their edges, and the run's last one.
ticks="123456,307777,555555,600000"

cd "$repo"
cargo build --release -p desvis
rm -rf "$dump"
mkdir -p "$dump"
"$repo/target/release/desvis" --recording "$recording" --dump-render "$ticks" "$dump" > /dev/null
CRUCIBLE_VIEWER_FIXTURE_DUMP="$dump" cargo test -p crucible-viewer --test dumped_rows -- --ignored
