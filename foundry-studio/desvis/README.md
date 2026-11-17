# desvis

The desktop simulator and viewer: it runs a warehouse simulation and draws it,
live in a window or afterwards from a recording.

A run is drawn through `worldview-render`, the same renderer the browser viewer
draws through, with desvis's own egui panels around it — the metrics, the
Gantt, the station diagnostics, the entity explorer. Every run desvis executes
is recorded as a `.worldview` (`worldview/worldview`), which is also what it plays
back, what the `workbook` command reads, and what the Crucible server stores.

Design: `docs/designs/worldview-streaming.html`. Format:
`docs/designs/worldview-format.md`.

## Building

```sh
cargo build --release -p desvis
```

Release, always, for anything you intend to watch or time: a debug build runs
the simulator at about a third of real time. The first build takes a few
minutes; after that it is seconds. The commands below are written as
`cargo run --release -p desvis --`, which builds if it must and then runs; the
binary itself is `target/release/desvis`.

## Running a simulation

**Headless, writing a recording.** No window, the run as fast as the machine
will take it:

```sh
cargo run --release -p desvis -- sim/dessim/tests/fixtures/cencora_v3_spec.json \
    --headless --simtime 10m --threads 1 --output-dir perf/profiles
```

You get a progress summary on stderr, then the run's station, VTM and duty
tables, and a line naming the file. Ten simulated minutes of cencora takes
about 21 seconds on a quiet 16-core machine and writes a 1.4 MB recording; it
takes half again as long with something else heavy running. The file is named
after the branch and commit it was built from — `agent_worldview-p6_fca1dc7.worldview`
— so two runs of one spec from different commits do not overwrite each other.
`--name` labels the run inside the file, where the workbook's cover sheet shows
it; it does not change the file's name. `--xlsx <file>` writes the workbook at
the end of the run as well.

**In a window, watching it go.** The simulation starts on its own and the
window stays open:

```sh
cargo run --release -p desvis -- sim/dessim/tests/fixtures/minimal_easyloop_flow_spec.json \
    --run --simtime 5m
```

The window opens in about a second on the effective configuration, which is
also printed to the terminal; the floor fills with robots and totes, and the
panels fill as the run goes. Leave off `--run` to open the window with the run
paused, and press the start button yourself.

## Playing a recording back

```sh
cargo run --release -p desvis -- --recording perf/profiles/<run>.worldview
```

The window opens on the first instant, and the playback controls move the
cursor anywhere in the run. The recording carries the world document, so
nothing else is needed to open one — a recording somebody sends you opens on
your machine.

**Without a window**, which is what the parity gate uses: `--dump-render
<ms,ms,…> <dir>` writes the arrays the renderer would upload at each listed
instant, instead of drawing them. That is how playback is checked against
master element by element (`scripts/worldview/parity_b.sh`,
`docs/designs/worldview-render-dump.md`).

## Comparing two runs

```sh
cargo run --release -p desvis -- compare <old.worldview> <new.worldview>
```

Under a second. It aligns the two runs slice by slice and prints every metric's
means with a verdict, so a change in the simulator shows up as the metrics it
moved. Two runs of one spec on one commit agree on everything except the wall
clock they took.

## The workbook, and looking inside a file

Both are their own commands over the same recording:

```sh
cargo run --release -p workbook -- <run.worldview> --out book.xlsx
cargo run --release -p worldview --features cli -- inspect <run.worldview>
cargo run --release -p worldview --features cli -- seek <run.worldview> 300000
cargo run --release -p worldview --features cli -- index <run.worldview> --verify
```

The workbook takes about half a second; `inspect` prints every frame with its
offset, instant, size and sections, `seek` prints what a scrub to that
millisecond would read, and `index --verify` says whether the file's index
block matches the frames ahead of it.

## Tests

```sh
cargo test -p desvis --bins
```

The gates that matter for drawing are the two parity harnesses, which compare
this branch against a pinned master checkout over four fixtures:
`scripts/worldview/parity_a.sh` (workbooks, cell by cell) and
`scripts/worldview/parity_b.sh` (render arrays, element by element).
