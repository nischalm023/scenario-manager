# Variant time-series companion

See the [reporting-tool overview](README.md) for prerequisites and artifact
layout, and the [Rust exporter reference](../../native-modules/src/migration/traffic_report/README.md)
for its CLI, input requirements, JSON fields, units and boundary conventions.

The companion is `docs/easyloop/overnight-2026-09-14/variants.html`. It embeds
its data, CSS and SVG chart renderer, so it works offline when shared alone.
The original `report.html` is never written by this generator.
Use the [packaging command](README.md#share-the-finished-reports) when sharing
both reports with their linked evidence files.

Build the streaming profile exporter, then generate the document:

```sh
cargo build -p native-modules --release --bin traffic_report
python3 scripts/walmart_matrix/variants.py
```

`--base` selects a matrix artifact directory containing `manifest.json`,
`source-audit.json`, and each case's spec, profile and independent flow audit.
`--output` selects a different companion HTML path. `--refresh` ignores cached
exports. `--original-report` selects the single-page report whose hash the
companion records and whose contents it protects. Keep `report.html` beside
`variants.html` so the viewer's link resolves. Each case's `variant-series.json` cache is keyed by profile, spec and
exporter SHA-256. The generator validates completion totals, inventory balance,
rack occupancy identities and wait conservation before writing the HTML.

For one recording, the exporter can also be used directly:

```sh
target/release/traffic_report run.worldview spec.json 16 series.json --series-minutes 5
```

This rebuilds the run model from the recording and folds its slices in order —
it holds the whole run, and starts no simulation. Completion journals and aisle
snapshots are retained for small time-series exports. The output includes the existing cumulative cell
traffic plus `series.bins` and physical source-rack snapshots in `series.racks`.

Navigational wait is **elapsed eligible HTM wait / HTM count / observed hours**,
displayed as minutes per HTM per hour. `WarehouseMap::navigation_wait_exclusions`
is shared by this exporter and the desvis heatmap. It covers typed waiting/work
cells, relay access, station and handoff queues, charging, and complete conveyor
equipment membership (including all EasyLoop cells). Queues declared as Open
cells are excluded by role. Ordinary upstream cells remain eligible. This does
not classify the cause of a wait; task/resource waits and idle holds can remain.

The companion offers 15-minute and hourly rebinning, a common first-16-hour
window, consistent comparison scales, hover/keyboard values, per-page CSV and
the entire embedded JSON dataset. Rebinning sums counts and exposure before
division. Partial final intervals use their actual observed duration; they are
excluded from the peak-complete-hour KPI. Rejected cases have no invented data;
equivalent configurations reuse an explicitly named source recording and are
clipped to the equivalent case's requested horizon.

The source rack denominator is derived from inventory-zone aisle snapshots:
170,570 slots. The loader's 174,020-slot diagnostic also includes 3,450
order-storage slots. An empty inventory start still has empty source totes in
racks, so the charts distinguish stocked slots from any-tote occupancy.
