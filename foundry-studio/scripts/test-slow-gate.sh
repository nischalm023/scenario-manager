#!/usr/bin/env bash
# The slow gate: the whole-fixture simulations under native-modules' tests,
# span parity's DFW6 slice and the scale benchmarks, un-ignored. Those tests
# carry `#[cfg_attr(not(feature = "slow-gate"), ignore)]` or read the feature,
# so the fast gate (`cargo nextest run`) skips them, and this run, which the
# nightly workflow calls, proves them. Every gated test lives in dessim or
# native-modules and the other crates' tests are the fast gate's, so the slow
# gate names those two alone. That also keeps desvis off the nightly's Linux
# runner: it declares eframe without a Linux window backend, and winit refuses
# to compile there.
# Extra arguments go to nextest.
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo nextest run -p dessim -p native-modules --features dessim/slow-gate,native-modules/slow-gate "$@"
