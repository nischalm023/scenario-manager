# EasyLoop station — design v0.1

**Status:** proposal. Vocabulary additions are architect-level rulings; this
document proposes them and does not enact them.
**Date:** 2026-08-17
**Authority read:** `specs/task-model-v2/TWO_LANE_SPEC.md` §2 (the two axes),
§3.1 (`Needs`/`FactRef`), §3.2 (`Park`), §6.3 (claims);
`world-protocol/{SPEC,LEXICON}.md`; the GO product catalog's
`EasyLoop station (low fidelity)` template.

---

## 1. What an EasyLoop is

Two halves that are one machine, and they work differently. Source for
everything in this section: the Vinay/Kalpesh/Daniel call, 2026-08-07.

**Inventory half — conveyor-borne, no robot present.**
An HTM drops an inventory tote onto an L-shaped twin conveyor and *leaves*.
The conveyor carries the tote; **transverse rollers** pull it off to one of
**two presentation positions**, it is presented to the operator, returned to
the conveyor, and eventually collected by a waiting HTM for back-to-store.

> **Daniel:** all of the inventory totes… exit to the side into the
> presentation area, there's… transverse rollers
> **Kalpesh:** Transfers, yeah… correct
> **Daniel:** the totes are temporarily exited from the conveyor and presented
> to the operator, and then returned to the conveyor, recirculated
> **Vinay:** it goes back to the inventory storage area… back-to-store

Inventory recirculation exists but is *"very minimalistic"* (Kalpesh) — a tote
presents about once and goes back.

**Order half — robot-borne, recirculating.**
A box is assembled onto a tray, loaded onto an HTM, and driven into the loop's
order side. HTMs queue in the horseshoe under the safety cover, present, and
**recirculate between the waiting block and the queue until the order is
complete**, then exit.

> **Kalpesh:** if the order is complete, then it will exit, if the order is
> not complete, it will… come back and park somewhere over here, for its turn
> to come back again. So it will keep on recirculating until the order gets
> complete

**The shape: two stations, one conveyor.** An EasyLoop is **two stations
served by one conveyor** — a left and a right. Each is a station in the
protocol's own sense: its own horseshoe queue, its own **operator**, its own
seat and presentation point, its own open orders. The L-shaped twin conveyor is
shared infrastructure feeding both.

**The pairing is a modeling ruling (2026-08-18), not a physical fact.** The
left seat serves the left presentation point; an operator takes a SKU out of
the tote in the seat and puts it into the order presented beside it, **when
given the signal**. Physically an operator may well be able to reach across,
and whether the sides are always paired in practice is **unconfirmed** — the
ruling is that the nuance does not matter at current fidelity, so the simpler
model wins. So in this model a station is one seat, one presentation point,
one operator, one queue, and nothing crosses between the two stations except
the belt.

**Revisit trigger:** if runs show one seat starving while the other queues, or
the site confirms cross-serving, the pairing is the first assumption to
re-open. It is deliberately cheap to relax — the station choice in §5.4 becomes
a (seat, presentation point) choice and nothing else moves.

**"EasyLoop" is never called a station in this document.** The EasyLoop is the
physical unit; the two things inside it are the stations. Using one word for
both is what made an earlier draft of this design unreadable.

That reconciles the counts. Daniel asked whether there is "any variability to
the two and two"; Vinay: *"two presentations, yeah, and two orders, yeah."*
Two of each is **2 stations x (one inventory + one order)**, not two pools of
two. There are correspondingly **two horseshoe queues**, one per station.
Fewer than two stations does not pay for the loop (Vinay); there is talk of
taking the order side to four. Queue depth is 3 — Kalpesh: *"One. 2 and 3. 3
boards can stand there"*, and *"3 waiting points physically."*

**This costs no new noun.** An EasyLoop is an assembly of two stations plus a
conveyor; a station has two seats of different kinds. Nothing needs a name
for "a set" because a set is a station.

Kalpesh's ruling, restated by Daniel and confirmed by both: **"Easy Loop is
always like this, and if it's not like this, then it's called something
else."**

A pick completes only when an inventory tote and an order are present
**together**. That co-presence is the whole reason the machine exists: the
gaps plan puts the box inside ~4 s of the tote, at ~290 tote and ~170 box
presentations per station-hour.

**Out of scope, by ruling:** the exit queue (departure stays
`exit_direction`), and conveyor mechanics — see §5.1.

---

## 2. The decision: one plan, one footprint

Two designs were considered.

**A (chosen).** One plan whose footprint spans both stations' resources,
ratified as one atomic transaction. Co-presence holds by construction: one
plan, one window.

**B (rejected for now).** Two plans coupled by a shared rendezvous resource
and a `Needs` on the partner's arrival fact. More flexible, and it needs the
any-of composition §3.1 explicitly stages.

**Rationale.** The inventory tote is the long pole: it comes off the rack
through VTM → relay → HTM, while an order tote comes from a nearer buffer.
When one side dominates the makespan the other has slack, so binding them
into one atomic unit costs few replans — the objection to A evaporates
exactly when A is cheapest. Revisit if box induction lead time and its
variability (gaps plan, Gap 4) turn the order side into the long pole.

---

## 3. What already exists (do not rebuild)

Design A needs almost no new mechanism, which is the point of choosing it.

| Need | Already provided | Where |
|---|---|---|
| Hold several resources at once, atomically | *"a plan's `footprint` names the resources it occupies. One plan claims many resources."* Ratification is one atomic transaction | TWO_LANE_SPEC §2 |
| Resources of mixed kind in one footprint | *"a VTM, a grid cell, and a relay slot are indistinguishable to it"* | §2 |
| A resource booked twice in one footprint | `ResourceTable::commit` reports each touched resource once, deduplicated, first-claim order | PR #398 fold |
| Two servers, one arena | Accumulation is *"by demand end, never by resource"* — one accumulator per EasyLoop, two resources in the footprint | §2 |
| Choosing the free position | `plan()` reads mutable world state and `Park`s on its next transition; kernel exposes `earliest_free` / `earliest_hold_start` | §3.2, PR #398 |
| A station's parts named as one machine | product-specific things are *"templates composed of fine generic parts"*, dot-namespaced (`go.easyloop`) | world SPEC §2, §6 |

Note what this buys: **the "either position" rule needs no any-of `Needs`.**
`plan()` queries `earliest_free` on both positions and claims the earlier one.
§3.2's placement rule sends it there: *declare exact-epoch prerequisites in
`needs`; for mutable world state, check the level in `plan()` and then `Park`.*

---

## 4. New vocabulary

Three additions. Each earns its row by being a thing a document or a plan can
name, per LEXICON's own bar.

| Term | Meaning |
|---|---|
| **EasyLoop** | The physical unit: an assembly of **two stations** and one shared conveyor. Not itself a station, and never called one here. Kalpesh's ruling: *"Easy Loop is always like this, and if it's not like this, then it's called something else."* |
| **station** (existing) | One of an EasyLoop's two. Carries a horseshoe queue, an operator, one seat, one presentation point, and its own open orders — so it is a **demand end** with its own accumulator. No new noun was needed for "half" or "side": a half of an EasyLoop *is* a station. |
| **seat** | A **fixture-borne** presentation position: a place a carrier sits, presented and worked, with **no robot holding it**. The transverse rollers draw an inventory tote off the belt into a seat; the tote is the only thing there. This is what distinguishes a seat from a `presentation_point`, where the carrier stays on the HTM that brought it — LEXICON's definition (*"where a carrier is presented to an operator"*) covers both, and the claimant is the difference that matters. An EasyLoop has two seats, both on its inventory side; its order side has presentation points and no seats. A seat is a **resource** in the kernel's sense — one exclusive timeline — and is the unit the head of a queue selects among. Distinct from `presentation_point`, which is the world layer's *place*; a seat is that place as a claimable timeline. The vendor's own word: Map Creator ships `pps_seat.json` beside `pps_bin.json`, and LEXICON prefers spoken warehouse vocabulary to invented terms. The word carries the distinction: a carrier *sits* in a seat, unattended. Rejected: "berth" (invented, and nobody in the domain says it), "bay" (taken by racking), "slot" (taken by storage; "presentation slot" is what the 2026-08-07 call actually said, but the compound is clumsy as a resource kind), "stand"/"post" (61 and 129 existing references), "server" (queueing-theory jargon). |
| **collection cell / dropoff cell** | The two cells an HTM occupies to hand a carrier to or from the conveyor. **Named from the robot's point of view**, which is the convention the codebase already uses (`loaded_speed` is documented as *"loaded (fork up)"* — the robot is what is loaded). So the **dropoff cell** is where an HTM unloads its tote onto the belt, and the **collection cell** is where an HTM loads a returning tote off it. Read from the conveyor's point of view both names invert, so the convention has to be stated once and kept. Each is an `Exchange` cell in `desgraph` terms — *"a place an HTM occupies, not a shelf"* — and each carries occupancy: one carrier at a time, so a queue can form behind it. Note the real-world collision: "collection cell" is a term of art for a weight transducer. Harmless internally, worth avoiding in customer-facing prose; "loading cell"/"unloading cell" is the escape if it ever matters. |
| **coupling** | The constraint that an inventory carrier and an order carrier be presented within one window at one demand end. A coupling is a property of a **plan**, not of a station: it is satisfied by the plan's footprint spanning one inventory **seat** and one order **presentation point** in one window — the two are different kinds of thing, which is the point. Which station's pair is the plan's one choice; within a station the pair is fixed by the §1 pairing ruling. Which station is a scheduling choice; which order presentation point pairs with which inventory seat is not — the pairing is structural, one operator working both. Rejected: "rendezvous" (implies two independently-scheduled parties meeting, which is design B), "sync" (a verb pretending to be a noun), "pairing" (reads as a fixed association between two things rather than a per-plan constraint). |
| **pacing chain / slack chain** | Of a coupled plan's two chains, the one whose makespan sets the window (**pacing**) and the one scheduled to it (**slack**). For the EasyLoop the inventory chain paces and the order chain has slack; the design does not hard-code which, because a slower box supply reverses it. Not new nouns: `chain` is already the codebase's word for a pick's robot journey (`VtmForward → HtmPickup → HtmDeliver → Presentation`), and `pace` is already idiomatic (`pick.pace_to_station`). |

**Deliberately avoided:** **"lane."** `TWO_LANE_SPEC` owns it for a different
axis entirely — lane 1 is sync resource work, lane 2 is async orchestration.
Calling the inventory and order chains "lanes" would collide with the model's
most load-bearing distinction. **"anchor"** is likewise avoided: 110
references in the tree already mean slot-to-cell anchoring.

**Not renamed:** the person is an **operator**, not an "associate". Walmart says
associate and the domain word is fine in prose, but `OperatorConfig` and 88
references already name this role, and LEXICON's rule is one term per concept.
An operator is now a **resource**: one per station, exclusive while picking,
which is what makes two stations worth twice one.

**Not modelled, but named:** **the signal.** The operator picks *when given the
signal* — pick-to-light, the vendor's `pptl.json`. It is the event that starts
the pick dwell, not a resource, and it is absent from every Map Creator dump
seen so far (`files_absent`). Named here so that when it does arrive nobody
mistakes it for new physics.

**Not renamed:** `BinHold` stays as it is. A BinHold is the *order's* slot at a
station, held from order open to its last pick. A seat is held for *one
presentation*. Different granularity, different lifetime, and conflating them
would be a modelling error.

---

## 5. The model

### 5.1 The conveyor is latency, not topology (standing ruling)

> **Daniel:** someday we'll model the conveyors themselves. But for now… we
> would have to just model the **latency**, and they would sort of
> **disappear, and they would reappear at a later time**.

So the inventory half is:

```
dropoff cell  →  [latency]  →  inventory seat  →  [latency]  →  collection cell
HTM unloads      transit      presented, no                    HTM loads
(unload_time)                 robot present                    (load_time)
```

The tote vanishes at the dropoff cell, reappears at a seat after a transit
delay, vanishes again, reappears at the collection cell. The catalog template already
implements the delay with `loop_transit_secs`, whose own comment calls it *"a
product-specific timing abstraction standing in for the L-shaped twin conveyor;
replaced by conveyor entities when fidelity rises."*

**The handoffs are relay-shaped, and are modelled as such.** The POT ledger
describes a conveyor port as *"the cell a robot stands on, the handoff height
and the heading"* — the same three parts as LEXICON's relay (*"one handoff
shelf where carriers change hands between the rack system and the floor"*) with
its access cell (*"the one cell an HTM occupies to exchange with a relay"*).
The only difference is the counterparty: a belt rather than a VTM.

This matters for one reason above tidiness: **as ports these handoffs have no
timeline, so inventory-side queueing is free.** As relay-shaped resources they
hold one carrier at a time and a queue can form behind them. At ~290 tote
presentations per station-hour across two stations onto one belt, the single
dropoff cell is a plausible bottleneck, and under a port it is invisible by
construction.

**Spec tension to settle:** LEXICON has a relay *"derived by its rack, one per
relay level per position."* A conveyor has no rack. Either that loosens to
"derived by its generator" — the same mechanism with a conveyor as generator —
or the conveyor handoffs are a sibling that reuses relay timing without being
relays. This design assumes the former: a relay means "handoff between a
transport system and the floor", and that the transport system has been a rack
until now is contingent, not definitional.

**The consequence that matters: the HTM detaches.** Between ingress and
egress no robot is holding the tote and no robot occupies the presentation
position. This is unlike every other station in the simulator today, where a
pick chain holds its HTM at the station through the presentation dwell. It
should free HTM capacity substantially, and it means the inventory half's
presentation resource is claimed by a **carrier**, not by a robot — which the
kernel does not care about, since *"a VTM, a grid cell, and a relay slot are
indistinguishable to it"* (§2).

**Handoff timing, and the two reasons a pick-up looks slower.** Keep them
apart; conflating them hides the thing worth measuring.

- *The action is slower.* Lifting off a belt versus setting down is a constant:
  `htm.unload_time_secs` at the dropoff cell, `htm.load_time_secs` at the load
  cell. Asymmetry is a config value, and the fields already exist.
- *The HTM waits for a tote.* That is **not** handoff time. It is a hold on a
  resource that is not ready — variable, load-dependent, and precisely what the
  simulation exists to measure. It must **emerge** from loop transit plus
  presentation dwell plus recirculation, never be folded into a constant. Same
  discipline as the D30 wait taxonomy: journal the hold with a cause rather
  than absorbing it into a duration.

If an egress constant has to be tuned to get plausible numbers, that is the
signal the belt has become a resource and the latency ruling has expired.

**Note on today's code:** `htm.load_time_secs` and `htm.unload_time_secs` exist,
default to 2.0 s, and **nothing reads them** — the only consumer of that family
is `composition.rs:430`, which uses the VTM pair. HTM-side relay timing comes
from `fork_ms` instead. So these cells give those fields their first consumer,
and adding them will *lengthen* inventory chains relative to today. That is a
correction rather than a regression, but any pinned figure that moves will move
for this reason and should be attributed, not absorbed.

### 5.2 Resources

Per EasyLoop, two stations, two seats each:

```
shared:        dropoff cell     <- HTM -> belt handoff, claimed by an HTM
               collection cell       <- belt -> HTM handoff, claimed by an HTM
station left:  seat            <- fixture-borne,       claimed by a TOTE
               present         <- horseshoe head,      claimed by an HTM
               operator        <- exclusive while picking
station right: seat
               present
               operator
```

Eight `ResourceId`s, each an exclusive timeline. Plus the existing chain
resources: cells, relay slots, the VTM, the HTM — but on the inventory side the
HTM leaves the footprint at the dropoff cell (§5.1).

A station's three resources are claimed together and never cross to the other
station — by the §1 pairing ruling, not by geometry. The dropoff and collection cells are
the **only** shared resources, since there is one conveyor with one
ingress-egress pair (Daniel, confirmed on the call:
*"there's an ingress-egress pair for the conveyor itself"*). They are therefore
the loop's only genuine point of inventory-side contention.

The **conveyor itself is not a resource** under the latency ruling: it is a
delay between the dropoff cell and a seat. If loop capacity ever binds the
ruling has to be revisited and the conveyor becomes a counted resource — and
§5.7 computes how close to binding it already is at design load with the
placeholder capacity of 10.

### 5.3 Two demand ends, one conveyor

**Each station is a demand end**, with its own accumulator: it has its own
queue, its own operator, and its own open orders. The loop is not an arena.

The shared conveyor does **not** make it one. Which station a tote is pulled
off to is a *resource* question — which inventory seat is free — resolved in
`plan()` by `earliest_free`, on the resource axis. Which station an order
competes at is a *demand* question, on the demand axis. §2 is explicit that
these are independent and must not be conflated; a shared conveyor is exactly
the kind of correlation that tempts one to merge them.

### 5.4 Planning a coupled pick

```
plan():
  1. select the order line (existing admission: bins, queue_factor, sequencing)
  2. quote the PACING chain — inventory tote, rack → relay → HTM → the
     UNLOAD CELL (+ htm.unload_time_secs), then + loop_transit_secs to reach
     a presentation seat. The HTM is released once unloaded; it is NOT in
     the footprint beyond the dropoff cell.
  3. st := the station (left or right) whose seat is free earliest at that
     arrival AND whose operator is free AND which has an order ready to
     couple with. This is the ONLY choice in the plan.
  4. st fixes everything else: its seat, its presentation point, its
     operator. The pairing is the §1 ruling — left serves left — so there
     is no second choice to make. Quote the SLACK chain: an order HTM already in st's
     horseshoe, or one dispatched to it, reaching st's presentation point in
     the same window. Slack by assumption, so it is scheduled TO the pacing
     arrival.
  5. footprint := pacing-chain resources up to and including the dropoff cell
                ∪ slack-chain resources
                ∪ { st.seat, st.present, st.operator }
     The COLLECTION cell is deliberately absent: back-to-store is a separate
     plan (§8), not part of the coupled pick.
  6. one window covering both presentations
  7. return the plan; ratification commits the whole footprint atomically
```

Step 2's split is the novelty: the inventory chain's *robot* resources end at
the dropoff cell, while its *carrier* continues on latency alone to claim the chosen station's seat.
One plan still, one footprint, one window — the HTM simply is not in the part of
it that spans the presentation.

Step 3 is where "head of queue takes whichever is free" lives. It is a read of
mutable state inside `plan()`, then a `Park` on the next transition of both
seats if neither is free — never an any-of `Needs`.

### 5.5 Why co-presence holds

Because there is one plan and one ratification. Either the seat, the
presentation point and the operator are all claimed in the shared window, or the
plan is not ratified at all. There is no state in
which the tote is presented and the box is not, and no separate constraint to
enforce or check.

---

### 5.6 One coupled pick, told end to end

Every term below is defined above. If the story needs a word that is not there,
the vocabulary is wrong.

An HTM carrying a tray and an empty box drives into the **left station**'s
**horseshoe queue** and comes to rest on its **presentation point**. The left
station is a **demand end** — its own operator, its own open orders — and one of
those orders wants SKU-4471.

The planner quotes a **pacing chain** for it: a VTM lifts the tote holding
SKU-4471 off the rack onto a relay, an HTM takes it from the relay and drives to
the **dropoff cell**, the one place where the floor hands carriers to the belt.
Then it picks a station, and that is the only choice in the whole plan. The left
station's **seat** is free earliest, its **operator** is free, and it has a ready
order — so left it is. Everything else follows by the pairing ruling: that
seat, that presentation point, that operator, because in this model left
serves left, full stop. The **slack chain** is short, since the
order HTM is already parked on the presentation point.

One **footprint** spans the pacing chain up to and including the dropoff cell, the
slack chain, and the left station's seat, presentation point and operator, in one
window. **Ratification** commits it atomically. There is no state in which the
tote is presented and the box is not, or in which both are presented and nobody
is there to move the SKU. That is the **coupling**, and it holds by construction.

Playback: the HTM reaches the dropoff cell, spends `htm.unload_time_secs` setting
the tote on the belt, and **leaves**. Nothing in the loop holds it. The tote
rides for `loop_transit_secs`; the transverse rollers draw it off into the left
seat, where it sits unattended — which is what makes it a seat and not a
presentation point. **On the signal**, the operator lifts SKU-4471 out of the
tote and puts it in the box beside it.

The tote returns to the belt and rides to the **collection cell**, where a waiting HTM
spends `htm.load_time_secs` lifting it off and carries it **back-to-store** — its
own plan, deliberately outside the coupled footprint.

The order HTM decides last. If the order is complete it exits the EasyLoop. If
not, it recirculates through the **waiting block** into the horseshoe queue and
waits for its next coupling. Meanwhile the right station has been doing all of
this in parallel, sharing nothing but the belt.

---

### 5.7 Model inputs (external), and the belt-capacity tripwire

What the loop model consumes from outside, with status. Ruling (2026-08-18):
**the timing values can wait** — defaults and placeholders carry the current
fidelity — but belt capacity is recorded now because the arithmetic below makes
it near-binding at design load.

| Input | Home | Value | Status |
|---|---|---|---|
| `loop_transit_secs` | template param | 45 | placeholder; **circuit vs per-leg unresolved — see below** |
| Belt capacity, in totes | **new** template param `belt_capacity` | **10** | placeholder — Kristina, recalled, 2026-08-18, **unconfirmed** |
| Transverse roller transfer | folded into transit | — | can wait |
| `htm.{unload,load}_time_secs` | config | 2.0 s | defaults, unvalidated; can wait |
| Operator pick dwell | `time_per_pick_secs` | 9 | can wait |
| Signal (pptl) latency | not modelled | 0 | assumed zero, stated |

**Calibration targets, not inputs** (Vinay, 2026-08-07 call): order tote cycle
~15–20 s, inventory tote cycle ~35–50 s. The model is roughly right when it
*produces* these.

**The tripwire, computed.** §5.1's latency ruling expires if the belt fills.
Little's law at the gaps-plan target (290 presentations/station-hr × 2 stations
= 580/hr onto one belt, 9 s seat dwell):

| If `loop_transit_secs` = 45 means… | Totes on belt | vs capacity 10 |
|---|---|---|
| the full circuit (out + back) | ~7.3 | fits, ~25% headroom |
| one leg (45 out + 45 back) | ~14.5 | **binds — ruling void at design load** |

So with capacity 10, the leg-vs-circuit question is not a timing nicety: it
decides whether the conveyor can stay a latency at all. **Ask Kalpesh both
numbers together** — belt capacity and per-leg transit — since either alone
cannot settle the tripwire.

**How capacity enters the model now:** measurement first, constraint second.
The observer counts totes in flight on the belt and journals a warning whenever
the count exceeds `belt_capacity` — the latency ruling visibly violated, never
silently. Only if runs actually trip it does the belt become a counted
resource (in kernel terms: `belt_capacity` interchangeable `ResourceId`s,
claimed like seats — a pool is just N exclusive timelines). Promoting it
earlier would be modelling a constraint we have not yet observed binding.

---

## 6. How a document says it

No core world-protocol change. The EasyLoop is a template whose
`go.easyloop` component names its parts — vendor-namespaced, so this costs
nobody else a negotiation (world SPEC §2: *"A consumer MUST ignore components
it does not understand"*).

```jsonc
"go.easyloop": {
  "inventory_station": { "local_ref": "inventory_station" },
  "order_station":     { "local_ref": "order_station" },
  "seats": {
    "inventory": [ { "local_ref": "present:inv:0" },
                   { "local_ref": "present:inv:1" } ],
    "order":     [ { "local_ref": "present:ord:0" } ]
  },
  "coupling": { "presents": [ { "kind": "tote", "designation": "inventory" },
                              { "kind": "tote", "designation": "order" } ] },
  "loop_transit_secs": { "$param": "loop_transit_secs" },
  "time_per_pick_secs": { "$param": "time_per_pick_secs" }
}
```

The template gains a second `station` local (order), a third
`presentation_point` local, and a queue of depth 1 on the order side. The
existing `order_queue` becomes the inventory station's entrance queue, and the
`waiting_block` stays as it is.

One known impedance remains and is not solved here:

- **`presentation_point` carries only `for`.** It cannot state a seat's
  height, presenting class, heading, or which side an operator reaches from.
  Two seats can be declared; **front-versus-side cannot be said.** This is
  the S1 schema gap and it blocks fidelity, not this design.

A queue here is the two-entity form: a lane, an entity of its own carrying
`queue` whose path ends at the cell it feeds. dessim's loader reads a queue no
other way, and refuses `queue` on a station.

---

## 7. What must change in code

Measured on the stack head.

| # | Change | Size | Note |
|---|---|---|---|
| 1 | `station` resolves queues that reference it, not only a `queue` on itself | S | **Blocker.** `worldmap/stations.rs:169` reads `queue` off the station and silently yields zero cells otherwise |
| 2 | `Station.presentation_cell: CellId` → a collection of seats | **L** | **153 references across 52 files.** Most assume "the" presentation cell |
| 3 | Seat selection at the head of the queue (`earliest_free` on both, else `Park`) | M | New behaviour; no existing analogue with one cell |
| 4 | A station carries order totes: a third `StationType`, or a designation on the station | M | `StationType` is exactly `{Pick, Putaway}` |
| 5 | Coupled plan: one footprint spanning both chains, one window | M | On the tmv2 kernel this is composition, not new machinery. On today's oracle it is a real change |
| 6 | `presentation_point` gains geometry, incl. a station reference | S | S1; also unblocks front/side |
| 7 | Load and unload cells as relay-shaped handoff resources with occupancy | M | Needs the §5.1 ruling on relay derivation. Gives `htm.{load,unload}_time_secs` their first consumer |
| 8 | Back-to-store as its own plan, triggered by a tote reaching the collection cell | M | Keeps it out of the coupled footprint; the loop's exit path |

(2) is the one to do carefully behind a green suite. (1) is the one that makes
any of it observable.

---

## 8. Failure modes

| Situation | Behaviour |
|---|---|
| Neither inventory seat free | `plan()` returns `Park::Until` on both seats' next transition. No partial claim |
| Order seat occupied, inventory free | Same: the coupling is in one footprint, so nothing is claimed |
| Order tote unobtainable | The plan does not form. The inventory tote is never dispatched, so no tote strands at a seat waiting for a box |
| Unload cell busy | The pacing chain's HTM queues for it, exactly as it would for a relay. Visible as inventory-side contention rather than absorbed into transit |
| Nothing waiting at the collection cell | The back-to-store plan does not form; no HTM is dispatched to idle there. The wait is the absence of a plan, not a held robot |
| Conflict at ratification | The whole footprint fails and both chains replan. Accepted cost of A; cheap while inventory paces |
| Order supply becomes the long pole | Pacing/slack swap. If replan churn becomes visible, that is the signal to revisit design B |

The third row is the real prize: today a coupled pick cannot half-happen,
which is what "synchronized" has to mean.

---

## 9. Sequencing, and the risk worth naming

The tmv2 kernel that makes (5) trivial is **dormant**. PR #398 is explicit:
*"a dormant landing: new machinery, nothing wired"*, discharged structurally
because no symbol outside `kernel/` names it. Issue #240's board puts the
ladder at PR 3 of 6, then 5a–5e, then shadow mode, and then:

> Phase 5 — first authority slice: **station presentation windows**

Station presentation windows *are* this design's home, and they are several
phases out from holding authority.

So there are two honest paths:

- **Interim on today's oracle.** Build (1)–(4) now, express (5) against the
  current reservation oracle, and accept that it is rewritten at the cutover.
  Delivers a working EasyLoop on the Walmart timeline.
- **Wait for Phase 5.** Right home, no rework, blocked behind the ladder.

This design is written so the interim can use the same vocabulary the kernel
will: seat is a `ResourceId`, a coupling is a footprint plus a window. The
migration should then be mechanical rather than a redesign.

**Recommendation:** interim, with (1) and (6) done first because they are
small, are needed either way, and (1) is what makes the current two EasyLoops
in `walmart_dfw6.json` more than cosmetic.

---

## 10. Open questions

**Answered 2026-08-18.** Each station has its own operator, so the two stations
are independently worked and loop throughput is ~2x a single station; the
operator is a resource in the footprint. Left-serves-left pairing is adopted **as
a ruling** (see §1) — physically unconfirmed, deliberately cheap to relax.
Remaining:

1. **At the collection cell, does the SAME HTM wait for its tote, or does any
   available HTM collect it?** The template says *"a waiting HTM receives the
   returned tote"*, which reads as any, and nobody said it out loud on the
   call. It decides most of the value of the detach: if the same robot must
   wait out a full loop transit, two robots are parked per loop and the
   capacity saving largely evaporates.
2. **Does the order tote leave with the inventory tote, or persist across
   several picks?** If it persists it is closer to a BinHold than a seat
   occupancy, and the coupling window is per-pick while the seat hold is
   per-order.
3. **Seat-to-queue-cell binding.** SPEC §10 leaves explicit binding open and
   v0 binds by proximity. With two seats behind one queue, proximity may pick
   the wrong one; an explicit binding is probably wanted.
4. **`StationType` or designation** for (4). A third variant touches every
   match on the enum; a designation field is additive but leaves `Pick` meaning
   two different things.
5. **Does the exit ruling hold under load?** Dropping the exit queue assumes a
   departing HTM never blocks a seat. If exit congestion backs into the
   seats, the ruling produces optimistic throughput and nothing will say so.
