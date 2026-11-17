# Putaway pick-station locality (DC striping v1)

**Status:** Phase 1 (destination striping + UI visualization) is IMPLEMENTED
on branch `agent/destination-striping-viz`, sanctioned by the settled
decisions D1/D9/D10/D11 and the Phasing section; an implementation review
round (Opus, 2026-07-30) is folded in — "(Opus review)" marks. Phases 2–4
remain DESIGN ONLY: do not implement them until D2–D8 and Q1 are settled.
The pre-implementation Codex (gpt-5.6-sol) spec review completed 2026-07-30
(session 019fb064-5043-7092-a82f-e25de88297f7); its accepted findings are
folded in and marked "(Codex)".

**Issues:** subset of [#245](https://github.com/greyorange/greymatter-foundry/issues/245)
(dynamic DC striping across pick stations) and
[#246](https://github.com/greyorange/greymatter-foundry/issues/246)
(selectable putaway location-assignment strategies — this is its required
strategy "DC/pick-station locality bias"). Adjacent: #244 (pallet planning,
which asks whether pick-station assignment is known at pallet composition —
this spec answers "yes, via the stripe map"), #159/#162/#163 (pick sequencing,
out of scope here), Dematic backlog T-08 (hybrid putaway ranking, Plan-B,
unaffected).

**Baseline commit for all measurements:** master `14d2c213`.

---

## 1. Motivation (Daniel, 2026-07-29)

Two goals:

1. **Make picking more efficient** by pre-locating inbound inventory near the
   pick stations that will later pick it.
2. **A short-term but durable HTM pick-flow fix:** locating first-picked items
   closer to their stations means first picks tend to dispatch first, smoothing
   HTM arrival at pick stations — versus today's placement, which tends to be
   the reverse, because storage-slot proximity to putaway stations is generally
   inverse to pick-station proximity in this layout.

Scheduler context: we now understand that eager scheduling is bad —
particularly the unjustifiably bad VTM schedule sparsity it produces. A lazy
scheduler is planned and may improve that on its own. Before it lands, the
theory is that reversing putaway locality from putaway-station-biased to
pick-station-biased helps in the short term: picking becomes FIFO with respect
to storage locality instead of LIFO, and most from-rack HTM delivery legs get
shorter, which may raise VTM throughput purely through more favorable timing.
This is not just a bandaid: pick-station-biased putaway is objectively closer
to real-world behavior, so the change is a keeper regardless of what the lazy
scheduler later recovers.

## Phasing (Daniel, 2026-07-30)

The work ships in four phases, in this order — visualization before behavior,
audit before enforcement. This supersedes the draft's single-feature scope
(see decision D2, superseded).

**Phase 1 — Destination striping + UI visualization (current phase).**
Compute the stripe map (Destination-keyed, static per D1) and make it
*visible* before anything consumes it: a planned-stripes view and a
striping-state vs per-destination pick-throughput time-series chart in
desvis. Validate the striping visually and with unit tests first.

Integration constraints for phase 1 (from the metrics agent, relayed by
Daniel 2026-07-30):

- Branch bases on `origin/agent/d30-state-family` (`e4838f1e`, PR #320
  "Canonical D30 vehicle-state report family", merging imminently); rebase
  onto `origin/master` once #320 merges.
- The striping view goes **inside the existing DC-distribution tab**, not a
  new tab: `desvis/src/metrics_panel.rs :: draw_dc_distribution()` gains
  an "Assigned stripes" companion with the SAME axes (destination × pick
  station) — either a Planned/Completed toggle next to the existing
  Measure/Display selectors, or side-by-side matrices. Planned (stripe map)
  and Completed (existing heatmap) are explicitly labeled and never mixed in
  one cell — the planned-vs-completed separation rule recorded in
  `specs/dematic_kpi_reporting/plan.md`.
- Rename the tab "DC distribution" → "Destinations" in the same PR:
  `MetricsView` enum + `label()`/`description()` in
  `desvis/src/types/mod.rs`, and the report-cache/dispatch entries in
  `metrics_panel.rs`. Add a definition line in-tab: "Destination =
  distribution center (DC) for Cencora."
- Do NOT rename xlsx sheets (`DC x Station` stays), and do not touch the
  DA50/D30 report families or `workbook/src/d30.rs` — separate review.

**Phase 2 — Putaway locality + earmarking.** Destination earmarks on
inducted inventory (§4.2), the deposit strategy seam (§4.3), locality
scoring (§4.4). First behavioral consumer of the stripe map.

**Phase 3 — Order ↔ pick-station assignment audit.** Measure how bulk
orders actually bind to stations against the stripe map (natural adherence,
no constraint yet). Open question Q1 below: whether the binding *constraint*
(§4.5) ships in this phase after the audit or is a separate later decision.

**Phase 4 — Pick-station vs earmark audit.** Record the consumed inventory's
destination earmark in pick events and report the cross-earmark consumption
rate (D9's metric) and pick-station-vs-earmark-stripe adherence. (Earmark
*plumbing* is phase 2; the audit surfaces are phase 4.)

## 2. Current behavior (master)

**Storing.** Filled putaway totes and post-pick returns go through the same
code path: `ReturnTotesToRack::create_store_chain` calls
`RelayService::reserve_relay_for_deposit`
(`sim/dessim/src/flows/ttp/demand/return_totes_to_rack.rs`,
`sim/dessim/src/model/relay/mod.rs` — the deposit reservation). Every zone
with deadlock headroom is considered; each relay contributes one candidate —
its cheapest empty rack slot by VTM return-cycle cost
(`best_empty_slot_per_relay` in
`sim/dessim/src/model/relay/inventory_index/mod.rs`). Candidates are scored by
predicted end-to-end completion: loaded HTM travel from the origin station cell
to the relay, wait for the relay and its floor access cell, wait for a VTM in
the zone, plus the VTM return cycle. Earliest completion wins. Putaway-station
locality enters through exactly one term — the loaded HTM travel leg — and that
is the term this feature displaces.

The function is handed only the origin cell, time, robot id, schedule pressure,
and a parking-relief adapter. It never sees the tote, its SKU, or its DC. A
`FIXME(zone-targeting)` at the candidate-enumeration site already anticipates
target-zone filtering. There is no strategy abstraction on the deposit side;
the retrieve side has both precedents this design reuses — a score-closure
parameter (`select_relay_for_retrieve_locked_with_score`) and station-targeted
selection (`select_relay_for_retrieve_near`).

**DC provenance is discarded before induction.** The pending-putaway list
reduces each demand-pool row to `(SkuId, qty, net_time)`
(`ArrivalItem` in `sim/dessim/src/flows/ttp/demand/feed_putaway_stations.rs`),
and putaway induction emits `dc_id: None` on its CycleComplete events
(`sim/dessim/src/flows/ttp/demand/putaway_induction.rs`). This is also why the
Dematic "DC Stock Saturation" KPI currently has no putaway-side DC association;
this feature incidentally fixes that.

**Picking.** Bulk orders are built one DC at a time in product-sequence order
(`sim/dessim/src/flows/ttp/demand/order_builder.rs` — DCs in first-appearance
order, first feasible DC drained before moving on) and inserted unbound into a
single global FIFO ready queue. Stations are polled round-robin; a station
wanting a new order scans the queue from the front and starts the first
feasible order — the only station filter is "unbound, or already bound to me"
(`sim/dessim/src/flows/ttp/demand/pick/dispatch.rs`, the eligibility match).
`start_picking` (`sim/dessim/src/flows/ttp/demand/order_queue.rs`) is the only
place an order's station is ever set, and it has exactly one caller — which is
what makes "already-active orders stay pinned when stripes move" structurally
free. `dc_id` exists on bulk orders but every read site is reporting.

**Pick-by-pick execution** is soonest-arrival-wins among the station's open
orders (`pick.sequencing` only narrows which lines compete). Sequence adherence
is therefore driven by travel time — which is why storing first-picked items
far away scrambles arrival order today, and why pre-positioning should smooth
it.

**No DC → pick-station affinity of any kind exists in the codebase** (verified
2026-07-29: no striping/affinity code, config, or spec; the only prior art is
the Java put-service's popularity-by-station-distance placement, and the
ancient `specs/zone_aware_sku_distribution/` write-up, both Java-era — treated
as historical only, per Daniel).

## 3. Scenario facts that shaped this design (cencora_v3, peak day)

- 10 pick stations, 16 putaway stations, 38 VTMs, 340 HTMs, 5472 rack slots.
- Initial rack inventory is **empty** — all 66,276 units flow through putaway,
  so putaway placement fully determines pick-side geography.
- **26 DCs**, every one smaller than one station's fair share: largest is 6.9%
  of units, smallest 0.1%. So the operative regime is *stations serving 2–3
  DCs*, not DCs owning multiple stations. "Fewer stations than DCs" is not an
  edge case here; it is the case.
- **Demand is well interleaved:** 23–26 of the 26 DCs have putaway volume in
  almost every hour of the product sequence (worst hours: 14 and 19 DCs, both
  tiny-volume hours). DC-burstiness — the main transient-starvation risk of
  striping — is low in this dataset.

## 4. Design

### 4.1 Stripe map (static v1 of #245) — phase 1

Striping is **destination pick-station striping**, not DC striping (Daniel,
2026-07-30): the stripe map keys on the `Destination` enum (§4.2), whose only
v1 variant is `DistributionCenter(dc_id)` — so future destination kinds
inherit striping without re-plumbing. "DC" below reads as "the v1 destination
kind."

A new, small module (working name `flows/ttp/demand/striping.rs`) computes, at
composition time, from the full demand pool:

- **Station order:** pick stations sorted by physical position along the
  pick-station bank (from `station_positions` / presentation-cell grid
  positions), never by raw station id. #245 explicitly forbids letting id order
  silently become stripe order; v1 derives order from the map and asserts it.
  Precisely (Codex — "physical order" needs a total-order definition): project
  each pick station's presentation-cell position onto the dominant axis of the
  pick-station point set (the axis of larger coordinate variance), order by
  projection, break exact ties by `(x, y)`. Composition asserts the projection
  is strictly monotone — one collinear-ish bank, true for cencora_v3 — and
  errors out otherwise, so a multi-bank layout forces an explicit station-order
  config instead of silently getting a wrong order.
- **DC order:** first appearance in the product sequence — the same order the
  order builder already walks, so stripe adjacency mirrors demand adjacency.
- **Apportionment — exact interval model** (Codex flagged the draft's
  "largest-remainder" phrasing as contradicting the interval description; the
  interval model is the intended one, now stated exactly). Let `U` = total
  units, `N` = pick-station count, `c_d` = cumulative units through DC `d` in
  DC order. DC `d` owns the half-open interval `[c_{d-1}, c_d)` of the unit
  line; station `k` owns `[k·U/N, (k+1)·U/N)`. All overlap tests compare
  `N·c` against `k·U` in u128 integer arithmetic — no floats anywhere.
  **eligible(d)** = stations whose interval overlaps `d`'s with nonzero
  length (a boundary touched exactly does not extend eligibility; a
  zero-volume DC has an empty interval and no stations → fallback path),
  each carrying its exact overlap — the destination's allocation across its
  stations is **proportional to these shares** (D13); there is deliberately
  no "primary station" concept in the interface, only a display-derived
  majority marker in the UI. Given 26 DCs over 10 stations this yields 1
  eligible station for most DCs, 2 for boundary-straddling ones.
- **Exposure:** a read-only handle (`StripeMap`) whose entries carry the
  eligible stations *with their exact shares*, injected into both the
  putaway flow and pick dispatch. This is the interface #245 requires
  ("expose a read-only DC → eligible-pick-stations mapping for the putaway
  location strategy"); dynamic restriping later replaces the constructor, not
  the consumers.
- Zero-volume DCs get no interval (fallback path below). Orders without a DC
  (pre-formed scenarios) bypass striping entirely.

### 4.2 Destination-earmark plumbing — phase 2

**Destination** (nomenclature per Daniel, 2026-07-30): an abstract end-point
inventory is headed for. A DC is one kind of destination, not the concept
itself — the type is an enum (`Destination::DistributionCenter(dc_id)` is the
only v1 variant) so future kinds (a specific station, a wave, an external
order) slot in without re-plumbing. A **destination earmark** is a
`Destination` carried by inducted inventory; the stripe map (§4.1) is keyed
by `Destination` as well.

- `ArrivalItem` gains the earmark — carried from the demand-pool row's DC
  instead of dropped.
- Putaway induction records the earmark **on the inducted inventory itself**,
  not the tote (Daniel, 2026-07-30): the inventory model's content lines
  (`ToteData.content_log` slices of `(SkuId, qty)`) widen to carry
  `Option<Earmark>`, so the tag follows the goods through partial picks and
  any future consolidation. This is a mechanical type-widening; if
  implementation finds it ripples badly, the agreed fallback is a tote-level
  earmark. Induction also puts the DC on its CycleComplete event (turning
  today's hard-coded `dc_id: None` into real data — reporting benefit for
  free).
- The contents' earmark is readable at return time, so `ReturnTotesToRack`
  can hand it to the deposit request.

**Shared SKUs and cross-DC consumption (Codex blocker, verified).** Placement
is DC-aware, but retrieval is SKU-keyed: order building reserves `(sku, qty)`
globally and pick-tote selection searches `ContainsSku` — nothing stops DC A's
order from consuming a tote placed for DC B. In the peak-day pool this is the
dominant regime: 941 of 1,179 SKUs are demanded by more than one DC, covering
**99.4% of units** (verified 2026-07-29). Two mechanisms keep placement
meaningful anyway: pick-tote selection is soonest-arrival, which naturally
prefers the totes near the picking station (i.e., the ones placed for it),
and picking follows the induction sequence, so a tote tends to be consumed in
the same demand neighborhood that inducted it. But these are tendencies, not
guarantees, so v1 **measures** the erosion: each pick records the consumed
inventory's destination earmark alongside the order's DC, and the
**cross-earmark consumption rate** (picks whose inventory was earmarked for a
different destination than the order's DC) is reported. Whether to also *act*
on it — a soft same-earmark preference in pick-tote selection — was decision
D9, settled measure-only (see decision log).

### 4.3 Deposit strategy seam (subset of #246) — phase 2

- A serializable config enum, default preserving current behavior:
  `putaway.strategy = shortest_putaway_time | pick_station_locality`
  (settable per run via `--settings`, printed in the SimConfig banner and xlsx
  Cover, like every other knob).
- `reserve_relay_for_deposit` grows a request context carrying what #246's
  "richer deposit-selection request" names: provenance (putaway-fill vs
  post-pick return), tote id, SKU, and the contents' destination earmark
  (`Option<Earmark>`) — alongside the existing cell/time/robot/pressure/relief
  arguments.
- The score closure becomes strategy-selected, mirroring the retrieve side's
  score-closure seam. Hard feasibility filters are strategy-independent and
  unchanged: empty slot, aisle side, deadlock headroom (N−2), access-cell
  enterability, VTM present in zone.

### 4.4 Locality scoring — phase 2

For a tote whose contents are earmarked for DC `d`, the future-cost term is
the **share-weighted expectation** over `d`'s eligible stations (D13; this
re-amends the draft's `min` — optimistic, per Codex — and the interim
primary-targeted form — lopsided, since allocation is proportional and a
majority-only target would starve the minority side and re-lump station
loads):

```
score(candidate) = predicted_putaway_completion(candidate)   // today's score, unchanged
                 + Σ over s in eligible(d) of
                       share(d, s) · future_retrieve_cost(candidate.slot, s)
```

where — this is the retrieve-shaped form, replacing the draft's
deposit-relay proxy —

```
future_retrieve_cost(slot, station) =
    min over same-side relays r that can serve slot of
        vtm_forward_cycle(slot → r) + htm_leg(r → station)
```

Codex correctly killed the draft's "the retrieve relay is by construction near
the deposit relay" claim: rack slots are indexed under **every** same-side
relay, and retrieval independently minimizes forward-cycle-plus-HTM-leg over
all of them — so the honest future cost must take the same min the retrieve
selector will take, not assume the deposit relay. Both ingredients are static
geometry: `htm_leg(relay → station)` precomputed once (the FPI machinery and
`StationFetchBounds` already walk these paths), `vtm_forward_cycle` is the
existing per-(slot, relay) cycle arithmetic. Both terms are in seconds, so the
sum proxies the tote's total system time — "cost to store it" plus "the
future delivery it commits us to." No tuning constant; a weight can be added
later if A/B runs show the balance is wrong.

Properties worth stating:

- Keeping the full putaway-completion term means congestion still matters: a
  saturated zone near the target station loses to a calmer zone by exactly its
  wait time — locality bias degrades gracefully under pressure instead of
  hammering one aisle.
- The candidate shape stays one-slot-per-relay (cheapest by VTM return cycle).
  That slot pre-selection is putaway-shaped, not retrieve-shaped — a known v1
  approximation, now scored honestly per above rather than assumed away.
- **Determinism on ties** (Codex — deposit candidates come off hash-map
  iteration today, so exact score ties are process-unstable, an existing
  latent issue the current selector shares): candidates are sorted by
  `(zone_id, relay_id)` before scoring, making first-minimum selection stable.
  This fix applies to both strategies.
- **Fallbacks** (all counted, per-reason, in the run summary): contents carry
  no destination earmark, or the earmarked DC has no stripe (zero-volume) →
  the future-cost term is dropped and the
  score degenerates to today's shortest-putaway behavior. An empty feasible
  candidate set is NOT a strategy fallback (Codex — the draft conflated
  these): both strategies share the same hard-feasible set, and emptiness
  keeps today's `NoCandidates` decline-and-retry semantics unchanged. The run
  never fails on a locality miss.

### 4.5 Pick-side stripe eligibility — phase 3+, gated on Q1 (audit first)

The ready-queue scan's station filter gains one clause: an order with a DC is
eligible at a station only if the stripe map says so (orders without a DC:
any station, as today). Config: `pick.stripe = off | strict`, default `off`
(so the pick-side change is independently A/B-able from the putaway-side
change). Already-active orders are never re-bound — structural, see §2.

Starvation safety: the operative quantity is DC diversity **in the ready
queue**, not in the putaway sequence (Codex — the builder drains one feasible
DC deeply before moving on, so hourly putaway interleaving alone proves
nothing). What actually bounds the risk: the refill loop targets a
2,000-order-deep ready queue — roughly the entire ~1,500-order peak-day pool —
constrained only by what putaway has physically landed, and §3's hourly
interleaving means the *inducted inventory* spans DCs at every point. So after
the embargo hour the queue is structurally DC-diverse; the thin window is
early picking over a shallow inducted base. v1 therefore: logs per-station
eligible-order counts, reports station idle-while-queue-nonempty time, and
keeps `borrow_when_starved` (take any order only when no eligible order
exists) as the designed escape hatch — deliberately NOT built until that data
says it is needed.

### 4.6 Diagnostics and metrics

- SimConfig banner + xlsx Cover: strategy name, stripe mode.
- Run summary: fallback counters by reason; stripe table (DC → stations,
  shares) printed once at composition.
- Adherence: the existing DC×Station matrix (xlsx + desvis) becomes the
  compliance view — under strict stripes it should go near-block-diagonal.
  Caveat (Codex): that matrix keys on the *order's* DC, so it cannot see
  cross-earmark inventory consumption — the consumed inventory's
  **destination earmark** in pick records (§4.2) is the complementary view and
  is required, not optional.
- Locality outcome: mean/percentile from-rack HTM delivery leg (HTM Journey
  Legs data already exists), per-zone laden fill spread (already in observer
  state), station starvation gaps (already in Resource Status Report / Station
  Gap Analysis).
- Per-decision auditability (Codex; #246 requires candidate scores/reasons and
  the selected zone/relay/slot): deposit decisions emit trace events —
  strategy, chosen zone/relay/slot, both score terms, fallback reason —
  through the existing `--trace` machinery, so any single placement is
  explainable without bloating default runs; aggregates stay in the run
  summary.

## 5. Expected system effects (assessment, 2026-07-29)

**Pick-station workload balance:** volume-proportional *in expectation*, not
by construction (Codex is right that the draft overclaimed): a straddling DC's
orders are eligible at both boundary stations with no fractional quota, and an
order binds to whichever station's round-robin scan reaches it first — so
~10%-per-station is a strong tendency (largest DC 6.9% caps the lumpiness),
enforced by rotation plus demand pressure, and verified by per-station
starts/picks in the A/B rather than guaranteed. The transient risk (a station
idle because *its* DCs have nothing ready) is bounded by ready-queue DC
diversity per §4.5, and measured. Global hour-to-hour volume swings (14k-unit
hour 6 vs 42-unit hour 18) hit all stations alike, exactly as today.

**Zone inventory balance:** deliberately NOT an objective of this strategy —
inventory will cluster by DC near each DC's stations; that clustering IS goal
1. Three bounds keep it sane: capacity feasibility (full zones drop out of the
candidate set), the retained congestion terms (busy zones lose by their wait
time), and volume-proportional stripes (each station bank's nearby aisles
attract ~its share of total inbound). Net: per-zone fill spread will widen vs
today's putaway-side clustering, in a different geography; we measure it in the
A/B rather than constrain it. If fill imbalance ever becomes a real problem,
#246's strategy 2 (zone inventory-fill balancing) is the dedicated tool — out
of scope here.

## 6. Validation

Baselines at master `14d2c213`, cencora_v3, seed 0, threads 1, headless:

- **A (quick differential):** `--simtime 2h`, `pick.embargo_secs=3600`.
- **B (proper gate):** `--simtime 8h`, embargo 7200 (config default).

**Phase 1 validation:** stripe-module unit tests (apportionment, contiguity,
determinism, zero-volume destinations, straddling, station-order assertion)
plus visual validation of the desvis surfaces against hand-computed stripes
for the peak-day pool — no behavioral gates, since phase 1 changes no
behavior. The 2h baseline xlsx doubles as the completed-picks side of the
planned-vs-completed comparison.

**Phase 2+ A/B matrix** — the full 2×2 (Codex: the draft omitted the
shortest+strict arm, without which strict-routing effects can't be separated
from locality-placement effects), same config otherwise; the strict arms (3
and 4) run only if Q1 resolves toward enforcement:

1. strategy=shortest_putaway_time, stripe=off — (regression) must match
   baseline trace-identically under the same seed.
2. strategy=locality, stripe=off — placement effect alone.
3. strategy=shortest_putaway_time, stripe=strict — routing constraint alone.
4. strategy=locality, stripe=strict — full feature.

The 2h arms additionally run under a second seed (Codex: one seed on one
well-interleaved scenario proves little); the 8h gate runs stay seed 0.
Stripe-starvation edge cases (bursty single-DC demand, zero-volume DCs,
temporarily unpickable stripes — #245's required cases) are exercised by a
small synthetic fixture as automated tests, not by cencora runs.

Gates (Codex: the draft's gates were non-falsifiable; thresholds below are
concrete but explicitly Daniel-tunable at ratification):

- **Goal 2, directly** (Codex: the draft never tested it): pick-completion
  order vs order-start sequence — inversion rate and rank correlation — and
  per-order first-pick dispatch latency, all must improve vs baseline in arms
  2 and 4.
- 8h completed picks ≥ baseline (any regression fails the gate; the
  hypothesis is a gain via VTM timing).
- From-rack HTM delivery legs: mean −10% or better, p95 not worse than
  baseline, in arms 2 and 4.
- Pick-station inter-presentation gaps: p95 ≤ baseline +5%; idle-while-
  queue-nonempty time reported per station (strict arms).
- Putaway completion: mean ≤ baseline +15% (putaway legs lengthen by design;
  the trade must stay bounded and quantified).
- Cross-DC consumption rate reported (§4.2); no gate in v1 — it calibrates
  decision D9.
- No deadlocks, no watchdog trips, zero pre-embargo picks; bit-identical
  rerun under the same seed (ties are now sort-stabilized, §4.4).
- Per-zone fill spread and DC×Station adherence reported (measured, not
  gated).

## 7. Decision log

Settled:

- **D1 — Fixed stripes v1, dynamic later.** Static stripe map computed once at
  composition from the full demand pool, behind the #245 read-only affinity
  interface so dynamic restriping can replace the constructor later.
  *Settled — Daniel, 2026-07-29 ("fixed stripes for now, maybe dynamic stripe
  resizing later").*
- **D9 — Cross-earmark consumption: measure-only in v1.** 99.4% of units are
  in SKUs shared across DCs (§4.2), and retrieval is SKU-keyed, so DC A's
  orders can consume inventory earmarked for DC B. v1 relies on the two
  natural mitigations (soonest-arrival tote choice, induction/pick sequence
  alignment) and *measures* the cross-earmark rate; a soft same-earmark
  preference in pick selection is designed only if the measured rate is high
  enough to erode goal 1 — the earmark is a proxy for nearness, and where
  they disagree, nearness is usually what we want.
  *Settled — Daniel, 2026-07-30, with two amendments: (a) the earmark lives
  on the inventory in the tote, not the tote itself (tote-level only as
  fallback if the content-line widening ripples badly); (b) the abstraction
  is named "destination earmark", DC being one kind of destination among
  future possible kinds — both folded into §4.2.*
- **D10 — The `Destination` enum keys the striping too.** Striping is
  destination pick-station striping, not DC striping; `DistributionCenter` is
  the sole v1 variant (§4.1, §4.2). *Settled — Daniel, 2026-07-30 ("apply the
  Destination enum to that as well").*
- **D11 — Four-phase ordering, visualization first.** Phase 1 striping + UI
  visualization, phase 2 putaway locality + earmarking, phase 3 order/station
  assignment audit, phase 4 pick-station-vs-earmark audit (see Phasing
  section). *Settled — Daniel, 2026-07-30.*
- **D12 — Assigned stripes are a time series from day one.** The stripe
  state is a step function sampled at change events: each (re)computation
  emits a timestamped `StripePlan` event, the observer keeps the ordered
  snapshot log, reporting serves `plan_snapshots`, and the UI renders the
  plan "as of" a temporal selection (initially a slider; superseded the
  same day by the chart-cursor timeline — clicking the docked throughput
  chart slices both matrices at that instant) — exactly one t=0 snapshot under the
  static v1 map, so dynamic restriping later changes the data, never the
  interface. Deliberately NOT the constant replicated into time buckets —
  the event-time snapshot log is the honest series. *Settled — Daniel,
  2026-07-30 ("the assigned stripes needs to be a time series … the UI
  needs to have a temporal component").*
- **D13 — Allocation is proportional to stripe shares; no "primary
  station".** A straddling destination sends work to its stations pro rata
  — that is how volume-proportional station load arises — so phase 2's
  locality score uses the share-weighted expected retrieve cost
  (`Σ_s share(d,s) · future_retrieve_cost(slot, s)`, replacing the
  primary-targeted form D4 briefly carried), phase 3's adherence audit
  compares observed per-station distributions against the shares, and
  `primary` was removed from the stripe map, event, and report types. The
  UI's ● is a display-derived majority marker only. *Settled — Daniel,
  2026-07-30 ("shouldn't orders simply be proportionally allocated to
  assigned stations?"). The ● itself was later removed outright (D14
  round: "not meaningful to the user").*
- **D14 — Chart form and station annotations (phase-1 closeout round).**
  The throughput chart renders step traces with the selected bucket shaded
  across its span. Legend entries are bare destination ids: under dynamic
  restriping a destination's stations change over the run, so any static
  per-line station annotation would lie about part of the series — the
  same rationale rejects coloring lines by station. The planned matrix
  gains the stripe ribbon (cumulative-demand band, destination segments,
  station-boundary ticks). Live pre-completion rendering of the planned
  matrix is deferred. *Settled — Daniel, 2026-07-30.*

Open question:

- **Q1 — Does phase 3 enforce, or only audit?** The pick-side stripe
  constraint (§4.5) was part of the superseded D2 bundle. Daniel's phase 3 is
  worded "auditing of order/pick-station assignment behavior" — audit-shaped,
  not enforcement-shaped. Claude's lean: phase 3 audits natural adherence
  first, and the binding constraint ships only afterward (behind
  `pick.stripe`) if the audit shows natural adherence is too weak for goals
  1/2 — enforcement becomes its own decision with audit data attached.
  *Deferred to phase-3 kickoff — Daniel, 2026-07-30: "we're focused on
  closing out phase 1 now; I don't think enforcement is relevant to
  that."*

Proposed, pending Daniel's ratification:

- **D2 — Scope. SUPERSEDED by the 2026-07-30 phasing** (see Phasing section):
  the draft bundled striping, earmarking, putaway locality, and the pick-side
  constraint into one feature. Daniel re-sequenced: striping + UI
  visualization first (phase 1), putaway usage second (phase 2), audits before
  any binding enforcement (phases 3–4). Still true from the original D2: step
  4 of the Slack plan (bubble sequencing, #163) stays excluded, and the
  putaway-alone-can't-guarantee-goal-1 observation stands — it is now the
  *reason* phases 3–4 audit assignment and consumption before enforcing
  anything.
- **D3 — #246 composition.** Build the strategy enum + richer deposit request
  now; implement only `shortest_putaway_time` (default, regression-gated) and
  `pick_station_locality`. Strategies 2–4 stay open under #246 (T-08's Plan-B
  trigger argues against building them speculatively).
- **D4 — No per-tote station pinning.** Totes carry DC, not a station;
  deposit scores by the share-weighted expected retrieve cost over the DC's
  eligible stations (re-amended by settled D13; the interim
  primary-targeted form and the original min-over-eligible are both
  superseded — see §4.4). With 26 DCs / 10 stations, most DCs have exactly
  1 eligible station, so this equals hard assignment in the common case
  without dead per-tote state. (Revisit if the DC/station ratio ever
  inverts.)
- **D5 — Boundary-drift stance (for the future dynamic phase).** Stripe
  assignment is an induction-time snapshot; stored inventory is never
  relocated; new order starts follow the current map; locality decay is
  measured (from-rack leg metric), not corrected.
- **D6 — Fallback, not failure.** Locality misses fall back to
  shortest-putaway with per-reason counters; a run never aborts on stripe
  policy grounds.
- **D7 — Post-pick returns keep today's behavior in v1.** The locality
  strategy applies to putaway-provenance deposits only (#246 explicitly warns
  about silently changing post-pick return behavior). Extending to post-pick
  returns (whose totes will also be re-picked by their DC) is a natural v2.
- **D8 — Strict-only stripe enforcement in v1.** `borrow_when_starved` is
  designed (§4.5) but not built until A/B data shows idle stations.

## 8. Implementation sketch (feature branch `agent/destination-striping-viz`,
based on `origin/agent/d30-state-family`; never master)

Phase 1:

1. `Destination` enum (sole variant `DistributionCenter`).
2. `striping.rs`: stripe computation + `StripeMap` + unit tests (proportional
   apportionment, contiguity, determinism, zero-volume destinations,
   straddling, station-order assertion).
3. Expose the stripe map + per-destination pick throughput to desvis as
   discoverable report data (per the metrics protocol: producers emit
   descriptors, consumers discover — no hardcoded enumerations).
4. desvis: Destinations tab rename + definition line; "Assigned stripes"
   planned matrix companion in `draw_dc_distribution()` (planned/completed
   never mixed); striping-state vs per-destination pick-throughput
   time-series chart.
5. Synthetic bursty-destination fixture + stripe edge-case tests (#245
   cases).

Phase 2:

6. Earmark plumbing: `ArrivalItem` destination, induction records earmark on
   inventory content lines (`Option<Destination>`), CycleComplete `dc_id`
   (fixes the reporting gap), observer pass-through.
7. Deposit request context + strategy enum + config/CLI + banner/Cover.
8. Locality score (retrieve-shaped future cost, §4.4) + precomputed
   relay→station leg matrix + candidate tie-sort + fallback counters +
   deposit-decision trace events.
9. A/B arms 1–2 per §6.

Phase 3 (shape depends on Q1):

10. Order/station assignment audit surfaces; goal-2 adherence metrics
    (inversion rate, first-pick latency). If Q1 → enforce: pick dispatch
    stripe predicate + `pick.stripe` config + per-station eligible-count/idle
    diagnostics; A/B arms 3–4.

Phase 4:

11. Pick-record earmark field + cross-earmark rate metric +
    station-vs-earmark adherence views.

Per the repo charter, every touched file gets a module doc comment stating its
noun's meaning and purpose.

Out of scope: #246 strategies 2–4, dynamic restriping, inventory relocation,
bubble sequencing (#163), initial-inventory locality (moot here — cencora_v3
starts empty; matters for scenarios with seeded stock, noted for later),
inventory-aware order→station routing beyond the stripe constraint.
