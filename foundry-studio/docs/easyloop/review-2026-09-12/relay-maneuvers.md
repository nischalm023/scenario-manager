# Relay entry and backing exit

September 12, 2026. Daniel moved this ahead of the throughput work.

The Walmart coordinates did not need moving. All 2,132 HTM standing positions
match the transformed relay shelf offsets, and a separate Desvis regression
checks that each position lies beneath a rendered board. The cell used for HTM
presentation is distinct from the synthetic rack reporting anchor and the VTM
exchange position. Neither of those is a reason to move the HTM a cell sideways.

The compiled floor now gives every relay one perpendicular approach. An HTM
enters forward, presents without turning, and backs out over the same segment
with its chassis heading unchanged. Along-rack travel through the presentation
cells is removed; the adjacent travel lanes remain available. The graph carries
the drive direction and a prohibition on HTM rotation. Reservation planning,
travel estimates, robot handoffs, and playback use that same contract. Reverse
movement stops before and after translation; it does not acquire an instantaneous
180-degree chassis rotation. Existing curved EasyLoop presentation movement is
preserved.

Initial relay residents inherit their forward-entry heading. Available robots
retain their actual heading through claims, parking, pickup, deposit, and reuse.
This also corrects relay candidate ranking that previously assumed an east-facing
source. Profiles retain reverse movement and slice starting headings so a later
stationary robot does not change orientation when the user seeks in playback.

The same maneuver applies to both Cencora loaders. Their previously documented
standing-position difference remains visible in the [parity report](relay-parity.md);
this work does not resolve the deferred format migration. The legacy parametric
generator additionally needs an actual lane between rack blocks: end-cap-only
access cannot justify a terminal relay, and adjacent three-column blocks otherwise
strand entire rack faces. The generated scenario test floor is widened to retain
its requested inventory capacity after those lanes are restored.

Validation includes all real-map relay maneuvers on Walmart, minimal EasyLoop,
and both Cencora loaders; physical Walmart shelf/stand and rendered-board checks;
unchanged curved approaches; complete minimal fulfillment; final-pick direct
egress regressions; and heading preservation in paths, task handoffs, profiles,
and playback. This establishes the corrected behavior, not a throughput gain.

The next throughput control is
`sim/dessim/tests/fixtures/walmart_dfw6_throughput_control_spec.json`.
It is the canonical full workload with only T0 order release and direct extraction
enabled. Source timestamps and exit designations are retained. Both Walmart specs
now explicitly use a 10-second operator transfer and an independent 9-second
extraction service. The multi-unit interpretation remains a calibration question.

Assignment/preparation separate from admission, direct first presentation,
physical conveyor sequencing, and exact station state diagnostics remain next.
