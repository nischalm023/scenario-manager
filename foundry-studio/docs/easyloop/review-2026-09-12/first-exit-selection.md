# Selecting an HTM that can leave its current cell

The induction-overlap full-map run at `2a82e8202` failed after 337 picks.
HTM 209 was selected for a relay pickup from cell 12091 to relay-access cell
6654, starting at 1,077.899 seconds. The destination was empty and the source
hold belonged to the selected HTM. Its only outgoing edge, a reverse move to
cell 12092, was booked continuously by 26 other HTMs until 1,388.830 seconds.
The route could not start within its 300-second search horizon.

The source is a buffer service strip with rotation forbidden. Its only exit
joins the buffer travel lane. The diagnostic replay reproduced the same
failure and printed the actual reservations; this was not an inferred
destination blockage or a static map disconnection.

## Correction

HTM selection previously screened only already-active open-ended exit holds.
It now also checks whether any graph-admissible first neighbor has an opening
within the pickup route's horizon. A trapped candidate is set aside while
selection considers another available HTM. The proposal repeats this check
before acceptance so a newly booked exit cannot silently invalidate selection.

If every candidate is delayed by finite traffic, the earliest delayed candidate
can publish a normal future departure proposal at a known first-neighbor opening.
It stays available until acceptance, so earlier competing work can still take
it. This avoids sleeping for an oracle mutation that reservation expiry alone
does not generate. Open-ended blockage continues to wait for a resource change.

The navigation query is a necessary feasibility screen, not another route
planner. It respects directed edges, cargo permissions and destination gates,
and asks the reservation oracle for the first opening, including separation.
Heading, move duration, narrow gaps, counterflow and downstream traffic remain
the pathfinder's responsibility. No dwell or existing reservation is shortened,
and the route-search horizon remains unchanged.

## Validation

The two-HTM regression fails when the finite-exit screen is omitted: it chooses
the nearer trapped HTM. With the correction it chooses the farther usable HTM
without spending either during selection. Removing the alternative also proves
that a single delayed HTM completes at the known future clearance without an
unrelated oracle mutation, and without overlapping reservations.

Additional checks cover oracle-only invalidation before acceptance, alternatives,
destination gates, resident work and missing static exits. The library suite
passes 969 tests, with one ignored and four unrelated profiling benchmarks
excluded. All 33 Cencora parity, fulfillment and relay-custody integration
checks also pass. The full-map rerun will determine the combined induction/selection
effect; no throughput gain is claimed for this correction yet.

## Full-map follow-up

The combined induction/selection build `1b448c56f` passed the original failure
but encountered a second relay-pickup horizon failure after 667 picks, at
2,795.721 seconds of planned work (settled barrier 2,369.081 seconds).
HTM 228 was departing buffer service cell 11986 for relay-access cell 2550.
The first-neighbor screen does not establish full temporal route feasibility;
this later failure remains unresolved.

The induction departure-demand optimization is therefore backed out pending
a stronger route-selection contract. Its source and regression tests remain
available in commit `2a82e8202`. The independent first-exit screen and future
candidacy tests remain. The benchmark should return to the previously measured
navigation-overlap flow, with that additional selection guard, before another
headed inspection is offered.

After the rollback, 966 library tests and all 33 targeted integration tests pass
(one ignored test and four unrelated profiling benchmarks excluded).

The frozen release build `84e1a9aca` completed the one-hour full-map control
without a route failure: 860 picks, matching the earlier healthy build. Fleet
busy time is 77.03%, and mean physical residents are 4.48, 5.00, 4.72 and 4.60;
these also match the earlier first-hour measurements. This validates recovery
through one hour, not removal of every possible temporal route failure.

The saved profile is
`notes/easyloop-layout-2026-09-12/stable-selection-230-1h/walmart_84e1a9a.desprofile`
in the surrounding workspace. Frozen binary SHA-256:
`c4cbe220b987e19193495670765e97f5eff7dec0945e94886998de7572da452f`.
Iteration 22 was launched without `--run`. A separate eight-hour validation
uses a 90-minute wall budget because the earlier 45-minute budget stopped
iteration 19 at 6.414 simulated hours; its results remain pending.
