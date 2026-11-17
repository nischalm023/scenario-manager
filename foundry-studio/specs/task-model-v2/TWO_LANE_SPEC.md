# Two-lane task model — specification v2.1

**Status**: v2.1 (2026-07-20) — v2 (both adversarial reviews + rebuttal
folded) plus Daniel's design-review rulings: payload module registries
(log 4c), the `Demand`/`Supervisor` lane-2 split (4d), async_trait
wherever trait methods return futures and **no alias types** (4f), and
**the generic `Domain`/`App` trait EXCLUDED** — the kernel lands concrete
against warehouse types (4g; one real application exists, so the
abstraction was speculative; the extraction path is a documented standing
item, stop-short by default). With 4g, round-2's T1 (generic time) is
retracted under the same deferred premise: the kernel uses `SimTime`
directly (coding standard §11); the four time role-types carry the
temporal discipline instead. Review provenance:
`reviews/two-lane-claude-review.md`, `reviews/two-lane-sol-review.md`.

**Ratification ladder** (three distinct gates):

1. **Factorization**: proceeding under Daniel's direction; he reserves
   questions at each phase — `PLAN.md`'s punchlist is the ratification
   vehicle.
2. **Trait/API for implementation**: this text survives one
   paper-adversary pass against the repaired traits (scheduled before
   PLAN PR 3).
3. **Production authority** (per migration slice): stamped reads (§6.9
   spike), demand licensing live (§6.4), vertical-slice boundary
   (`PLAN.md`), credit/obligation audits (§6.7–6.8).

**Governing principle** (Daniel, binding): *conventions that task authors
must remember are not an acceptable abstraction contract* — every
obligation is a type, a kernel-enforced rule, or a named entry in §10.

---

## 1. Purpose

Preserved from the round-4 evidence: plan-eager / commit-lazy scheduling
over reservation timelines; tentative plans competing in accumulators;
atomic ratification; facts (scheduling truth) distinct from playback
(physical truth, the only mutator); typed reports; bit-identical
determinism available. Changed from the winner's authoring surface: no
async on the resource path; no templates; declarative dependencies; async
confined to orchestration under scoped capabilities. The story and
provenance: `README.md`. Why regimentation: the conventions had a body
count (the 30 GB grant-retention class, Sol findings 3/5, expressible E7
violations) — a wrong task should be inexpressible, not detectable.

## 2. The substrate — and the two axes (read this first)

Every **resource** is an opaque `ResourceId` carrying one uniform
timeline semantics: half-open **windows**, open-ended owner-tagged
**holds** (parties = physical identities; holds survive tasks; successors
close them atomically — T6), per-resource **generations**, prune+`floor`.
The kernel does all resource-specific machinery — overlap validation,
invalidation fan-out, earliest-free queries, pruning — identically on all
of them: a VTM, a grid cell, and a relay slot are indistinguishable to
it. (This resource-blindness never depended on a generic trait; resources
were only ever ids.)

**THE TWO AXES — do not conflate them:**

- **The resource axis (claiming)**: a plan's `footprint` names the
  resources it occupies. One plan claims many resources.
- **The demand axis (competing)**: a plan's `home` names the
  **accumulator** — the demand end's menu — in which it competes for
  selection. Accumulation is **by demand end, never by resource**: a
  station's arena, a zone's end, today's single flow arena. The axes
  often correlate (a zone end's plans mostly claim that zone's VTMs) but
  are deliberately independent — which arena a plan competes in is a
  demand-licensing choice (§6.4), not a property of what it touches.
  This resolves the original per-resource-accumulator question
  (Connor call, 2026-07-13: "instance or type?" — answer: neither;
  per demand end).

Lifecycle: plans are **tentative** until a demand pop selects them;
**ratification** is one atomic transaction; the clock advances only at
quiescence; **playback** realizes committed physics and is, with
commit-apply, the only world mutator. Narrative walkthrough: the
dossier's `bakeoff/round4/MODEL.html` (PR #235).

**Concrete kernel (4g)**: the kernel names `WhFact`/`WhEffect`/`WhCond`/
`WhPlayback`/`WhWorld`/`SimTime` directly. The former I8 enforcement
("the kernel cannot open payloads") becomes a CI lint — *no `match` on
payload types inside `kernel/`* — plus §10 entry 5. Extraction of an
`App` trait is a standing item IF a second real application ever
materializes (the production runtime); until then, stop-short is the
default.

**Payload organization (4c — the DI completion)**: effects and playbacks
never cross modules → they are **module-local**, each demand module
registering its applier/player in a deterministic startup registry
(effects remain inspectable data; the round-1 apply-as-closure conviction
stands). Facts and conditions are **interfaces**: a small shared
material-flow vocabulary (`Deposited`, `PickedUp`, `ArrivalQuote`,
`CellFreed`, …) plus module-local types with typed access. A new demand
module = one crate + one rig registration; zero central edits.

**Snapshot cost clause**: planning batches read frozen per-batch views
whose construction cost is O(delta since last batch), never O(state) —
the winner's CoW evidence (113–249× vs deep clone) is the bound's
provenance, not a mandated implementation. A standing regression probe
guards the bound.

## 3. Lane 1 — `Task`: resource work (sync, kernel-driven)

The clients of the substrate: anything that claims timeline space.

```rust
/// A unit of schedulable work. Constructed directly by its creator with
/// real dependency ids (§5). Kernel-driven lifecycle:
///   needs satisfied → plan() → [tentative → pop → ratified] →
///   post-commit: on_commit() → Done | Continue(next needs) → plan() …
pub trait Task: Send + std::fmt::Debug {
    fn label(&self) -> String;

    /// Declarative gate: what must exist before plan() is FIRST called
    /// (re-armed by Continue). Facts and time only — level-safe by
    /// construction; world-condition gating is §3.2.
    fn needs(&self) -> Needs;

    /// One PURE planning attempt — pure BY SIGNATURE (&self): a failed
    /// or invalidated attempt structurally cannot mutate the task, so
    /// replay and retry order cannot alter future candidates. Called
    /// only when needs hold; re-called on wake/invalidation with the
    /// typed cause in ctx. Output is the only channel.
    fn plan(&self, ctx: &PlanCtx<'_>) -> Result<Plan, Park>;

    /// The ONLY mutation point, run in the deterministic POST-COMMIT
    /// phase (after the transaction, same instant, before any newly
    /// woken task is polled — §6.6). Receives the exact committed fact
    /// and task-local state only. Defaults away for single-increment
    /// kinds.
    fn on_commit(&mut self, fact: &WhFact, sp: &mut SpawnCtx<'_>) -> Next {
        let _ = (fact, sp);
        Next::Done
    }
}

pub enum Next { Done, Continue(Needs) }
```

**The round-2 boundary, cited on purpose**: R2-B was convicted for a
`plan`/`on_commit`/`take_wait`/`done` callback protocol (round-2
scorecard; SCORING §3a floor-scores the shape by name). This trait stays
on the right side because every convicted element is absent: no
wait-stash (`Park` is plan's return value), no pulse-stash (the fact is
an argument, delivered once), no author-side retry lattice
(kernel-owned), and `on_commit` defaults away — a single-increment kind
is `needs` (data) + one pure `plan` fn. The **routing rule** for richer
lives is semantic, not numeric: *one persistent entitlement/owner across
commits → a multi-increment Task (subject to the §12 cold-read gate);
narratively distinct increments → a lane-2 supervisor spawning one-shot
children.*

### 3.1 `Needs` and `FactRef`

```rust
/// All-of semantics. Level/edge condition leaves and any-of composition
/// are STAGED (they arrive with the windowed program as one atomic
/// sample-and-arm settle operation); v2.1 ships exactly this.
pub struct Needs {
    pub facts: Vec<FactRef>,
    pub after: Option<Deadline>,        // pacing floor; §7 time types
}

/// Epoch-addressed fact reference (multi-increment producers commit one
/// fact per epoch; "latest" is racy and does not exist here).
pub enum FactRef {
    Exact { task: TaskId, epoch: Epoch },
    NextAfter { task: TaskId, epoch: Epoch },
    Terminal { task: TaskId },
}
```

Wake resolution CAPTURES the satisfying `(producer, epoch, value)`; that
exact value is what `PlanCtx::fact(ref)` returns. Committed epochs are
retained until no live reference can name them. A producer that retires
without satisfying a reference resolves the wait as a typed
`ProducerRetired` cause (§3.3) — never a forever-park.

### 3.2 `Park` — the computed gate

```rust
pub enum Park {
    /// NON-EMPTY wake set (empty = kernel fault — the armless park,
    /// round-1 convergent finding 4). Versions stamped at park.
    Until(Vec<WhCond>),
    /// Wake at this instant (Deadline or CommittedAt — never a Quote).
    After(Deadline),
    Fault(Fault),
}
```

The placement rule (stated once, teachable): *declare immutable or
exact-epoch prerequisites in `needs`; for mutable world state, check the
level in `plan()` and then `Park` on its next transition.*
Wrong-direction errors are loud: an undeclared `ctx.fact` is a fault; a
computable gate cannot be declared.

### 3.3 `PlanCtx` — the read surface and the cause

```rust
impl<'a> PlanCtx<'a> {
    pub fn now(&self) -> CommittedAt;
    pub fn fact(&self, r: FactRef) -> &WhFact;      // captured value; undeclared = fault
    pub fn snapshot(&self) -> &ReservationSnapshot; // frozen committed truth (§2 cost)
    /// Typed, source-scoped, STAMPED domain read oracles (§6.9). No raw
    /// &WhWorld in the authoring API — decision-bearing reads carry
    /// stamps that join the plan's read set, validated at ratification.
    pub fn oracle(&self) -> &Oracles<'a>;
    pub fn cause(&self) -> &PlanCause;              // deterministic SET, not latest-wins
}

pub enum PlanCause {
    Initial,
    Woke(WakeSet),
    Invalidated(Vec<Invalidation>),
    ProjectionRejected(ProjectionError),
    ProducerRetired { producer: TaskId, last_epoch: Epoch },
}
```

## 4. Lane 2 — orchestration: `Demand` and `Supervisor` (async, capability-scoped)

Async lives where narrative reads matter and its dangers don't: these
contexts simply lack the resource verbs (no propose, no reserve, no
commit). The lane splits by the one power that differs — `fire` — and by
quiescence semantics (ruling 4d):

```rust
/// The DemandModule successor: standing articulators of demand (books,
/// pumps, feeds). The ONLY holders of fire/DemandHandle. Idle-at-
/// quiescence is benign BY TYPE (no is_daemon flag exists).
#[async_trait]
pub trait Demand: Send + std::fmt::Debug {
    fn label(&self) -> String;
    async fn run(self: Box<Self>, ctx: DemandCtx) -> Result<(), Fault>;
}
// DemandCtx: spawner() · fact(FactRef).await · after(Deadline).await ·
//            view() -> SupervisorSummary · fire(&DemandHandle).await

/// Parents/orchestration (an OrderLine supervising its chain). No fire.
/// Parked supervisors ARE counted at quiescence.
#[async_trait]
pub trait Supervisor: Send + std::fmt::Debug {
    fn label(&self) -> String;
    async fn run(self: Box<Self>, ctx: SupCtx) -> Result<(), Fault>;
}
// SupCtx: spawner() · fact(FactRef).await · after(Deadline).await · view()
```

Capabilities are rig-minted VALUES (open-world in kind, bounded in
authority): `SpawnPermit { root, claim_scope, demand_scope, budget }`
scopes a spawner; `DemandHandle` is minted only to `Demand` impls, with
its ends and fairness class registered (§6.4). `view()` returns a sealed
summary DTO — no table, service client, or snapshot-grade oracle can ride
in it. **Sealed awaitable surface**: the adapter faults any `Pending`
without a kernel-owned suspension token — awaiting a foreign future is
the armless park by another name, detected not hung. Async
selected-candidate resource preparation is deliberately unsupported.

Per ruling 4f: `#[async_trait]` is the spelling wherever trait methods
return futures, and there are **no alias types** for future/return
shapes.

## 5. Construction and identity (templates deleted)

```rust
impl<'k> SpawnCtx<'k> {
    /// Reserve a real TaskId BEFORE construction (sibling cross-refs);
    /// reserved ids are transaction-scoped — they cannot escape a
    /// failed batch.
    pub fn reserve(&mut self) -> TaskId;
    /// Construct-and-register (validated: every referenced TaskId must
    /// be live, reserved-in-batch, or committed).
    pub fn spawn(&mut self, task: impl Task + 'static) -> TaskId;
    pub fn spawn_demand(&mut self, d: impl Demand + 'static) -> TaskId;
    pub fn spawn_supervisor(&mut self, s: impl Supervisor + 'static) -> TaskId;
}
```

- **Identity is hierarchical, order is not**: `TaskId = (parent, seq)`
  in structured storage (depth/width limits at rig config) for identity,
  the task tree, and prefix cancellation; **scheduling order is a
  separate kernel-assigned monotonic registration index** (apply order,
  registration-order policy) — reparenting cannot silently change
  simulation policy.
- `SpawnCtx` is exclusive and non-Send (same-parent seq allocation
  cannot race).
- Spawning is legal in `Demand::run`/`Supervisor::run`,
  `Task::on_commit` (post-commit phase), and the rig. Never in `plan()`.
- The report record is the struct's `Debug`, snapshotted at spawn and
  refreshed at each `on_commit`.

## 6. The kernel contract

### 6.1 Lifecycle (lane 1)

Registered → **Gated** (needs armed; already-satisfied refs captured) →
**Plannable** → `plan()` (pure; cause delivered) → tentative in its
accumulator | Parked (non-empty wake set) → pop → **ratify** (§6.5) →
**post-commit phase** (§6.6) → Done | Gated(next needs, next epoch).
Invalidation returns the task to Plannable with the cause set. The kernel
never calls `plan()` outside these rules.

### 6.2 Settle, advance, convergence

Batch-poll plannable work against one frozen snapshot per batch; apply
outcomes in **registration-index order**; one demand **policy epoch**
served per pass at the fixpoint; advance only at quiescence; playback
mutates and wakes. "Plans root ≥ now" is **enforced**: ratification
validates every footprint start AND every enqueued playback instant.
**Convergence is bounded**: per-instant spawn/commit/fire budgets; a
`Continue` with neither a new dependency edge nor a time floor is
rejected; exhaustion is a deterministic `NonConvergentInstant` fault with
a causal trace.

### 6.3 Plans

```rust
pub struct Plan {
    pub home: AccumulatorId,        // the DEMAND axis: where it competes
    pub score: Score,
    pub footprint: Vec<SlotClaim>,  // the RESOURCE axis: what it claims
    pub closes: Vec<HoldClose>,     // T6
    pub reads: ReadStamps,          // §6.9
    pub est: WhFact,
    pub effect: WhEffect,
    /// OPT-IN chain-atomic co-ratification (decomp's overlay). Cycles
    /// rejected at propose time. Causal gating is needs/Park, never
    /// this.
    pub bundle_with: Vec<TaskId>,
}
```

### 6.4 Demand ends and licensing

Accumulators and their demand ends are **rig-declared**; a `DemandHandle`
is minted per `Demand` impl with allowed ends and fairness class
registered. All fires within one instant form **one policy epoch** — the
`SequencingPolicy` sees the set (with the clock and declared dynamic
terms), so plural allocation is a policy property, not a race outcome.
An end cannot be served through an unlicensed accumulator. **Scoring
stabilizers** (safety/headroom as hard feasibility classes;
deadline/pressure/fairness as tracked, refresh-stamped objectives with
hysteresis) are REQUIRED before any production-authoritative book
(ladder gate 3; PLAN Phase 9).

### 6.5 Ratification integrity: prepared projections

Validation produces, for every domain projection in the effect, a
**`PreparedProjection` token bound to the same generation/overlay the
transaction validates**. The ratifier validates the union — claims,
closes, read stamps, bundle, projections — then commits kernel state and
consumes every token as ONE operation; apply is mechanically infallible
(token-typed, no user code between claims and apply). A projection that
cannot be prepared rejects BEFORE any write
(`PlanCause::ProjectionRejected`). Runners fail on any integration
fault; stderr-and-continue does not exist.

### 6.6 The post-commit phase, facts, and future supply

After the transaction commits, the kernel runs the deterministic
post-commit queue — `on_commit` calls in registration-index order, their
spawns registered and validated — and **no newly woken consumer is
polled until the queue drains**. Failures here have owned semantics: the
transaction is never rolled back; a failed construction/registration is
a typed `PostCommitSpawnFault` attached to the parent; clean quiescence
cannot report success while one is unresolved.

Facts are epoch-addressed `(task, epoch)` per §3.1. **Future supply is
time-typed and atomically consumed**: a commit may publish
not-yet-physical supply only as `(item, available_at: CommittedAt)` with
the item's own timeline claimed through `available_at`; consumers floor
their cycles at it; consumption is atomic (two consumers cannot reserve
the same future item).

### 6.7 Cancellation: obligations are kernel-owned

`kill(id)` reaps the prefix subtree's uncommitted state; committed
facts/claims/holds persist. Resulting obligations (ownerless party
holds, unconsumed future supply, orphaned bundle members) are
**kernel-held**: `KillOutcome` is `#[must_use]`; each obligation
requires a typed disposition — `Recovered(receipt)`,
`Transferred(owner)`, or domain-validated `SafeAbandon(reason)`. No
unvalidated discard. Clean quiescence FAILS while any obligation is
ownerless.

### 6.8 Bounds, credits, termination

History: pruning + `floor` default; full-history opt-in; a
pruned/unpruned digest-equality CI pair. **Rig-level `CreditPool`** for
any globally-conserved manifest (per-consumer copies of a global bound
are unrepresentable; conservation audited). Clean-quiescence reporting
enumerates live leases, claims, future supply, demand handles,
suspensions, spawn faults, and obligations.

### 6.9 Stamped reads (the named pre-cutover spike)

Decision-bearing domain reads (inventory, topology, ownership, channels)
return **source-scoped stamps** that join `Plan.reads` and are validated
at ratification. Granularity (per-channel/per-key, fan-out cost bounds)
is the one surface named without full specification: a REQUIRED
pre-cutover spike (ladder gate 3; PLAN Phase 7). The coarse
world-generation fallback is rejected (it would invalidate on every
playback). Lane-2 summaries obey the same rule when used to decide
spawns.

## 7. Typed time (four role types over `SimTime`)

```rust
Quote        // advisory prediction (ArrivalQuote): may rank; NEVER gates;
             // consumed via reanchor into a proposed instant
CommittedAt  // ratified schedule truth (window ends, available_at)
ObservedAt   // playback/physical truth (ArrivedAt transitions)
Deadline     // configuration/policy thresholds (embargo, pacing)
```

All wrap `SimTime` directly (T1 generic time retracted with ruling 4g —
same deferred premise; coding standard §11 applies). Footprints and
playback enqueue take proposed instants that become `CommittedAt` on
ratification; physical gates key on `ObservedAt` transitions;
`Needs.after`/`Park::After` take `Deadline` or `CommittedAt`. A
reanchored quote never becomes "actual" by reanchoring — committed
schedule truth and observed physical truth are different types.

## 8. Worked examples

```rust
// Chain-head leg: no needs; plans immediately; commit-lazy at the pop.
impl Task for VtmForward {
    fn needs(&self) -> Needs { Needs::none() }
    fn plan(&self, ctx: &PlanCtx) -> Result<Plan, Park> {
        let (vtm, slot) = best_forward_slot(ctx, self.sku)?;   // Park::Until on none
        Ok(deposit_plan(ctx, vtm, slot, self.sku))
    }   // on_commit defaulted → Done
}

// Dependent leg: declarative gate; captured fact arrives via ctx.
impl Task for HtmDeliver {
    fn needs(&self) -> Needs { Needs::fact(FactRef::Terminal { task: self.pickup }) }
    fn plan(&self, ctx: &PlanCtx) -> Result<Plan, Park> {
        let WhFact::PickedUp { htm, access_cell, at, .. } =
            ctx.fact(FactRef::Terminal { task: self.pickup }) else {
            return Err(Park::Fault("pickup published a non-PickedUp fact".into()));
        };
        let leg = plan_leg(ctx, *access_cell, station_cell(self.station), *htm,
                           at.reanchor(ctx.now()))?;
        Ok(carry_plan(self.station, *htm, leg))
    }
}

// Two-stage gate: fact need, then level-check + Park on the transition.
impl Task for Presentation {
    fn needs(&self) -> Needs { Needs::fact(FactRef::Terminal { task: self.deliver }) }
    fn plan(&self, ctx: &PlanCtx) -> Result<Plan, Park> {
        let WhFact::ArrivalQuote { htm, station, .. } = ctx.fact(…) else { … };
        if !ctx.oracle().at_cell(*htm, station_cell(*station)) {    // stamped read
            return Err(Park::Until(vec![WhCond::ArrivedAt(*htm)])); // ObservedAt gate
        }
        Ok(dwell_plan(ctx, *htm, *station, self.sku))
    }
}

// Parent: spawns with real ids under its permit; supervises by fact refs.
#[async_trait]
impl Supervisor for OrderLine {
    async fn run(self: Box<Self>, ctx: SupCtx) -> Result<(), Fault> {
        let mut sp = ctx.spawner();
        let fwd     = sp.spawn(VtmForward { sku: self.sku });
        let pickup  = sp.spawn(HtmPickup  { deposit: fwd });
        let deliver = sp.spawn(HtmDeliver { pickup, station: self.station });
        let present = sp.spawn(Presentation { deliver, sku: self.sku, station: self.station });
        let FactOutcome::Fact(WhFact::Presented { htm, tote, .. }) =
            ctx.fact(FactRef::Terminal { task: present }).await else {
            return Err(Fault("presentation retired without presenting".into()));
        };
        let mut sp = ctx.spawner();
        let store = sp.spawn(HtmStore { htm, tote });
        let ret   = sp.spawn(VtmReturn { tote, store });
        ctx.fact(FactRef::Terminal { task: ret }).await;
        Ok(())
    }
}

// Demand: licensed fire; capability-scoped by construction; benign idle.
#[async_trait]
impl Demand for FlowPump {
    async fn run(self: Box<Self>, ctx: DemandCtx) -> Result<(), Fault> {
        loop { ctx.fire(&self.demand).await; }
    }
}
```

## 9. Migration

> **Operative punchlist: `PLAN.md`** (serial PRs, <2 kloc, spec lands in
> PR 1, the phase2 gate — including Daniel's headed release run G5 — on
> EVERY PR, halt on regression; each phase carries its why and its
> grounded references). Principles binding on the ladder: two PR species
> (golden-identical transformations vs dormant landings); authority cut
> only at **vertical slices** (a resource plus both its handoffs; never
> two writable authorities for one interval — the one-leg cutover is
> rejected); deliberate re-pins only, declared in the PR that moves
> them.

## 10. Accepted conventions (the residual, small and named)

1. `plan()` compute budget (bounded work per attempt).
2. `label()`/`Debug` quality.
3. Lane-2 narrative discipline (powers are scoped; prose can still rot —
   Daniel's law applies at review).
4. The needs/Park placement rule (§3.2) — stated, teachable, loud when
   violated.
5. **The kernel-payload lint** (ruling 4g's price): no `match` on
   payload types inside `kernel/` — CI-grepped, review-owned.

## 11. Open questions

- **OQ-A** (pre-authority): the §6.4 stabilizer/dynamic-term concrete
  shape (PLAN Phase 9 gates on it).
- **OQ-B**: §6.9 stamp granularity (the spike, PLAN Phase 7).
- **OQ-C**: windowed factorization choice (Appendix A holds the
  constraints; opens after PLAN Phase 7).
- **OQ-D**: whether lane-2 spawn pacing gets a structural CONWIP-credit
  form beyond §6.8's conserved manifests.

## 12. Review register and traceability

Criteria for the paper-adversary pass (ladder gate 2): v1.1's §12
criteria PLUS ergonomics (cold-read the §8 shapes per the bakeoff's
SCORING §3a; ceremony budget vs the winner's equivalents).

Traceability: Sol spec-review findings 1→§6.6, 2→§3.1, 3→§3.3, 4→§4+§5,
5→§6.7, 6→§6.5, 7→§3.1, 8→§7+§6.6, 9→§5, 10→§6.9, 11→§6.4, 12→§9/PLAN,
13→App A, 14→§6.2, 15→§6.8, 16→§4; Claude findings 1→§6.6, 2→§3.1, 3→§5,
4→§3.3/§6.9, 5→§4/OQ-D, 7→§3.3, 10→§3, 11→§3.2, 12→§2; rebuttal R1→§6.6,
R2→§3.1, R3→§3 (`&self`/on_commit), R4→§5, R6→§6.9, R7→status header;
Daniel's rulings 4c→§2, 4d→§4, 4e→PLAN, 4f→§4/status, 4g→§2/§7/status.

---

## Appendix A — Windowed pathfinding (compatibility constraints, not a proof)

Expressible in these nouns, NOT yet proven; constraints a future charter
must satisfy: factorization is a semantic choice (persistent entitlement
→ multi-increment Task; conflict-group → group Task; narratively
distinct windows → supervisor + children — the §3 routing rule governs);
each window is one increment through propose → compete → commit
(incomplete routes re-enter competition by construction); epochs address
windows exactly (`Terminal` = arrival); the §7 time types resolve the
estimate/commitment/observation split; extension demand must be bounded
and level-shaped (one outstanding extension per owner, §6.2 budgets);
clock regimes must be chosen (strict barrier default; overlapped
planning is a chartered extension); safety invariants owed before
production: safe-hold at every committed prefix end,
execution-dependency acyclicity under drift, k-robust envelope pricing,
dynamic realization order, commit-cut/GC rules (lit re-review
F2.1–F2.5). Cancellation mid-route returns the safe-stop obligation
through §6.7.
