# `traffic_report`: export a recording's measurements to JSON

`traffic_report` is a Rust binary in the `native-modules` crate. It reads a closed
`.worldview` recording and writes cumulative HTM traffic measurements, with
optional throughput, waiting and inventory time series. It does not run the
simulation or open a window.

- Entry point: [`../traffic_report.rs`](../traffic_report.rs).
- Optional time-series extraction: [`series.rs`](series.rs).
- Shared navigational-wait filter:
  [`WarehouseMap::navigation_wait_exclusions`](../../../../sim/desmap/src/map/navigation_wait.rs).
- HTML generation and batch workflow:
  [Walmart reporting utilities](../../../../scripts/walmart_matrix/README.md).

## Build and invoke

From the repository root:

```sh
cargo build -p native-modules --release --bin traffic_report
```

The default binary location is `target/release/traffic_report`.
Export the first 16 simulated hours, including five-minute time-series bins:

```sh
target/release/traffic_report \
  /path/to/run.worldview \
  /path/to/spec.json \
  16 \
  /path/to/measurements.json \
  --series-minutes 5
```

Alternatively, Cargo can build and invoke it in one command:

```sh
cargo run -p native-modules --release --bin traffic_report -- \
  /path/to/run.worldview /path/to/spec.json 16 /path/to/measurements.json \
  --series-minutes 5
```

The positional arguments, in order, are:

| Argument | Meaning |
| --- | --- |
| `recording` | A closed `.worldview` file, whose motion section carries the canonical HTM moves. |
| `spec` | The matching simulation spec. Its referenced world/map and other required input paths must resolve. The map supplies cell geometry, equipment membership and source-inventory zones. |
| `until-hours` | A positive number of simulated hours from T0, such as `16` or `0.25`; do not append `h`. The effective cutoff is the earlier of this value and the recorded end time. |
| `output.json` | Destination JSON file; its parent directory must already exist. |
| `--series-minutes N` | Optional positive whole-minute bin width. Omit this pair for cumulative traffic only. |

Use the map that produced the recording. Matching cell IDs and operational roles
are required; substituting a revised map can mislabel recorded cells. The reader
rebuilds the run model from the recording and folds its slices in order. Series
mode also retains the relevant completion journals,
aisle snapshots and per-bin cell wait aggregates. A short reporting cutoff does
not avoid reading through the rest of the recording.

## Output

The output is a JSON object, not HTML. The top-level fields include:

| Field | Contents / units |
| --- | --- |
| `profile` | Input recording path. |
| `until_ms`, `recorded_end_ms` | Effective measurement cutoff and recording end, in simulated milliseconds. |
| `distance_m` | Total observed HTM travel distance, metres. |
| `all_wait_s` | HTM-seconds of stationary waiting across all cells. |
| `eligible_wait_s` | HTM-seconds of waiting after the shared navigational-cell exclusions. |
| `canonical_moving_ms`, `playback_moving_ms`, `motion_difference_ms`, `missing_motion_slices` | Motion-coverage and duration-reconciliation checks. |
| `cells` | Per-cell geometry, identity, exclusion reason, waiting, implicit holds, rotation, movement, distance and moving-speed distribution. Only cells with observed traffic are included. |
| `floors` | Floor rectangles and kinds, for drawing spatial context. |

Each `cells` row contains `cell`, `entity`, `name`, `kind`, `zone`, `x_mm`, `y_mm`,
`size_mm` and `exclusion`, plus:

- `wait_s`: total stationary HTM wait seconds; `implicit_hold_s` is a subset.
- `rotate_s`, `moving_s`: rotation-in-place and travel durations, separately.
- `distance_m`, `mean_m_s`: distance and moving-only mean speed. The mean is
  `null` when there is no measured movement.
- `speed_seconds`: duration in 11 speed bins: `[0, 0.5)`, `[0.5, 1.0)`, …,
  `[4.5, 5.0)`, `[5.0, infinity)` metres/second. Stationary waits and rotations
  are not included in these moving-speed bins.

With `--series-minutes`, the additional `series` object contains:

| Field | Contents |
| --- | --- |
| `bin_ms`, `htm_fleet_size` | Requested bin width and the recorded HTM fleet count. |
| `bins` | Interval rows with `start_ms`, `end_ms`, raw flow counts and eligible HTM wait durations. |
| `racks` | Source-inventory rack snapshots at T0 and every bin end. |
| `totals` | Sum of eligible `elapsed_wait_ms` and its `implicit_hold_ms` subset. |
| `excluded_cell_ids`, `equipment_cell_ids`, `filter` | Cell membership and the shared filter's name. |

Each `bins` row contains raw counts, **not rates**:

- `picks`: units picked; `pick_transfers`: completed transfer tasks.
- `inducted_units`: units added at decant; `filled_totes`: completed decant fills.
- `stored_totes`: completed VTM rack stores of filled putaway totes.
- `wait.elapsed_wait_ms`: total eligible HTM waiting during the interval;
  `wait.implicit_hold_ms` is the implicit stationary-hold subset.

Each `racks` row contains `at_ms`, `capacity`, `occupied`, `stocked`, `empty`,
`unknown` and `net_inventory_units`. Slot counts cover inventory zones only;
order-container storage and order buffers are excluded. `stocked` counts slots
containing a tote with at least one unit; `empty` counts slots containing an
empty tote. `occupied = stocked + empty + unknown`.

`net_inventory_units` is inducted units minus picked units through that instant.
It does **not** include the initial preload. Add the independently established
initial inventory to obtain absolute source inventory. That unit ledger includes
inventory in circulation as well as in racks; units already picked into order
containers have been removed from it.

## Rates, boundaries and waiting

For a bin, let `hours = (end_ms - start_ms) / 3_600_000`:

```text
pick_units_per_hour = picks / hours
putaway_totes_stored_per_hour = stored_totes / hours
wait_minutes_per_htm_per_hour = wait.elapsed_wait_ms / 60_000 / htm_fleet_size / hours
```

Sum counts and observed durations before calculating a rate across multiple
bins. The final bin can be partial. Completions at the exact observation cutoff
are included in the last bin; future completions are excluded. Other flow-bin
boundaries use `[start, end)`; rack snapshots describe state through their exact
timestamp. Ongoing waits are clipped at the observation cutoff, not discarded.

Navigational waiting excludes typed queue, presentation, exchange, relay and
charging cells; relay access; station presentation and queue cells; declared
handoff queues; inventory holding queues; and complete conveyor-equipment
membership, including all EasyLoop cells. This uses the same map-derived filter
as the desvis waiting heatmap, including waiting cells whose traversal type is
`Open`. An ordinary cell upstream of a queue remains eligible.

This is a location-based measure. It includes task/resource waiting and idle
holds in eligible cells, so it is not a causal measurement of traffic congestion.

## Checks and repeatability

The exporter checks finalization and canonical motion coverage, reconciles
moving durations, and checks series waiting against cumulative eligible waiting.
Treat a nonzero exit status as an invalid export, even if an output file exists.

The [companion generator](../../../../scripts/walmart_matrix/VARIANT_CHARTS.md)
adds independent flow-audit reconciliation, inventory-balance checks, rack
occupancy checks, and SHA-256 cache keys for the profile, spec and exporter.
Preserve the recording, matching spec/world/input files, simulation binary or
source revision, and exporter revision with an experiment. The Rust source and
generators are tracked in Git; the large September 14 recordings and run caches
are retained separately from the repository.
