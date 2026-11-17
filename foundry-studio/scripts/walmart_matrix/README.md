# Walmart profile analysis and HTML reports

The exporters, generators, styles and chart renderer are tracked in Git on the
`walmart` branch. The generated September 14 HTML reports are also checked in.

The [complete guide](../../docs/report-generation.md) covers experiment
design, input provenance, collection, audits, both report formats, measurement
definitions, browser checks, packaging and adapting the templates.

| Tool | Reads | Produces |
| --- | --- | --- |
| [`prepare_variant.py`](prepare_variant.py) | The checked-in Walmart baseline plus selected preload, putaway, timing and induction options | A portable review spec and a printed headed-run command |
| [`traffic_report`](../../native-modules/src/migration/traffic_report/README.md) | One `.worldview` recording plus its matching simulation spec/map | JSON cumulative HTM traffic, optional flow/wait time series and source-rack snapshots |
| [`analyze.py`](analyze.py) | Profiles, source CSVs and the experiment manifest | Flow/buffer audits, report facts, backlog diagnostics and traffic JSON |
| [`report.py`](report.py) | The experiment manifest and precomputed analysis JSON | The original matrix report, with comparisons, spatial traffic figures and authored interpretation |
| [`variants.py`](variants.py) | Retained profiles, matching specs and completed analysis artifacts | A separate self-contained HTML with one page per variant, interactive charts and CSV/JSON downloads |
| [`package_reports.py`](package_reports.py) | Both finished HTML reports and their linked evidence | A portable ZIP with relative evidence links, audits/logs, instructions and file hashes |
| [`run.py`](run.py) | The dated matrix definitions and source fixtures | Frozen inputs and simulation runs; this is the runner, not a report renderer |

Start with the **[exporter reference](../../native-modules/src/migration/traffic_report/README.md)**
for a single profile, or **[companion regeneration instructions](VARIANT_CHARTS.md)**
for the HTML. `variants.py` invokes the Rust exporter as needed and reuses
matching cached exports. It never starts a simulation or overwrites the original
`report.html`.

## Reproduce the September 14 companion

From the repository root, with the retained experiment directory available:

```sh
cargo build -p native-modules --release --bin traffic_report
python3 scripts/walmart_matrix/variants.py \
  --base /path/to/walmart-matrix-2026-09-14 \
  --original-report /path/to/report.html \
  --output /path/to/variants.html
```

The companion renderer needs Python 3.11 or newer and only its standard library.
The earlier profile audit/analysis scripts additionally require Python 3.14
(`compression.zstd`) and `msgpack`.

The artifact directory contains:

```text
walmart-matrix-2026-09-14/
  manifest.json
  source-audit.json
  inputs/                         # Matching world, order and stock files
  <case-id>/
    spec.json
    state.json
    run.log
    *.worldview                   # Exactly one per recorded case
    flow-audit.json
    report-facts.json
    variant-series.json           # Optional cache; regenerated as needed
```

Rejected cases have their failed preflight log and no profile. Equivalent cases
identify their source recording through `alias_of` in the manifest. The original
matrix report also uses `buffer-audit.json`, `backlog.json` and `traffic.json`.
The archived directory currently lives at
`../notes/walmart-matrix-2026-09-14` relative to this checkout. It is **outside Git**;
a fresh clone has the tools and finished HTML, not all the underlying recordings.
Share the artifact directory too when another person needs to regenerate the
analysis. After relocating it, update absolute spec/manifest/analysis path
references to point to the same input contents in their new location.

The original report is regenerated with:

```sh
python3 scripts/walmart_matrix/report.py
```

That default command **replaces** the original `report.html` from existing
analysis. Prefer explicit revision paths:

```sh
python3 scripts/walmart_matrix/report.py \
  --base /path/to/walmart-matrix-2026-09-14 --output /path/to/revision/report.html
```

The analyzer also accepts `--base`, plus `--report-output` for watcher/refresh
operations. The runner still uses the dated `BASE` in `run.py`. Both report
templates retain this matrix's narrative and display assumptions; a new
experiment needs reviewed case definitions, controls and interpretation in
addition to new measurements. Invoking `run.py` is unnecessary when rebuilding
charts from existing recordings.

## Share the finished reports

The two checked-in outputs are
[`report.html`](../../docs/easyloop/overnight-2026-09-14/report.html) and
[`variants.html`](../../docs/easyloop/overnight-2026-09-14/variants.html). Both
embed their figures/data; the companion also embeds its JavaScript and CSS.
The original report also links to separate JSON audits and preflight logs.
Zipping only the two HTML files leaves those evidence links pointing at the
author's disk. Build the portable package instead:

```sh
python3 scripts/walmart_matrix/package_reports.py \
  --base /path/to/walmart-matrix-2026-09-14 \
  --output /path/to/walmart-throughput-reports.zip
```

`--documents` optionally selects a directory containing different finished
`report.html` and `variants.html` files. For this matrix, the packager includes
76 linked evidence files: 72 JSON audits/metadata files and four preflight logs.
It resolves the original `<case>/<file>` links under `--base`, so the evidence
directory can move to another machine. Packaged HTML copies use relative links;
the source reports and evidence files remain unchanged. A manifest records
source and packaged SHA-256 hashes. The command checks static HTML links and
anchors, parses all included JSON, and verifies the ZIP's integrity before
publishing it. The original report's source hash remains labelled as such in
the companion; the packaged report's hash is recorded in the manifest.

A reader needs only to unzip the **entire** archive and open `variants.html` in
a browser. Keep both HTML files and the `evidence/` folder together. The
companion's JSON and CSV download buttons work offline and do not need extra
JSON files. Source profiles, binaries and input fixtures are not included;
this is a report/evidence bundle, not a simulation reproduction archive.
