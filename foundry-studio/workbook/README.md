# workbook

A finished run as the xlsx people read it in: the raw fact logs, the report
sheets each family owns, and the charts on them.

Almost everybody who does not run the simulator meets a run through this file,
so the sheets are the run's public face and have to say the same thing every
time. The crate computes nothing canonical — every value is built by
`dessim::metrics::reporting` — and owns only which sheet a value lands on,
where, and in what number format.

Both ways of producing a book go through one function, so they produce the same
book: desvis writes one when a run it ran completes, and the `workbook` command
writes one from a `.worldview` recording, days later and on another machine.

## Building and running

```sh
cargo run --release -p workbook -- <run.worldview> --out <book.xlsx>
```

About half a second for a ten-minute cencora recording. A recording with no end
block — a run that was cut short — yields a partial book over the slices it did
reach, and the command says so on the way past.

## Testing

```sh
cargo test -p workbook
```

Two runs of one spec are compared cell by cell in the parity gate rather than
here:

```sh
scripts/worldview/parity_a.sh <results-dir>
```

That gate builds a book from this branch and from a pinned master checkout for
four fixtures and compares every cell under a named exclusion list (wall-clock
timings and the provenance stamps, which differ by construction).
