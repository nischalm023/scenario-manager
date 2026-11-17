# Walmart floor alignment — September 8

The broader floor-alignment request remains unfinished. The apron-to-highway,
band-floor, west-buffer and putaway boundaries contain 275 laterally offset
compiled links. The [boundary audit](floor-boundaries-2026-09-08.md) lists their
identities, measurements, omitted test coverage and repair options. The installed
rack-bay and highway changes below align 172 inventory-to-highway links and
4,762 inventory lateral links; they do not establish alignment across every
adjoining region.

The floor now follows the existing relay and station positions. Equipment and
steel remain in place; changing the floor partitions resolves these mismatches
without shifting the EasyLoops or interpreting the unconfirmed layout revision.

## Geometry

| Area | Before | Updated |
| --- | --- | --- |
| EasyLoop apron | Uniform 852.8 mm X pitch; machine 1's two inventory ports shared `cell:31,0` | 104 adjoining columns with a separate, exactly aligned approach for each of the 24 EasyLoop doors and two box induction points |
| South/north cross-floor bands | Uniform 852.8 mm X pitch | 106 columns following the rack service strips, travel lanes, aisles and open-floor columns |
| Inventory service/travel rows | Continuous 687.9 mm Y pitch | Four 633.2 mm rows per ordinary bay; three 670.7 mm rows per tunnel bay |
| Inventory aisle/open-floor rows | Continuous 850 mm Y pitch | The same 21 bay sections as the under-rack floor |
| Inventory/buffer relay stands | Centres 374.5–395 mm inside the rack face | 800 mm service-strip width puts the centre exactly 400 mm inside the face, beneath the relay |
| Rack-block north boundary | Aisles/open floor stopped 422 mm short | Last bay ends at 74,366.0 mm, immediately before the cross-floor band at 74,366.1 mm |

The companion travel strips use the remainder of the original rack depth:
698–780 mm. The physical bays, relay offsets and rack envelope do not move.
Inter-bay links cross the existing 216.9–221.1 mm gaps. Each one-way travel lane
retains its direction across those joins. The 85 existing cross-aisle crossing
records retain two crossing rows in each of their named bays.

The apron and rack columns have different spacings. Their transition remains in
the open cross-floor band, with lateral travel available there. Exact alignment
at every station door and rack entrance does not require one uniform site-wide
pitch. All apron cells remain at least 695 mm wide; there are no thin filler
cells between mandatory door positions.

Six open-floor side seams previously jumped past the neighboring service strip
to reach a travel lane. They now connect to the immediately adjacent floor
section. Contact tolerances at new joins include only the physical gap plus one
0.1 mm primary step for centre-rounding error.

## Representation and provenance

Each original aisle/region identity remains the parent of its floor sections.
In particular, `access.along` still names one whole aisle; subdivision does not
create additional zones, fleets or services. Five floor templates express the
repeated sections using existing protocol components. This does not begin the
parked world-format migration.

Two small loader corrections support that existing representation:

- Floor-access validation checks contributed surviving cells, including cells
  inside stamped sections. A fully excluded grid still produces a warning.
- A template-local cell address expands to its declared root and full derivation
  key. Previously the expander put a derived address in the UUID-only generator
  field, so directed stitches between template sections could not be loaded.

The authorized patcher applies this plan through record byte spans:

```sh
python3 scripts/reauthor_dfw6_easyloops.py --align-floor --audit
```

Its pinned pre-image is git blob `6da68764ff0474ab54e0c5f236fef0755f2a09bf`
(the reviewed map at `a32b3cce0`). It updates 401 existing records, adds five
templates and proves that all 1,239 other records remain byte-identical. The
fleet remains 144 HTMs, 40 inventory VTMs and eight provisional buffer VTMs.
Vinay still needs to confirm the additional eight machines.

## Validation

The graph checks cover all 2,132 relay positions, 28 aisle identities, independent
inventory-port approaches, rack/open-floor seam alignment, one-way bay joins,
and floor footprint overlap. All five alignment checks and all ten existing
EasyLoop floor/door/curved-motion checks pass.

The final 500-source-order CollisionDes/HTM-kinematics gate completes all 576 picks,
500 extractions, 246 box orders and 316 physical buffer round trips. Conveyor
custody/capacity, shared-operator exclusion, reusable-container return and final
conservation checks pass. Batch completion is **5,350.287 simulation seconds**,
versus **5,449.938 seconds** on the same 144-HTM fleet before alignment: 1.83%
shorter. This is a finite-batch comparison, including fill and drain, not a
claim of steady-state site capacity. The runner used 181.901 wall seconds;
this rerun overlapped the long gate, so its wall time is not a controlled speed
comparison. The geometry-only checkpoint before the dispatch fix completed in
5,340.411 simulation seconds and used 168.647 wall seconds, versus 116.710
wall seconds before alignment. Faster simulated throughput does not imply a
faster solver.

The larger 4,000-order cohort exposed a dispatch defect during startup. HTM
`r(178)` was selected at parking cell `c(65)` even though all three outgoing
neighbors (`c(64)`, `c(66)`, `c(173)`) held other idle HTMs with open-ended
reservations. Box induction `c(680)` was clearing normally; its preceding hold
ended at 26.548 seconds. The static route existed, but the selected robot could
not leave. The same cohort did not reproduce this startup failure in a short
35-second wall-time control on the preceding map. That control is not a
completed long-run baseline.

Box induction and relay pickup now exclude robots whose legal first steps are
all occupied by other open-ended holders at departure. Selection rechecks the
exact proposed departure time, and acceptance rechecks before taking the robot
from the idle pool. If every candidate is blocked, existing fleet/reservation
notifications wake selection after an exit clears. Finite holds and future
holds remain for the pathfinder to time; this screen does not assert that the
whole route is clear, and missing legal edges remain static routing errors.
All 21 focused selection/claim/induction tests and 11 fulfillment orchestration
checks pass, including eight new regressions for the blockage, wakeup and timing
cases. The first rerun reached 4,000 fully picked orders and all 4,594 picks but
hit its 1,800-second wall-clock guard during cleanup (18,565 of 18,582 VTM cycles).
It is not a completed physical gate. The test budgets now allow 3,600 wall seconds
for the long cohort and 300 for the small cohort; simulated timing, collision
horizons and required outcomes are unchanged. The final long rerun passes: all
4,000 orders, 4,594 paired picks, 1,871 cartons, 2,568 buffer round trips and
18,582 VTM cycles finish in **42,297.482 simulation seconds** (11 h 44 m 57 s).
The runner used 1,852.130 wall seconds. Final custody, extraction, returns and
conservation assertions all pass. The longest physical pick-completion gap,
including startup, is 107.674 seconds; 1,288 orders remain unpicked at eight
hours. This is completion evidence for the finite cohort, not a claim that the
site throughput target has been met.

For the final 500-order cohort, mean conveyor WIP was 1.69–2.04 totes per
EasyLoop and mean boarding-to-collection time was 63.88–64.63 seconds. Peak
physical occupancy was four to five totes per conveyor, against a limit of eight.
Integrated physical WIP equals arrival rate times mean residence time for the
complete, drained cohort. Operator utilization over that same interval was
23.7–28.6%; the eight-tote physical conveyor limit was never reached.
These observations identify spare capacity, not a prediction of attainable
site throughput after upstream bottlenecks are removed.

The 173 protocol tests pass, including full-pipeline template-local directed
stitches and surviving-floor validation. Cencora map parity and loader checks
also pass. The legacy `the_putaway_spec_puts` test fails both before and after
alignment with the same initialization stall: 1,000 scheduler passes without
progress, zero picks and zero putaways. Its historical `putaway.enabled` setting
does not add an inbound root to Walmart's current composition. This existing
workflow gap remains; no putaway throughput improvement is claimed here.

After the shared HTM departure change, Cencora's
`minimal_putaway1_no_violations` also reproduces its previously documented
end-of-stock stall at the same 1,160.183-second barrier, after three picks and
43 putaways. This is not counted as a passing parity check.

Validation logs and custody records are under
`/Users/daniel/vt/greyorange/notes/walmart-floor-alignment-2026-09-08/`.

These changes prepare physical aisle cells for floor-running VTMs. They do not
add VTM floor reservations; the current collision gate still covers HTM traffic.
