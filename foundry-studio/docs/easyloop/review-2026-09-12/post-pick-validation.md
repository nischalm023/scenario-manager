# Post-pick station validation — 13 September

Implementation checkpoint: `33452fd27`. The [layout and workflow guide](../outbound.html)
contains the recovered client drawing, station quantities, provisional coordinates,
implemented routes and remaining assumptions.

## Checks

- Release desvis build passed.
- Library suite: 972 passed, one ignored, four profiling benchmarks excluded.
- Integration suites: 38 passed across Cencora parity, EasyLoop orchestration,
  early collection and HTM custody. New checks exercise role-specific routing
  for cartons, trays and both tote routes, and reject sealed-box pickup as a
  source of new empty boxes.
- Full-map gates verify station counts, independent service/approach cells,
  loaded arrival from all eight EasyLoop egresses, and empty departure toward
  a relay. The same reachability checks cover the six relocated decant stations.
  New handoffs enter and leave their presentation cells straight.
- The existing DFW6 inventory, rack, fleet and EasyLoop invariants passed.
  The map authoring check proves unrelated entities remain byte-identical.

An additional, obsolete putaway probe failed with a scheduler no-progress error.
Its fixture selected Ghost mode and the Walmart composition, which does not
contain a putaway root task. That probe is excluded from validation; the fixture
now selects the normal planner. Its missing putaway task configuration and
runtime failure remain unresolved. Reachable decant approaches do **not** establish
that putaway works end to end.

## Initial full-map run

Iteration 27 completed the requested 1,200 simulated seconds in 123.7 seconds
of simulation wall time, with exit code zero. This is an initial flow check,
not a full-workload completion or a sustained-throughput comparison.

| Measure | Result |
| --- | ---: |
| Picks / fully picked orders | 369 / 369 |
| Mean picks per simulated hour | 1,107 |
| HTM fleet idle time | 50.4% |
| VTM fleet idle time | 62.3% |
| Putaways | 0 — no putaway task in this run |

The run uses 230 HTMs, 48 VTMs, the full map and order file, T0 order release,
buffer bypass, normal extraction and normal box induction. Operator picks take
ten seconds and handoff dwell remains nine seconds. Its early single-unit
orders do not validate later multi-line behavior. Sealed-box return and shared
sealing-equipment capacity are not implemented.

Provenance under the surrounding workspace's `notes/easyloop-layout-2026-09-12/`:

- `iteration-27-spec.json`, `iteration-27-world.json`, and `desvis-iteration-27.build.json`
- `post-pick-230-20m-process.json` and `post-pick-230-20m.log`
- `post-pick-230-20m/walmart_33452fd.desprofile`

Frozen binary SHA-256:
`dab90516fc9c46d52fd2accf0549d4eb025fc08fcf020c6c06ebeb82e51b753f`.
The headed iteration 27 uses this same binary and snapshot, with an eight-hour
limit, and was launched without automatically starting the simulation.
