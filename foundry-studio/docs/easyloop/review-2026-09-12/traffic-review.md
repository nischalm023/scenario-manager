# Traffic heatmaps: measurement contract

Updated September 13, 2026. Read alongside Daniel's preserved [message](user-message.txt) and the [call review](transcript-review.md). Cumulative waiting and moving-speed views are now implemented; arbitrary observation periods and temporal/directional wave analysis remain follow-up work.

## Existing data is not sufficient as-is

`desvis/src/types/mod.rs:627` stores visit-count heatmap vectors. `sim/dessim/src/observer/time_slice.rs:188` stores visits and robot presence. Neither supplies the requested waiting duration or speed distribution. In `observer/nav_handler.rs:135-169`, cumulative presence includes Move, Wait and Rotate durations; it cannot be relabelled as waiting. Live UI visit reconstruction in `desvis/src/app/polling.rs:108-130` counts Move steps instead, so these paths do not even share one visit definition.

**Correction to the September 12 proposal:** it incorrectly treated navigation cancellation as supported behavior. The dormant `NavCanceled` event had no producer. It and the observer's unconditional navigation-tail removal have been deleted. Committed navigation is immutable, and per-robot overlapping commits are rejected before changing recorded facts. A new journey can end an idle display hold; it cannot erase prior movement. Parking may dissolve before proposal acceptance, never cancel a committed parking leg.

Accepted future plans are not yet elapsed traffic. The heatmaps integrate only the observed prefix, clipped to the simulation clock or playback cursor. This is observation-window clipping, not plan cancellation. Later-delivered commits can refresh idle playback snapshots and therefore their displayed totals.

`observer/nav_kinematic.rs:43-139` already interprets kinematic movement and curved links for playback. Reuse the physical motion definition, not the number of rendered frames or the visual microstep count. The existing renderer's microsteps all carry the destination cell ID; a per-cell speed measure needs an explicit spatial attribution rule instead of assuming that ID proves physical occupancy throughout an edge.

## First view: waiting time

Color each eligible cell by accumulated HTM waiting time, in HTM-seconds. Sum actual stationary waiting intervals, clipped to the elapsed observation window. Classify them once: route/path clearance, waiting for work/resource, intentional service, and rotation. Exclude deliberate service and rotation from waiting; expose reason subtotals so a long wait is not automatically described as congestion.

Exclude queue/waiting cells and all presentation/access cells through their semantic map relationships, including relay HTM-access cells even if their underlying kind is Open. Excluded cells remain neutral, and hover says why they are excluded. No observations and zero measured wait are different states. Existing parked/idle relay residents consequently do not dominate the map.

Implemented hover: accumulated stationary waiting in HTM-seconds and rotation duration, excluded from the color. Queue, presentation/access, exchange and charging cells are excluded. Eligible cells include stationary waits for work or resources, so this is explicitly not a pure congestion metric. Waiting episodes and reason subtotals are not yet implemented; a later congestion-only filter requires reliable cause classification.

## Second view: speed

Default color: **distance travelled / moving time**, displayed in m/s. Use actual translational movement, including movement through a curve. Exclude pure rotation and stopped time from this denominator; they have separate accounting. This answers how quickly HTMs move when traversing a cell. A cell with no moving exposure has no speed observation, not zero speed.

Accumulate a small fixed-bin, duration-weighted speed histogram as well as distance and moving seconds. This supports an approximate median and shows two speed populations without retaining every observation. Bins and units must be explicit; do not present approximate histogram quantiles as exact medians. Tooltips should include moving mean, exposure, and a compact histogram once the underlying measurements are verified. A count-only or mean-only view cannot answer the bimodality concern in the request.

Movement uses canonical committed spans, preserving trapezoidal kinematics where configured. Half of each link's distance belongs to its source cell and half to its destination, including curved links and map gaps/overlaps. Integration splits at the arc-distance midpoint and speed-bin boundaries. It conserves distance and elapsed motion independently of playback microsteps. The speed histogram uses 0.5 m/s bins through 5 m/s, then one ≥5 m/s bin. Hover reports the median's containing bin, not an exact median.

Standard deviation is not a congestion score. A large spread can describe perfectly healthy alternating fast and slow traffic. A histogram can show two populations, but also loses time order: the same histogram can come from separate waves or interfering flows.

Later, a selected-cell time strip should show speed exposure, stopped time, rotation, and travel direction in successive short windows. Crossing traffic may be moving straight across an aisle rather than turning, so straight-versus-turn alone is insufficient. Directional movement plus the time strip is the way to investigate whether traffic waves overlap. A cumulative view must not claim to establish that causal relationship.

## Storage, consistency and verification

- Use sparse per-cell scalar totals plus bounded histogram bins. Do not add dense cell × time-slice × histogram histories to the current memory footprint.
- Accumulate only the elapsed prefix of immutable accepted movement and holds. No cancellation or replacement event exists.
- Live display, completed-run output and playback at the same time must use the same measurement definition and exclusion mask.
- Verify one fixed-speed pass, a stopped interval, pure rotation, curved movement, and two known speed populations. Results must be invariant under UI frame rate, playback microstep size and observation slice boundaries.
- Reject overlapping navigation commits. Verify adjacent commits, live idle backfill, repeated slice delivery, reverse seeks, profile round trips and clipping at the simulation barrier without double-counting.
- Keep whole-run view as the first UI scope. Make the chosen aggregation period visible; period selection can follow without changing what the units mean.

The implementation retains one sparse accumulated prefix and a current snapshot, with two recent slices recomputed on live updates. Older profiles still load but lack canonical speed data; the UI reports that limitation rather than estimating speed from rendering microsteps. Waiting remains available from recorded stationary steps. The completed iteration 32 recording includes canonical motion for every HTM slice. A streaming audit reconciles canonical and playback movement at exactly 1,554,467,925 HTM-ms, with no overlap or duration difference. Profile round-trip and egui hover-unit tests pass; final GPU appearance still needs visual acceptance.
