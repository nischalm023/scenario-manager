# cencora_phase2 headless baseline (task-model-v2 gates G2/G3/G4)

Pinned 2026-07-20 by PR 2 (`tmv2/pr2-demand-di`, tracking issue #240).
This is the identity + performance reference every transformation PR in
`specs/task-model-v2/PLAN.md` compares against.

## Invocation

    DES_DIGEST=1 DES_HEARTBEAT=600 \
      cargo run --release -p desvis -- \
      sim/desmap/tests/fixtures/cencora_phase2.json --headless

(`DES_DIGEST=1` enables the FNV hash tier of the event digest;
`DES_HEARTBEAT=600` prints one `[hb]` stride line per 600 sim-seconds.
Both are read-only probes — see `sim/dessim/src/observer/digest.rs` and
`sim/dessim/src/planner/heartbeat.rs`.)

## The pinned identity (gate.txt, compared VERBATIM for G3/G4)

    result=complete stop=SimTimeLimit sim_ms=36000000
    picks=12301 putaways=22321 orders=328 events=1169536
    digest fnv1a=89308db1dda4b043  (per-kind counts in gate.txt)

The run completes the fixture's full 10-hour day and stops on the sim
clock (`SimTimeLimit`) — the honest terminal state for this fixture.

## Wall reference (informational, NOT identity)

- run2 (solo machine): **668 s** ≈ **53.9:1** sim:wall.
- run1 (concurrent with a full test-suite run): 787 s ≈ 45.7:1 —
  recorded to show load sensitivity; compare wall only against
  like-for-like load.

## Provenance of the two runs

- `run1-pre-change-hb.log` — binary at the instruments commit
  (7d30b00f), BEFORE the Phase-1 demand-DI transformation.
- `run2-post-change-hb.log` — binary after the DI move + `Demand`
  rename (d5579e6f).
- The `[gate]` identity lines of the two runs are **byte-identical**
  (every counter, every per-kind event count, the FNV-1a hash) — that
  equality is PR 2's G4 evidence, and it simultaneously witnesses
  cross-build determinism of the fixture on this baseline.
- Same-binary double-run determinism was separately proven on
  cencora_phase0 (digest `cd9f3ebaa5cb5029` twice; note phase0's
  terminal state on master is a PRE-EXISTING planner deadlock bail at
  sim≈3679 s — the `[gate] result=error` path exists exactly so such
  fixtures stay A/B-comparable).

## How to compare a candidate run (the A/B method)

1. Re-run the invocation above on the candidate.
2. `diff` the `[gate]` lines minus `wall_secs` against `gate.txt` —
   G4 (transformation PRs) requires byte identity; G3 requires
   `sim_ms`/`picks`/`putaways`/`orders` not to regress.
3. For perf triage on a mismatch or slowdown, JOIN the `[hb]` lines on
   the `stride=` key against the pinned logs — never line-diff heartbeat
   streams (wall fields differ run to run by design). Method provenance:
   seam `SHAPE.md` R4.19; `perf/gate-evidence/2026-07-20/README.md` on
   `bakeoff4/seam-live-gui`.

The recordings themselves are not committed (≈58 MB); regenerate with the
invocation above, which writes `.worldview` files now.

**Known identity shift (2026-07-30, destination-striping slice):** bulk-pool
runs now emit one `StripePlan` event at t=0, so against this pinned gate the
candidate shows `events` +1, a new `StripePlan=1` per-kind count, and a
different FNV hash. That exact delta is expected and benign; anything beyond
it is a real mismatch. Re-pin `gate.txt` on the first transformation PR after
the striping slice merges.
