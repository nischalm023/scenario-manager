# Cencora validation on the Walmart branch

Daniel's September 15 decision is to land the Walmart integration as one PR.
Cencora needs regression validation against the accumulated renderer, world-map,
task construction and state-service changes before that landing.

## Baseline run

Iteration 59 runs Cencora v3 from `walmart` at `4c7b51b7f`, using
`sim/dessim/tests/fixtures/cencora_v3_spec.json`: eight simulated hours, 340 HTMs,
38 VTMs, empty initial inventory, and picking enabled after two hours.
`putaway.ignore_hour_gate=true` makes inbound lots eligible from T=0; station
induction remains paced at 216 totes/hour. The VTM pick/drop/alignment settings
are 5.7/4.6/2.75 seconds, and putaway placement uses random slots.

The requested `putaway.forbid_rack_after_secs=7150` option belongs to the unmerged
direct-empties branch and is absent here. It is omitted, without substituting
another policy. This run therefore measures the Walmart branch's Cencora flow;
it does not reproduce the direct-empties implementation.

The last earlier verified Cencora check was a 20-pick miniature smoke test on
September 12. It did not establish full Cencora v3 correctness or throughput.

## Live container rendering

Live polling advanced robot positions, while container rendering continued to
read the unselected playback cursor at T=0. Custody and contents consequently
remained at their initial state even as simulation inventory increased.

Container rendering now uses the same live-or-playback time as the resource
inspector. The regression test exercises relay pickup, an inducted unit, a live
HTM's position, backward scrubbing, and subsequent live updates while paused.
It failed before the fix. The Desvis unit suite then passed 124 tests, with the
two existing ignored tests unchanged. This correction changes visualization,
not simulation scheduling or inventory.

## Relay access geometry

All 684 world HTM access positions were one 850 mm column farther from the
VTM aisle than their legacy counterparts, alongside the steel. The old parity
test explicitly allowed this discrepancy. The corrected map declares three
access cells per rack bay beneath the boards, with explicit shelf bindings
and perpendicular links to the existing travel lane. Both shelf levels share
the corresponding physical stand.

The correction preserves rack, storage, station, fleet and existing floor
records. `scripts/align_cencora_relay_access.py --check` (since deleted; in git history) proved both committed
fixture copies equal the bounded patch of the source at `47734574`. It checks
the source hash and every existing record, without regenerating the map.

The parity gate now requires all 684 physical access positions to equal the
legacy coordinates, without applying a translation. All 31,740 world arcs
have legacy counterparts. The remaining differences are named and checked:
RTP flags on the common graph; 74 station-footprint HTM flags; and 684 inert
VTM-zone labels on relay ingresses that neither map permits VTMs to traverse.
Rack reporting anchors remain distinct from physical HTM access cells.

Validation passes all six map-parity tests, nine Cencora world-loader tests,
and both relay-maneuver tests. The maneuver checks cover Cencora, Walmart and
the minimal EasyLoop map: shared-floor connectivity, exactly one forward
entry and backing exit per relay, no rotation, and actual path reservations.
The renderer test separately places every Cencora and Walmart access centre
under a rendered board; Cencora failed that test before the correction.

## Corrected headed run

Iteration 60 uses the same eight-hour configuration as iteration 59, with
corrected relay access and live container rendering. Its label is
`Cencora v3 iteration 60 | walmart branch | relay access + live totes | 8h`.
The binary, scenario, world map and launch manifest are retained under
`../notes/cencora-review-2026-09-15/headed-60/`. Unit and geometry checks do not
establish full-run throughput; that remains a separate validation result.
