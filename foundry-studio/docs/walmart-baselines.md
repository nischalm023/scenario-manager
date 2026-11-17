# Running the Walmart baselines

For a headed demonstration with picking and putaway together, use
[`walmart_dfw6_baseline_partial_spec.json`](../sim/dessim/tests/fixtures/walmart_dfw6_baseline_partial_spec.json).
It reproduces the configuration of `partial-on-t0-order_release` in the
[September 14 report](easyloop/overnight-2026-09-14/variants.html#partial-on-t0-order_release).
That recording averaged **1,249.6 picked units/hour over 16 simulated hours**,
with a peak complete hour of **1,391**. It had the best measured average in the
matrix; the small difference from the full-preload control is not evidence of
a statistically significant gain.

## Checkout and launch

Prerequisites: access to the private GitHub repository and a working Rust/Cargo
toolchain. On macOS, Xcode Command Line Tools supply the native compiler/linker.

For a new checkout:

```sh
git clone --branch walmart git@github.com:greyorange/greymatter-foundry.git greymatter-walmart
cd greymatter-walmart

cargo run --release --locked -p desvis -- \
  sim/dessim/tests/fixtures/walmart_dfw6_baseline_partial_spec.json \
  --run --simtime 16h \
  --label "Walmart baseline | partial prefill + putaway | T0 | 16h"
```

Run the command from the repository root. The first invocation builds desvis;
subsequent runs reuse the build. The window opens and starts calculating
automatically. Omit `--run` to inspect the map first and use the Run button.

In an existing checkout, fetch and switch to the `walmart` branch, then update
it with `git pull --ff-only origin walmart` before using the same Cargo command.
Record the checked-out commit when comparing results across versions.

All required map and CSV inputs are tracked in the repository. The preset uses
relative paths and does not need the original author's local experiment archive.

## What this runs

- Partial preload: 747,243 units in 81,030 stocked inventory totes, with the
  remaining stock supplied through putaway.
- All 22,160 orders released at T0, for a saturated-throughput comparison.
- 230 HTMs, 48 VTMs, 128 reusable order trays, four EasyLoops and a 10-second
  operator transfer time.
- Putaway enabled at six stations, up to 60 fills/hour per station and six units
  of one SKU per synthetic fill, using `order_release` sequencing.
- Physical box induction and extraction, with a one-hour synthetic DS buffer
  hold. The normal traversal and resource constraints remain active.
- A 16-hour simulation horizon. The GUI stays open for inspection afterward.

This is the measured comparison configuration, including the 128-tray diagnostic
setting. The original partial-putaway preset uses different defaults; use the
`baseline_partial` file above when reproducing these reported numbers.

## Full-preload control

For the comparison with putaway disabled, replace the launch command with:

```sh
cargo run --release --locked -p desvis -- \
  sim/dessim/tests/fixtures/walmart_dfw6_baseline_full_spec.json \
  --run --simtime 16h \
  --label "Walmart baseline | full prefill, putaway off | T0 | 16h"
```

This preloads all 1,397,632 units and retains the matching fleet, order-release
and outbound settings. The measured `full-off-t0` recording averaged **1,245.1
picked units/hour**, with a peak complete hour of **1,415**. Initial stock
availability and placement also differ, so this pair does not isolate only
the cost of putaway.

These presets reproduce the frozen configurations using repository inputs;
the input hashes were checked against the measured archive. The baselines use
provisional inbound and outbound rules, a simplified VTM model and no battery
workload. See the report for interpretation, and the
[general report-generation guide](report-generation.md) for retaining and
comparing new measurements.

## Choose another variant

The standard-library Python helper exposes the comparison variables while
preserving the baseline's fleet, trays, map, operator and outbound controls:

```sh
python3 scripts/walmart_matrix/prepare_variant.py \
  --preload partial --putaway on --order-timing t0
```

It writes `target/walmart-variant.json` and prints a ready-to-run headed Cargo
command with a descriptive label. It does not launch anything itself. Run the
printed command, or use:

```sh
cargo run --release --locked -p desvis -- \
  target/walmart-variant.json --run --label "Walmart variant review"
```

| Option | Choices | Default |
| --- | --- | --- |
| `--preload` | `partial`, `full`, `empty` | `partial` |
| `--putaway` | `on`, `off` | `on` |
| `--order-timing` | `t0` releases all orders immediately; `source` respects the order file | `t0` |
| `--induction-sequence` | `order_release` follows static order priority; `sku_demand` ranks SKUs by total demand | `order_release` |
| `--induction-lead-secs` | Seconds before source order release when demand lots become eligible | Omitted: all lots eligible at T0 |
| `--hours` | Positive simulated duration | `16` |
| `--output` | Generated spec path | `target/walmart-variant.json` |

Useful configurations, followed by the printed launch command:

```sh
# Full-preload control, no putaway
python3 scripts/walmart_matrix/prepare_variant.py --preload full --putaway off

# Empty inventory start with putaway
python3 scripts/walmart_matrix/prepare_variant.py --preload empty

# Partial preload with the order file's release timing
python3 scripts/walmart_matrix/prepare_variant.py --order-timing source

# Earlier SKU-demand induction policy for comparison
python3 scripts/walmart_matrix/prepare_variant.py --induction-sequence sku_demand

# Source timing with demand lots eligible one hour before release
python3 scripts/walmart_matrix/prepare_variant.py \
  --order-timing source --induction-lead-secs 3600
```

Choose one preparation command per run; each replaces the default generated
spec. Use distinct `--output` paths to retain multiple variants. The two
checked-in baseline presets are protected from overwrite. All generated input
paths are relative to the output spec and point into this checkout.

The flags cover the archived matrix's configuration dimensions. Use `--hours 52`
for its extended source-timed cases; source releases span roughly 47 hours.
Induction lead applies to `source` timing with putaway on and `order_release`
sequencing. Induction settings have no workload effect with putaway disabled.

Partial/empty preload with putaway off are intentional negative controls: they
fail the insufficient-stock preflight. Full preload with putaway on has no
remaining finite inbound stock, so it is equivalent to the no-putaway workload.
Other variants can expose stock-readiness or release-timing restrictions; they
are not all saturated-capacity tests. The current 16-hour default keeps the
common observation window used in the report.
