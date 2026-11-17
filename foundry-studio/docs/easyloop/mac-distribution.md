# Temporary Walmart macOS distribution

The `walmart-demo` feature makes desvis open the bundled full Walmart scenario
without arguments. Opening the app does **not** run the simulation; the user
presses **Run**. This is short-lived tooling for Kalpesh's Walmart demonstrations,
not a new general installation framework.

## Build an iteration

From this checkout on macOS, with Rust and the Xcode command-line tools available:

```sh
python3 scripts/package_walmart_macos.py --iteration 1
```

The default builds a universal Apple silicon/Intel app for macOS 13 or newer.
Both Rust targets must be installed (`aarch64-apple-darwin` and
`x86_64-apple-darwin`). For a faster Apple silicon-only local check:

```sh
python3 scripts/package_walmart_macos.py --iteration 1 --arch arm64 --output-dir /tmp/walmart-local-demo
```

`CARGO_TARGET_DIR` can reuse an existing build cache. `--output-dir` defaults to
`dist/walmart-demo` in the checkout. A fresh numbered release folder is produced;
an existing iteration is never overwritten. Increment `--iteration` for every
version shared with colleagues. Use a separate output directory for local checks
so an arm64 check does not occupy a universal release's iteration.

Each release contains the versioned `.app`, a ZIP containing that intact app,
`README.txt`, `manifest.json`, and `SHA256SUMS.txt`. Share the ZIP in Slack;
recipients need no source checkout, compiler, Python, or separate data files.
This script does not upload anything.

## What is bundled

`Contents/MacOS/desvis` is the actual executable; there is no shell launcher.
It locates `Contents/Resources/walmart/walmart_dfw6_spec.json` relative to itself,
so moving the app or launching from Finder does not depend on a working directory.

The build copies the main spec, map, full on-hand CSV, and full outbound CSV.
If the main spec declares `scenario.stock_top_up_csv`, that overlay is copied too.
Only the copied spec's file paths are rewritten; the source CSVs are unchanged.
The main spec has no order-count cap. Any stock overlay remains explicit in the
spec and manifest, separate from the site's original on-hand extract. Automatic
inventory placement remains provisional pending Vinay's mock-put file.

Fonts, shaders, the HTM mesh and named compositions are already embedded in the
binary. The packager rejects non-system dynamic libraries and wrong binary
architectures. Its advertised minimum is macOS 13; a real smoke test is still
needed on each hardware/OS combination before claiming compatibility there.

Build identity is compiled from `WALMART_DEMO_VERSION` (for example `001`),
`WALMART_DEMO_COMMIT` (full SHA), `WALMART_DEMO_BRANCH`,
`WALMART_DEMO_DIRTY` (`true` or `false`), and `WALMART_DEMO_BUILT_AT`
(UTC RFC3339). Recorded results use that commit instead of inspecting a colleague's
working directory or invoking git. The manifest records the copied input hashes,
architectures, signing mode, and build timestamp; the external manifest also
records the signed executable and ZIP hashes. Prefer a clean WIP commit for a
shared build so its commit fully describes its code.

## Signing and first launch

The default uses an ad-hoc signature and verifies the finished bundle. If a valid
Developer ID is available, pass `--sign-identity 'Developer ID Application: …'`.
That signs with the hardened runtime and timestamp; this script does not submit
the app for notarization.

The generated README explains first launch. This package is not notarized, so
macOS can block a downloaded copy. For a trusted copy, Apple documents the
app-specific **Open Anyway** control in **System Settings → Privacy & Security**;
managed Macs may require IT assistance.
[Apple's first-launch guidance](https://support.apple.com/en-us/102445).

Do not change files inside the signed app. Build a new numbered iteration instead.
No global security settings or quarantine-removal commands are part of this tool.

## Release checks

- Unzip a copy outside the checkout, preferably into a path containing spaces.
- Double-click it with no arguments: the full Walmart map loads and stays idle.
- Press Run, then exercise Stop/Reset and another Run. The same scenario should
  load again, and the window should identify this iteration.
- Confirm the effective run has 144 HTMs, 48 VTMs, collision avoidance enabled,
  full recorded stock plus any declared overlay, and all 22,160 source orders.
- Keep a clear distinction between a packaging smoke test and a validated full
  simulation. The full all-order run has not yet been validated to completion.

The ordinary headed GUI does not automatically save profiles or XLSX files unless
an output path has been configured. The app bundle should remain read-only;
outputs belong in a writable user directory, never inside `Contents/Resources`.
