range: cdbf81b4c5535a88faccae7ffd5d7d43a13f76d0..c9187c4a5bed6f929cfc11bd0408f17c1ebbd5b8

# SOL review — Two-Lane Task Model v1.1

## Verdict

**REVISE BEFORE RATIFICATION.** The two-lane factorization is directionally right. In
particular, it removes the most dangerous `Grant`-across-`await` authoring surface,
keeps resource decisions in a synchronous lane, gives orchestration a first-class
home, and turns several Round 4 caveats into explicit contracts. Those are meaningful
improvements.

The current surface is not yet a sound executable contract, however. Four defects are
ratification blockers:

1. user code is run inside the ratification transaction after state has been published;
2. `Needs` conflates edge-triggered wakeups with level-triggered prerequisites and can
   lose a wake at task handoff;
3. the Supervisor lane's claimed resource incapability is a convention, not a capability
   boundary; and
4. cancellation permits untracked abandonment of compensation obligations.

The epoch, time, identifier, read-validation, and migration contracts also need to be
closed before the first production-authoritative cutover. Ratify the **factorization**
after the blockers below are repaired; do not ratify this exact API as the production
contract.

## Review vocabulary

- **BLOCKER** — must be resolved in the normative model before ratification.
- **HIGH** — must be resolved before the first production-authoritative cutover.
- **MEDIUM** — may be implemented incrementally, but needs an explicit contract and a
  named gate.
- **PRESERVED / PARTIAL / SILENT / SUPERSEDED** — disposition of an earlier decided
  doctrine. Silence on a load-bearing doctrine is itself a defect.

## Method

This is an **architecture and abstraction-contract review**, not a branch/code-quality
review. The exact range identifies the specification and evidence revision reviewed.
Winner and master code are used only to test feasibility, behavioral preservation, and
migration walkability; implementation style, naming, and diff aesthetics are out of
scope. Findings were derived from the specification's §12 criteria and the review-request
charter.

## Findings

### 1. BLOCKER — `on_commit` runs fallible user code inside an allegedly infallible transaction

The trait says `on_commit` runs "inside the ratification apply, after this task's fact is
published" and may spawn successors (`TWO_LANE_SPEC.md:117-120`). Section 6.5 says the
opposite kind of thing: all domain projections are prevalidated before any kernel write,
then apply cannot fail (`TWO_LANE_SPEC.md:304-310`). `SpawnCtx::spawn` constructs and
registers arbitrary task code; construction can allocate, panic, recurse, or violate a
rig invariant. A task's claims, domain writes, and fact have already become visible when
that code runs.

This is not hypothetical friction with the winner. The current ratifier closes claims
and commits resource state before domain application and fact commit
(`../bakeoff4-seam/sim/dessim/src/multisched/kernel/ratify.rs:197-213`). Adding a user
callback at that point broadens the partial-failure boundary.

**Required disposition:** make successor publication a deterministic post-transaction
phase. Prefer `on_commit` returning inert `SpawnSpec`s computed from the exact committed
fact, with all registration validation completed before publication. Alternatively,
enqueue an infallible, kernel-owned post-commit record and do not poll any newly woken
consumer until that queue is drained. `on_commit` must not see a world/resource view.
OQ1 therefore leans **after the transaction**, and OQ2 leans **fact plus task-local data
only**.

### 2. BLOCKER — `Needs.conds` has an unavoidable lost-wake ambiguity

`Needs` says its conditions are transitions that "must have fired"
(`TWO_LANE_SPEC.md:134-149`). A newly spawned task that arms after a relevant transition
already fired will wait for the *next* transition, possibly forever. This is exactly why
the winner's condition primitive is explicitly edge-after-arm: readiness is
`current_version > registered_version`
(`../bakeoff4-seam/sim/dessim/src/multisched/readiness.rs:31-52`). The winner also has
explicit `All` and `Any` forms (`readiness.rs:59-75`), while `Needs` exposes only
all-of vectors.

The Presentation example avoids the bug by doing a level check in `plan()` and returning
`Park` only if it is currently false. That escape hatch is unavailable to an initial
`Needs` created by a predecessor at the exact handoff the model wants to make safe.

**Required disposition:** replace raw `Vec<ConditionId>` with a readiness algebra that
distinguishes at least:

- `Level(predicate)` — evaluate current state atomically with registration;
- `AfterNext(condition, observed_epoch)` — require an edge after a named arm point;
- `FactExact` / `FactNextAfter` — capture the fact epoch that satisfied the wait; and
- `All` / `Any` composition.

Specify registration plus current-state sampling as one settle operation. Runtime-valued
conditions may still be expressed as `plan()` plus `Park`, but initial handoff gates must
not silently change from level to edge semantics. OQ3 leans **boolean readiness tree with
explicit level and edge leaves**.

### 3. HIGH — typed invalidation causes exist in prose, not in the authoring API

The lifecycle and replacement table promise that `plan()` is re-called with a typed cause
(`TWO_LANE_SPEC.md:174-183,256-265`), but the trait only receives `&PlanCtx`, and no
normative `PlanCtx::cause()` or cause algebra is defined (`TWO_LANE_SPEC.md:109-115`).
This reopens Round 1 doctrine 2: a task cannot distinguish its initial plan, a resource
invalidation, a condition wake, a producer retirement, or a projection rejection.

The accepted `plan(&mut self)` convention makes the omission worse. Mutations made by a
failed or stale attempt survive, so retry order can affect future candidates while the
document still calls planning pure.

**Required disposition:** define a one-shot `PlanCause`, for example
`Initial | Woke(WakeSet) | Invalidated{resource, at} | ProjectionRejected{domain} |
ProducerRetired{producer, epoch}`. Specify precedence when several causes accumulate and
whether a cause is consumed by `Park`. Prefer staging task-local plan state and committing
it only with the accepted plan; at minimum, document persistent mutation and test replay
of invalidated attempts.

### 4. BLOCKER — the Supervisor lane is not structurally resource-incapable

The document repeatedly claims that a Supervisor "cannot touch the resource timeline"
(`TWO_LANE_SPEC.md:185-225`). The API only withholds direct reserve methods. It still
permits three capability leaks:

1. `D::View` is an unconstrained associated type. In the winner it need only be
   `Clone + Send + Default` (`../bakeoff4-seam/sim/dessim/src/multisched/kernel/domain.rs:56-61`).
   It can therefore contain `Arc<Mutex<ResourceTable>>`, a mutable service client, or a
   snapshot-grade oracle.
2. `SpawnCtx` lets any Supervisor spawn a `Task` with `Needs::none()` that requests any
   resources.
3. `fire(AccumulatorId)` lets any Supervisor repeatedly license resource work. Raw IDs
   provide neither authority nor an allowed-end set.

Thus a Supervisor can smuggle a mutable capability, proxy arbitrary resource acquisition
through children, or generate a spawn/fire storm. "Cannot directly call reserve" is a
useful ergonomic boundary, but it is not resource incapability.

**Required disposition:** use a capability-sealed `SupervisorView` DTO, not arbitrary
`D::View`; make resource-bearing domain types private to the Task lane; and mint scoped
handles in the rig, such as `Spawner<AllowedTaskKinds>` and
`DemandHandle<AllowedEnds>`. `fire` should consume or await an idempotent, bounded license,
with at most one outstanding demand per declared end unless a joint round explicitly
licenses several. Reword the architectural claim to match the authority actually
provided. OQ4 leans **only declared demand Supervisors with rig-minted typed handles**;
OQ7 leans **small, owned, capability-sealed summaries**.

### 5. BLOCKER — cancellation still uses a remember-to-do compensation contract

Section 6.7 returns compensation obligations to the caller, who "must schedule them or
explicitly discard" them (`TWO_LANE_SPEC.md:325-331`). This is the exact failure mode the
Round 4 Q19/Sol 19 lineage rejected: the obligation can be forgotten, and "discard" makes
that omission legal. It also conflicts with the governing saga principle that committed
physical work has a total lifecycle.

**Required disposition:** make `KillOutcome` `#[must_use]` and keep its obligations in
kernel-owned state until a typed disposition is committed. Legal dispositions should be
`Recovered(receipt)`, `Transferred(owner)`, or a domain-validated
`SafeAbandon(reason)`—not an unvalidated discard. Clean quiescence must fail while an
obligation is ownerless. Define reverse-order compensation for the committed prefix,
idempotence, and the default forward-recovery policy for a robot already in motion. Prefix
cancellation must also state what happens to shared bundle children and their published
facts.

### 6. HIGH — projection prevalidation does not make multi-domain apply atomic

Section 6.5 closes an important Round 4 hole in intent, but `validate` followed later by
`apply` is vulnerable to a generation change between those operations. A multi-leg or
multi-domain projection can also validate each part against different state. Removing a
`Result` return does not prevent a panic or side effect.

**Required disposition:** validation must produce a `PreparedProjection` tied to the
same locked generation/transaction overlay used for *all* domain writes. Apply consumes
that token and is mechanically infallible. The ratifier must validate the union, commit
kernel state and every prepared projection as one operation, then run the post-commit
queue from finding 1. If a domain cannot support that contract, its projection belongs
after the kernel transaction with an explicit recoverable/fail-stop protocol, not under
an atomicity claim.

### 7. HIGH — the public fact API cannot express the epoch semantics promised by §6.6

Section 6.6 says facts are epoch-qualified and later commits do not retroactively satisfy
an earlier need (`TWO_LANE_SPEC.md:312-323`). Yet `Needs.facts` and both context accessors
are keyed only by `TaskId` (`TWO_LANE_SPEC.md:138,201-203`). "Latest epoch" is racy: a
consumer may arm for epoch *n*, be woken by *n+1*, and read *n+2* by the time it plans.
The winner's current store overwrites facts by `TaskId`
(`../bakeoff4-seam/sim/dessim/src/multisched/facts.rs:17-20,49-66`), so the new surface
has to change this explicitly.

`ProducerRetired` also cannot be returned by an accessor typed as `&Fact` without an
unstated panic/error channel.

**Required disposition:** use an opaque `FactRef<F>` with
`Exact(epoch) | NextAfter(epoch) | Terminal`, and make wake resolution capture the exact
`(producer, epoch, value)` consumed by `plan()`. Retain committed epochs until no live
reference can name them. Model producer retirement as a typed readiness result or task
fault, not an exceptional lookup.

### 8. HIGH — `Actual<T>` still conflates observed time with committed scheduled time

The spec correctly prohibits raw quotes from gating, but it then says that reanchoring a
quote against plan-time truth turns it into `Actual<T>` (`TWO_LANE_SPEC.md:340-347`). A
future reserved arrival or safe-stop endpoint is committed schedule truth; it is not a
physical observation. Appendix A similarly calls a committed prefix's endpoint an
"Actual fact" (`TWO_LANE_SPEC.md:556-562`). This repeats the structural error behind E7
under a better name.

**Required disposition:** distinguish at least:

- `Quote<T>` — advisory prediction;
- `CommittedAt<T>` (or `Scheduled<T>`) — ratified timeline fact;
- `ObservedAt<T>` — playback/physical truth; and
- `Deadline<T>` — configuration/policy threshold.

Footprints and playback enqueue use committed schedule times. Physical gates use observed
transitions. Configuration deadlines may gate time directly. Future-supply records in
§6.6 must use `CommittedAt`, carry the owner's interval claim, and be atomically consumed
or shared according to an explicit policy; otherwise two consumers can reserve the same
future item.

### 9. HIGH — hierarchical IDs do not yet guarantee deterministic parallel spawn

The `(parent, local_seq)` proposal (`TWO_LANE_SPEC.md:227-254`) leaves several decisive
details unspecified:

- concurrent child futures sharing one parent still allocate `local_seq` in poll order
  unless `SpawnCtx` is exclusive and non-`Send`;
- recursive storage, overflow, maximum depth, and prefix cancellation are undefined;
- returning a real ID immediately removes the winner's useful batch-mint property, where
  sibling IDs exist before any sibling constructor observes the world; and
- using tree-lexicographic ID order as the apply tiebreak makes a pure refactor or
  reparenting change simulation policy.

The claim that dangling `TaskId` references are unrepresentable is also false if `TaskId`
is an ordinary copyable value.

**Required disposition:** retain batch spawn; derive sibling IDs from stable input order;
make `SpawnCtx` exclusive/non-`Send`; store a structured parent relation or full path with
defined limits; and separate stable identity from scheduling priority. Use a generative,
non-forgeable `TaskHandle<F>` or validate every dependency in the spawn transaction.
OQ8 leans **structured parent/path storage plus batch minting**, not opaque bit-packing.

### 10. HIGH — non-slot domain reads have no commit-time validity contract

Lane 1 still plans from `D::View`, but the spec validates only resource claims and domain
projections. A task can read inventory, topology, channel state, or an ownership table and
commit after that source changes, with no read-set conflict. This is Sol finding 7 and the
unclosed half of Round 3's snapshot-economics ruling. Removing a snapshot-holding Grant
does not make the read fresh.

**Required disposition:** every planning oracle read that can affect claims or writes
must return a stamped token included in the plan read set, or all such mutations must bump
a coarse world generation that ratification validates. Supervisor summaries must follow
the same rule when a summary is used to decide which resource task to spawn. Keep the
token narrow and source-scoped to avoid restoring the snapshot economics the model is
trying to remove.

### 11. HIGH — demand licensing is prose-only and cannot enforce plural-policy ownership

Section 6.4 says every open resource must have a licensed demand Supervisor and that
policy chooses among plural demands (`TWO_LANE_SPEC.md:294-302`). The surface exposes raw
`AccumulatorId` and an unrestricted `fire`, so neither existence, ownership, one-shot
use, nor the allowed set of demand ends is enforceable. A set of Supervisors can recreate
registration-order policy by racing their fires, bypassing the stated joint-solve
boundary.

**Required disposition:** require a rig-declared `DemandHandle`; register its ends and
fairness class; make same-instant fires visible to one policy epoch; and prevent an end
from committing through an unlicensed accumulator. Safety and headroom stay hard
constraints. Dynamic distance/aging terms need source-stamped refresh, timer refresh, and
hysteresis before a production book is authoritative. OQ6 leans **hard feasibility
classes plus tracked dynamic objectives**, not a static score snapshot.

### 12. HIGH — §9's first "one leg" cutover is not walkable across the VTM wall

The strangler decomposition correctly separates cheap mechanical seams from semantic
walls, but §9 still proposes cutting over one leg after mechanical extraction
(`TWO_LANE_SPEC.md:416-440`). Master's VTM truth is a scalar frontier:
`available_at` (`sim/dessim/src/model/relay/vtm/mod.rs:103-108`) is selected against and
advanced (`vtm/mod.rs:260-269,343-364`). The live VTM task eagerly selects, mutates, and
hands off (`sim/dessim/src/flows/ttp/task/vtm_forward.rs:109-137,185-196,409-420`), while
relay reservations use panic-on-drop RAII
(`sim/dessim/src/model/relay/reservation.rs:35-70,85-93`).

A v2 leg that owns a lazy time-indexed timeline cannot safely coexist with legacy
neighbors that still consult the scalar frontier. Mirroring v2 into the scalar destroys
holes; not mirroring permits double booking. That boundary is not leg-local.

**Required disposition:** first land a no-authority shadow kernel and parity recorder.
Then cut authority at a whole resource/vertical-slice boundary—VTM timeline, relay claim,
and both handoffs together—with a one-way legacy oracle bridge. Do not dual-write two
authoritative truths. Gate each step on direct golden parity, same-physics makespan,
recirculation, adversarial hole reuse, and clean-quiescence reports. The real-day Phase 2
case remains a release gate, not necessarily a per-edit unit test.

### 13. HIGH — Appendix A is plausible, but its forward-compatibility claim is too strong

Multi-shot tasks can express rolling-window work only after additional semantics are
fixed. As written:

- `Needs.after` is an absolute time, not a level-triggered lookahead threshold;
- a Supervisor can repeatedly fire and spawn, so bounded outstanding work is not
  guaranteed;
- strict settle barriers freeze model time during extension computation, whereas a
  production planner may need the robot to execute inside an already committed prefix;
- the spec leaves per-robot versus conflict-group/joint factorization undecided even
  though that choice changes conflict and fairness semantics; and
- k-robust envelopes, action-dependency-graph acyclicity under duration drift, dynamic
  realization order, and the commit/garbage-collection cut are named but not contracted.

The entitlement literature also leaves a valuable option—reallocation before physical
binding—without a lease/repair surface. That is not required for v1, but the document
should not claim that nothing else is needed to express the family.

**Required disposition:** keep `Task::Continue` as the base mechanism (OQ5), but define a
lookahead-level condition, one outstanding extension per owner/conflict group, a safe-hold
invariant, execute/commit window relation, and the selected production factorization.
Expose two clock regimes: deterministic barrier mode, and overlapped planning within the
committed envelope (or explicitly charge planning latency). Require an acyclic execution
dependency graph and a priced safety envelope before calling the adapter production-ready.

### 14. MEDIUM — zero-time spawn/fire/continue cycles can prevent settle forever

`Needs::none`, Supervisor spawn/fire, post-commit spawning, and `Next::Continue` can form
an unbounded same-`Actual` cycle. The spec gives settle deterministic order but no
convergence bound (`TWO_LANE_SPEC.md:267-275`). This reopens Round 1's finite-rooted-work
requirement.

**Required disposition:** reject a `Continue` that has neither a new edge/time dependency
nor a statically bounded remaining iteration count. Add per-root/per-instant spawn,
commit, and fire budgets with a deterministic `NonConvergentInstant` fault and causal
trace. A daemon that intentionally lives forever must still yield on a named condition or
time edge.

### 15. MEDIUM — global credits and clean termination are still outside the two-lane surface

The StageLoop failure was a global conservation error, not merely a scheduling bug.
Spawning one Supervisor per station can reproduce the same duplicate manifest unless the
rig owns a single licensed credit pool. Likewise, clean termination must account for
open holds, future-supply claims, child trees, retired producers, and compensation
obligations—not just runnable tasks.

**Required disposition:** add a rig-level `CreditPool`/manifest capability with atomic
transfer and conservation checks. Make clean-quiescence reporting enumerate every live
lease, claim, demand, daemon suspension, and recovery obligation. This should be a named
migration gate and closes the typed-rig-validation lesson of E12.

### 16. MEDIUM — the async Supervisor adapter needs a structural armless-future rule

The model describes a noop-waker adapter for async Supervisors, but `async_trait` permits
awaiting an arbitrary library future. Such a future may return `Pending` and depend on an
external waker that the adapter ignores, producing a permanent hang indistinguishable
from a legitimate kernel wait.

**Required disposition:** seal the awaitable surface to futures produced by `SupCtx`, or
track an owned kernel suspension token and fault `Pending` without one. Do not present the
lane as a general executor. The same decision should explicitly say that async
selected-candidate resource preparation is deliberately unsupported; expensive resource
planning must run as bounded synchronous `plan()` work on a compute worker or through a
future, separately ratified extension.

## Open-question recommendations

| OQ | Recommendation |
|---|---|
| 1 | Run successor handling after the transaction in a deterministic kernel-owned post-commit phase. |
| 2 | Give `on_commit` only the exact fact and task-local committed data; no world or resource view. |
| 3 | Use an `All`/`Any` readiness tree with explicit level, edge-after-arm, exact-fact, and time leaves. |
| 4 | Permit `fire` only through rig-minted `DemandHandle`s scoped to declared ends and fairness classes. |
| 5 | Keep multi-shot `Task::Continue`; add bounded lookahead/extension semantics rather than a new kernel planner type. |
| 6 | Make safety/headroom hard constraints and dynamic terms tracked, timer-refreshed objectives with hysteresis. |
| 7 | Prefer small owned, capability-sealed summaries. Use stamped query tokens if more detail is required. |
| 8 | Use structured parent/path storage, batch minting, and an exclusive `SpawnCtx`; keep priority separate from identity. |

## Appendix A ruling

Appendix A identifies the right broad decomposition—windowed resource commitments in
Lane 1, long-lived coordination in Lane 2—but it is a **compatibility sketch, not yet a
proof of expressibility**. Before it can serve as an implementation charter it must pick
the production factorization and clock regime, define bounded extension demand, state the
safe-hold and execution-dependency invariants, use exact fact epochs, and repair the time
types. With those changes, no dedicated kernel-native planner task appears necessary.

## Migration ruling

The ladder is credible through shadowing and mechanical extraction. The first authority
cut is too fine-grained. Replace "cut over one leg" with:

1. shadow/no-authority v2 plan and commit records;
2. mechanically extracted `PathIntent -> validate -> apply` and prepared domain writes;
3. one authoritative vertical slice including a resource and both of its handoffs;
4. direct golden, phase-0 provenance, recirculation, adversarial-density/hole, makespan,
   bit-determinism, and clean-quiescence gates; and
5. only then removal of the corresponding legacy scalar/RAII truth.

No phase may maintain two writable authorities for the same VTM or relay interval.

## Corpus disposition ledger

### Round 1 doctrines and errata

| Earlier ruling | Disposition in v1.1 |
|---|---|
| 1. waits are transitions, not periodic predicates | **PARTIAL** — `Park` preserves edges; `Needs` does not distinguish level from after-arm edge (finding 2). |
| 2. invalidation cause is typed input | **PARTIAL** — promised in prose, absent from the API (finding 3). |
| 3. committed facts survive producer progress | **PARTIAL** — stated, but the reference surface cannot name exact epochs (finding 7). |
| 4. exit reaps descendants and committed work has a total lifecycle | **BROKEN** — prefix cancel is good; unvalidated discard reopens the lifecycle hole (finding 5). |
| 5. armless wait is a structural fault | **PARTIAL** — intended for Supervisors; generic foreign futures can still hang armless (finding 16). |
| 6. IDs are stable independent of poll order | **PARTIAL** — hierarchical intent is good; shared-parent and batch-mint rules are missing (finding 9). |
| 7. demand participants, not task registration order, express policy | **PARTIAL** — prose preserved, raw `fire` cannot enforce it (finding 11). |
| 8. pull/settle reaches a deterministic quiescent instant | **PARTIAL** — order is stated; finite convergence is not (finding 14). |
| 9. `CommitTx` is write-only/disjoint and atomic | **BROKEN** — prevalidation helps, but user callbacks and unstamped reads violate it (findings 1, 6, 10). |
| 10. policy is chosen at simultaneity boundaries | **PARTIAL** — named, but multiple racing fires can bypass one joint epoch (finding 11). |
| E1. make policy-at-simultaneity normative | **PARTIAL**, same as doctrine 10. |
| E2. the old S5 bound was non-informative | **PRESERVED PROCEDURALLY** — migration keeps adversarial performance gates; do not reuse S5 as proof. |
| E3. base-break was procedural | **PRESERVED** by the explicit baseline/golden gates. |
| E4. expand temporal fixtures T1-T8 | **PARTIAL** — typed time is explicit, but committed versus observed time remains conflated (finding 8). |

### Round 1 contestant-shape rulings

| Shape record | Disposition in v1.1 |
|---|---|
| async | **SUPERSEDED WITH RATIONALE** on the resource path: resource awaits and outside-substrate allocation were convicted. Narrative async survives only in Lane 2, subject to findings 4 and 16. |
| actor | **SUPERSEDED WITH RATIONALE**: implicit/level state and the table-less frontier hold remain forbidden; typed holds and timeline claims preserve the useful ownership lesson. |
| multisched | **PRESERVED AS SUBSTRATE/LINEAGE**: tentative menus, pop-at-quiescence, fact dependencies, and data effects are the direct Lane-1 ancestry. Its stage-machine burden motivates the two-lane split. |
| spawn | **PARTIAL**: direct/open-world spawning is preserved, but v1.1 drops the proven batch-mint-before-construction property (finding 9). |
| effects | **PRESERVED WHERE DECIDED**: effects remain typed data and apply closures stay dead. A universal letterbox/replay surface was not ratified and is not required here. |
| hybrid | **SUPERSEDED ON RESOURCE CALLBACKS; PRESERVED FOR SUPERVISION**: Round 2 convicted its multi-method resource protocol, while long-lived narrative bodies support Lane 2. Finding C10 states the boundary needed to avoid recurrence. |

### Round 2 and Round 3 rulings

| Earlier ruling | Disposition in v1.1 |
|---|---|
| E5. accepted shared kernel is the reference substrate | **PRESERVED**. |
| E6. the planning narrative was not an implementation charter | **SUPERSEDED** by a deliberate two-lane charter, subject to this review's blockers. |
| E7. split `ArrivalQuote` from `Arrived`; rename ambiguous ticks | **PARTIAL** — quote is separated, but future commitment is mislabeled `Actual` (finding 8). |
| T1-T8 temporal suite and P lookahead probe | **REQUIRED MIGRATION GATE**; the spec should name the expanded suite, not only generic golden tests. |
| Round 3 F1. invalid legacy fixture | **PRESERVED** through baseline provenance gates. |
| Round 3 F2. unsafe scoped-view/panic path | **SUPERSEDED** for Lane 1 by synchronous planning; mutable Supervisor view leakage remains (finding 4). |
| Round 3 F3. global condition over-wake | **PARTIAL** — source-scoped IDs remain, but level/edge semantics are incomplete (finding 2). |
| Round 3 F4. missing real edge conflicts | **PRESERVED IN SUBSTRATE**; migration must retain the adversarial edge suite. |
| Round 3 F5. reused-wake semantic conflict | **PARTIAL**, repaired for `Park`, reopened for initial `Needs` (finding 2). |
| Round 3 F6. proposal outcome ambiguity | **SUPERSEDED** by the ratified-plan model. |
| Round 3 F7. three snapshot regimes | **SUPERSEDED** as an author surface, but view read validity remains silent (finding 10). |
| Round 3 F8. snapshot economics | **PARTIAL** — no Grant-held snapshot; stamped, narrow reads are still required (finding 10). |
| E8. baseline fixture premise was false | **PRESERVED PROCEDURALLY**; typed rig validation from E12 is still missing (finding 15). |
| E9. no quote gates; snapshot economics are contract-level | **PARTIAL**, findings 8 and 10. |

### Round 2 temporal audit A1-A10

| Audit finding | Disposition in v1.1 |
|---|---|
| A1. concrete `SimTime` field-wide | **PRESERVED FIX** — generic `Time` remains substrate vocabulary. |
| A2. untimestamped mutation APIs | **PRESERVED IN INTENT** through typed contexts; the spec must keep the semantic instant on prepared projections. |
| A3. inconsistent fact stamps | **PARTIAL** — facts are timed/epoched in prose, but exact epoch addressing is absent (finding 7). |
| A4. interval truth represented as levels | **PRESERVED** for resources through windows/holds; future-supply availability needs an owner interval, not a scalar time (finding 8). |
| A5. expiry/reanchor/Blocked-vs-Stale were conventions | **PARTIAL** — stale/blocked substrate survives, but quote reanchor is assigned the wrong semantic type (finding 8). |
| A6. sentinel or table-less open holds | **PRESERVED FIX** — typed, owner-tagged holds remain normative. |
| A7. playback time derived from resume instant | **PRESERVED DIRECTIONALLY** — playback remains model-side; observed and committed instants still need distinct types. |
| A8. event stamp and mutation instant diverged | **PRESERVED REQUIREMENT** — prepared domain writes must carry the same committed/observed instant they publish. |
| A9. raw-millisecond arithmetic | **PRESERVED FIX** by generic time and typed constructors. |
| A10. raw time folded into scores | **PRESERVED MECHANICALLY**; dynamic-score stability and refresh remain open (finding 11). |

### Round 2 nine-fault regression register

| Convicted fault | Disposition in v1.1 |
|---|---|
| frontier hold without a table row | **PRESERVED FIX** by typed holds. |
| HTM fleet allocated outside the substrate | **PRESERVED FOR TASKS**; Supervisor proxy authority must not recreate it (finding 4). |
| dirty plan sleeps with frozen score | **PRESERVED IN SUBSTRATE**; dynamic external score sources still need stamped refresh (finding 11). |
| brick/external mutation bypasses invalidation | **PRESERVED IN SUBSTRATE**. |
| a resource plan can sleep | **STRUCTURALLY PRESERVED** in Lane 1; generic async remains only in the nominally non-resource lane. |
| lodged proposal outlives owner | **PRESERVED IN SUBSTRATE**, assuming post-commit children are not published mid-reap (finding 1). |
| milestone/source registration leak | **PARTIAL** — source-scoped versions survive; initial `Needs` can still lose a transition (finding 2). |
| retired owner leaves proposals live | **PRESERVED IN SUBSTRATE**; descendant/obligation lifecycle is incomplete (finding 5). |
| apply-as-closure | **PRESERVED FIX** — effects remain data and prepared apply must stay infallible. |

### Round 4 rulings and gate addendum

| Earlier ruling | Disposition in v1.1 |
|---|---|
| R4 F1 / E10. same-physics speed correction | **PRESERVED** by the same-physics baseline gate. |
| R4 F2. Phase-0 and VTM-tight density are load-bearing | **PRESERVED AS MIGRATION GATES**; do not infer density from window purity alone. |
| R4 F3. greedy liveness and registration-order solver failures | **PARTIAL** — joint policy is named, licensing and dynamic score refresh are not enforced (finding 11). |
| R4 F4. scalar VTM frontier is the semantic wall | **ACKNOWLEDGED BUT NOT WALKABLE** at a one-leg cut (finding 12). |
| R4 F5. real-day case proves direct production semantics | **PRESERVED** as the final release gate. |
| R4 F6. convergence evidence is not architecture proof | **PRESERVED**; no architectural claim should rest only on the Phase-2 run. |
| R4 F7. performance wall/history | **PRESERVED** by default pruning and full-history opt-in. |
| R4 F8. API hygiene/recirculation | **PARTIAL** — Task Grant hazard is removed; Supervisor view and recirculation gates remain (findings 4, 12). |
| E11. Phase-0 procedural caveat | **PRESERVED PROCEDURALLY**; keep the caveat in the named migration fixture. |
| E12. silent real-data book hang requires typed rig validation | **SILENT** — add rig-level validation and global credit conservation (finding 15). |
| R4.19 / R4.22. prune claims and done-task history | **PRESERVED** in §6.8. |
| R4.20 / R4.21. recirculation and bridge realism | **TEST-ONLY, NOT STRUCTURAL** — must be explicit gates in §9. |
| R4.23. no grants across awaits | **PRESERVED FOR TASKS**; Supervisor capability/read leakage is the remaining analogue (findings 4, 10). |
| operational future-supply race | **PARTIAL** — §6.6 recognizes it, but epoch and time/claim types are incomplete (findings 7, 8). |
| stairstep allocation policy | **DEFERRED WITH RISK** — acceptable only while hard constraints and refresh semantics are settled before authority (finding 11). |
| R4.18 startup parade | **NOT A DESIGN PROOF**; preserve its fixture control and ensure the shadow/authority gate does not special-case startup ordering. |
| R4.24 complete gate table | **PRESERVED AS THE RELEASE EVIDENCE BASELINE**: direct real-day, memory, bit-determinism, fault, and same-physics results remain mandatory comparators. |

### Five Round 4 lineage records

| Lineage record | Disposition in v1.1 |
|---|---|
| seam (`R4.1-R4.24`) | **DESIGN ORACLE, PARTLY SUPERSEDED AUTHOR SURFACE** — timelines, ratifier, facts, policy, CoW evidence, and real-day gates remain authoritative. Grant/template ergonomics are deliberately replaced; their semantic invariants are not. |
| narrative | **PRESERVED AS AUTHORABILITY EVIDENCE** for long-lived parents/daemons and the fact→level-check→Park pattern. Its simplified planner/fidelity results are not a production semantics oracle. |
| decomp | **PRESERVED** for chain atomicity, direct spawn structure, CoW economics, and supervision separation; its unsafe scoped view is not prior art to copy. Its whole-chain/semantic-wall evidence argues against a one-leg authority cut. |
| solver | **PRESERVED AS A SEPARATE PROGRAM** — F4 registration-order inversion, K-swap stranding, and greedy joint-liveness failures justify plural same-epoch licensing. v1.1 reserves but does not enforce that boundary (finding 11). |
| strangler | **PRESERVED AS THE MIGRATION WALL INVENTORY** — `CommitTx` and path intent extraction are bankable designs; transition arms, disjoint playback, demand unification, generic time, and lazy VTM authority are semantic cuts. §9 understates the last one (finding 12). |

### Original Sol findings 1-20

| # | Disposition |
|---|---|
| 1. serial atomic commit | **PARTIAL** — §6.5 helps; findings 1 and 6 reopen atomicity. |
| 2. closure overlay / bundle-union validation | **PRESERVED IN SUBSTRATE**, but prepared multi-domain validation must be explicit (finding 6). |
| 3. dependency versus gate split | **PRESERVED** by facts, readiness, and bundle-cycle rejection. |
| 4. accumulator registration order | **PARTIAL** — plural demand is prose-only (finding 11). |
| 5. fixed-epoch finite scan | **PARTIAL** — kernel scan remains; same-instant spawn/continue can be infinite (finding 14). |
| 6. supply wakeup | **PARTIAL** — accumulators preserve it; `Needs` handoff can lose it (finding 2). |
| 7. non-slot read set | **SILENT** (finding 10). |
| 8. overlap refresh | **PRESERVED IN SUBSTRATE**. |
| 9. dynamic score freshness | **OPEN** and production-critical (finding 11). |
| 10. stabilizers/aging/hysteresis | **OPEN** and production-critical (finding 11). |
| 11. solver replacement boundary | **PRESERVED** as a separate policy layer; same-epoch fire collection must be enforced. |
| 12. rolling committed prefix | **PRESERVED** in Appendix A. |
| 13. prediction versus event time | **PARTIAL** — finding 8. |
| 14. synchronous candidate generation, optional selected preparation | **PARTIAL** — synchronous planning is selected; async selected preparation is implicitly foreclosed (finding 16). |
| 15. algorithm/runtime factorization | **PRESERVED AS OPTIONS**, but the production choice remains mandatory (finding 13). |
| 16. one-shot/multi-shot/joint option set | **PRESERVED**; multi-shot is the recommended base. |
| 17. fact epochs | **INTENDED BUT NOT EXPRESSIBLE** (finding 7). |
| 18. determinism contract | **PARTIAL** — structured IDs need completion (finding 9). |
| 19. clock and cancellation | **BROKEN/PARTIAL** — findings 5, 8, and 13. |
| 20. choose a windowed authoring shape | **PRESERVED** by multi-shot Task plus Supervisor orchestration, after the blockers. |

### Sol Round 4 findings 1-22

| # | Disposition |
|---|---|
| 1. domain projection can partially fail | **INTENDED CLOSED, NOT YET CLOSED** (findings 1 and 6). |
| 2. downstream supply races future occupancy | **PARTIAL** (findings 7 and 8). |
| 3. StageLoop manifest duplication | **SILENT** (finding 15). |
| 4. bundle/fact graph semantics | **PRESERVED** by cycle rejection and producer retirement intent; epoch access remains broken. |
| 5. playback must not enqueue in the past | **PRESERVED** by §5.1. |
| 6. history retention is unbounded | **PRESERVED** by §6.8. |
| 7. supply waiter and stable same-instant order | **PARTIAL** — stable order preserved, initial condition wake can be lost. |
| 8. demand end licensing | **INTENDED BUT UNENFORCED** (finding 11). |
| 9. snapshot/view lifetime | **PARTIAL** — Task Grant removed; Supervisor view leakage remains (findings 4, 10). |
| 10. `Done` latch semantics | **SUPERSEDED** by explicit fact epochs and lifecycle, once finding 7 is fixed. |
| 11. fact epochs | **INTENDED BUT API-INCOMPLETE** (finding 7). |
| 12. history default and CI mode | **PRESERVED**. |
| 13. Row V cutover wall | **ACKNOWLEDGED, MIGRATION STEP UNSOUND** (finding 12). |
| 14. master scalar time versus v2 intervals | **PARTIAL** — v2 chooses intervals, but typed time and coexistence are unresolved. |
| 15. runner/projection failure semantics | **INTENDED CLOSED, NOT MECHANICALLY CLOSED** (finding 6). |
| 16. future supply availability | **PARTIAL** — wrong time and atomic-consumption types (finding 8). |
| 17. every open end needs a demand owner | **PROSE-ONLY** (finding 11). |
| 18. bundle semantics and cycle rejection | **PRESERVED**. |
| 19. terminal cohort/obligation lifecycle | **BROKEN** by unvalidated discard (finding 5). |
| 20. grant hygiene | **PRESERVED FOR TASKS**, incomplete for Supervisor views/read sets. |
| 21. dynamic scores | **OPEN** (finding 11). |
| 22. bounded history | **PRESERVED**. |

### Entitlement and rolling-horizon literature re-review

| Literature finding | Disposition |
|---|---|
| F1.1 lazy commitment is central | **PRESERVED** by multi-shot committed prefixes. |
| F1.2 admission/pricing at the boundary | **SILENT**; acceptable for v1 only if headroom stays hard and no throughput claim relies on it. |
| F1.3 slack/safety envelope | **SILENT** and required for a production window adapter (finding 13). |
| F2.1 action-dependency-graph acyclicity | **SILENT** (finding 13). |
| F2.2 commit cut and garbage collection | **PARTIAL** — generic history pruning exists; multi-window dependency retirement is unspecified. |
| F2.3 k-robust safety | **OPEN/OUT OF SCOPE**; therefore the Appendix cannot claim production completeness. |
| F2.4 quantified replan/throughput tradeoff | **NOT NORMATIVE**, but clock regimes and planning latency must be explicit (finding 13). |
| F2.5 dynamic realization order | **SILENT**; required if duration drift can reorder dependencies. |
| F3.1 solver remains a replaceable policy | **PRESERVED**. |
| F3.2 one-shot and rolling boundaries can share a kernel | **PRESERVED**. |
| F3.3 reallocation before physical binding | **NO SURFACE**; acceptable deferral, but not forward-compatible by assertion alone. |
| F3.4 online assignment policy | **OUT OF SCOPE**, explicitly document the deferral. |
| F4.1 compensation must cover the committed prefix | **PARTIAL/BROKEN** (finding 5). |
| F4.2 leases need total ownership and disposition | **BROKEN** by caller-owned discard (finding 5). |

### Prior adversary findings

| Adversary finding | Disposition in v1.1 |
|---|---|
| F1. atomic pop cannot await/replan mid-commit | **PRESERVED IN LANE 1**, but fallible `on_commit` and projection apply reopen the same atomic boundary after selection (findings 1 and 6). |
| F2. within-pop convergence and finite live menu | **PARTIAL** — fixed-epoch ratification remains; zero-time spawn/fire/Continue introduces a new unbounded same-instant loop (finding 14). |
| F3. additive deadline pressure defeats occupancy feedback/anti-windup | **SILENT/OPEN** under OQ6; hard feasibility classes and stabilizers are required before authority (finding 11). |
| F4. leaf-only commitment removes the relay pre-stage buffer | **SILENT FOR PRODUCTION NOISE**; zero-noise parity may pass while jitter slack disappears. Carry one-ahead/safety-envelope evaluation into the future window/noise program (finding 13). |
| F5. watermark versus clock-advance deadlock | **SUPERSEDED WITH RATIONALE** — the accepted substrate uses fixed-epoch pool completion rather than the rejected global demand-watermark protocol. Any future overlapped window mode must reintroduce an eager next-contribution rule explicitly. |
| M3. generation-only hot-bank invalidation is quadratic | **PRESERVED IN THE WINNER'S INTERVAL INDEX EVIDENCE**; keep K>=8 contention and density probes. |
| L1. VTM score may never own a forcing point | **PARTIAL** — demand licensing is the intended home, but raw fires do not prove that the VTM objective participates (finding 11). |
| L2. noise dirtiness of committed versus tentative work | **DEFERRED**; Appendix A must not imply that typed time alone supplies repair (finding 13). |

### Program context and motivating handoff

| Program record | Disposition in v1.1 |
|---|---|
| planner-async T1: defer-versus-greedy | **PRESERVED AS EXPLICIT POLICY INTENT** through accumulators, but firing/admission must be licensed so deferral is not an accidental Supervisor race. |
| planner-async T2: poll order becomes policy | **PARTIAL** — stable sorting exists, but hierarchical IDs and multiple fires can turn construction/poll order back into policy (findings 9 and 11). |
| planner-async six wait families / clock analysis | **SUPERSEDED FOR LANE 1** by kernel phases; **PRESERVED FOR LANE 2** only through sealed kernel-owned suspensions (finding 16). |
| planner-async selected-candidate preparation | **DELIBERATELY UNSUPPORTED BUT UNSTATED** — document the synchronous compute-budget decision and extension seam (finding 16). |
| VTM schedule-gaps handoff | **DIRECTLY ADDRESSED IN THE TARGET MODEL** by lazy time-indexed claims; **NOT YET WALKABLE** while master `available_at` remains authoritative (finding 12). |
| gate evidence record | **PRESERVED** as the empirical release contract; it supports parity/performance, not the untested capability, cancellation, epoch, or window-noise claims in this review. |

## Ratification conditions

Before ratification, close findings 1, 2, 4, and 5 in the normative API and lifecycle,
and make empty/armless `Park` a kernel fault (cross-examination C11 below).
Before the first production-authoritative slice, close findings 3 and 6-13 and add their
adversarial fixtures. Findings 14-16 may land incrementally only with named failure modes
and release gates.

## Independent-review boundary

Everything above was drafted before reading `two-lane-claude-review.md`. The following
cross-examination section is intentionally appended only after that independent case was
fixed.

## Cross-examination of `two-lane-claude-review.md`

Claude's file has nine initial findings, four corpus-sweep findings (10-13), an ergonomics
meta-finding, and five ergonomics findings (14-18). The dispositions below cover each one.

### Initial findings 1-9

| Claude finding | Disposition | Cross-examination |
|---|---|---|
| 1. `on_commit` inside ratification | **CONFIRM** | Independently found as finding 1. Post-commit must mean after all kernel/domain writes, with no newly woken task polled until the deterministic post-commit queue drains. |
| 2. lost wake in `Needs.conds` | **CONFIRM DEFECT; PARTLY DISPUTE REMEDY** | Dropping `conds` from `Needs` is the safest minimal v2 repair because today's `Domain::Cond` has no level predicate. A future typed `Level` leaf can be sound if sampling and arm registration are atomic. The broader rule is not "conditions never belong in Needs"; it is "an after-arm edge cannot stand in for a prerequisite level." |
| 3. hierarchical ID order | **CONFIRM** | A separate registration/apply index is necessary. Claude does not cover same-parent concurrent allocation, sibling batch minting, path depth/overflow, or reparenting changing policy; independent finding 9 adds those constraints. |
| 4. Lane-1 read surface | **CONFIRM GAP; DISPUTE `&World` FIX** | A borrow-scoped `&World` prevents retention but not stale decision reads between planning and ratification, and it exposes a much larger authority surface. Use narrow stamped oracle/query tokens or a validated world generation. The Presentation example should use that surface, not an unrestricted world borrow. |
| 5. Supervisor incapability overstated transitively | **DISPUTE SEVERITY** | This is more than a compute/admission note. Unconstrained `D::View`, raw `fire`, and unrestricted task spawning form capability-smuggling and proxy-commit paths (finding 4). CONWIP tokens help admission, but typed spawner/demand/view capabilities are required to justify the lane claim. |
| 6. copied view data can go stale | **CONFIRM** | A prose rule against cached world reads repeats the convention problem. Tracked read tokens and staged task-local state are stronger repairs (findings 3 and 10). |
| 7. replan-cause delivery | **CONFIRM GAP; DISPUTE LATEST-WINS** | Coalescing to only the latest cause can hide another invalidated resource or producer transition. Deliver a deterministic cause/wake set (or a lossless ordered queue with a documented reduction), not merely "at least the most recent." |
| 8. banked strangler patches may not rederive | **CONFIRM** | Reword them as banked designs and begin with a rebase-feasibility spike. This is necessary but not sufficient: even perfectly rederived mechanical patches do not make the scalar-VTM/lazy-timeline one-leg authority boundary sound (finding 12). |
| 9. claimed unchanged survivors | **PARTLY DISPUTE** | The lane split, synchronous Task planning, and two-PR-species migration frame survive. Typed instants, epoch facts, future supply, and kill obligations do **not** survive unchanged: findings 5, 7, and 8 identify contract failures in each. Direct construction also needs non-forgeable/validated handles. |

### Corpus-sweep findings 10-13

| Claude finding | Disposition | Cross-examination |
|---|---|---|
| 10. distinguish the Task trait from the Round-2 callback conviction | **CONFIRM LINEAGE; PARTLY DISPUTE ROUTING RULE** | Defaulting `on_commit` to `Done` and citing the prior conviction are good. "Use a Supervisor at three increments" is too mechanical: rolling entitlement may require one resource identity across many commits, while a Supervisor is intentionally resource-incapable. Prefer one-shot children when increments are narratively distinct; permit multi-shot Tasks when one entitlement/owner genuinely persists, with a cold-read complexity gate. Appendix A must choose its production factorization based on those semantics, not an arbitrary count. |
| 11. armless `Park` must fault | **CONFIRM; MISSED IN INDEPENDENT PASS** | Adopt as a ratification condition. The kernel must reject `Park::Until([])` and a Supervisor `Pending` without an owned suspension token. This strengthens independent finding 16. |
| 12. snapshot cost bound is silent | **CONFIRM REQUIREMENT; REFINE PRESCRIPTION** | The spec needs a cost budget and regression probe. It may cite the winner's CoW evidence, but should not mandate CoW as the only implementation. More importantly, cheap freezing does not validate non-slot reads; independent finding 10 still applies. |
| 13. two-lane split has strong Round-1/Round-3 lineage | **CONFIRM** | This is useful rationale, not a defect. It strengthens ratification of the factorization while leaving the concrete API blockers intact. |

### Ergonomics meta-finding and findings 14-18

The meta-finding is **confirmed**: §12 omits the previously weighted cold-read and ceremony
criteria. Ergonomics should be added to the v2 review register. That does not change this
packet's method; it adds an evidence-backed authorability dimension to the design review.

| Claude finding | Disposition | Cross-examination |
|---|---|---|
| 14. single-increment artifact count drops | **CONFIRM DIRECTIONALLY** | The proposed Task surface removes the builder field shuttle, async wrapper, and service plumbing for ordinary one-increment work. Exact line-count savings remain implementation-dependent, but the structural reduction is real. |
| 15. cold-read grades | **PARTLY CONFIRM** | The one-increment and Supervisor shapes are promising. Presentation passes only after exact epochs and level/edge gating are fixed. The `>=3` multi-increment failure threshold is a heuristic, not a semantic boundary; use the entitlement/narrative rule from C10. |
| 16. one-sentence Needs/Park rule | **DISPUTE WORDING** | "If you can name the gate without planning, use Needs" would encourage the very lost wake in finding 2. Safer: "Declare immutable or exact-epoch prerequisites in Needs; for mutable world state, check the level in `plan()` and then Park on its next transition." |
| 17. pre-plan reporting improves | **CONFIRM** | Declarative fact/time prerequisites make never-planned tasks more diagnosable. The benefit depends on rendering exact epochs and retirement outcomes, not only raw task IDs. |
| 18. residual micro-ceremony | **CONFIRM AS MINOR** | Registration-time accumulator binding is worth considering, but it is also an authority decision: the rig must prevent a task from rebinding itself to an unlicensed end. Typed per-edge facts remain a compatible future improvement. |

### What the comparison review found that this independent pass missed

1. The explicit Round-2 callback-conviction comparison and the value of a default
   `on_commit -> Done`.
2. The need to reject empty/armless `Park` directly, not merely bound zero-time cycles.
3. The requirement to carry the frozen-snapshot cost model and its performance probe into
   the normative evidence register.
4. The omitted ergonomics criterion and the concrete single-increment ceremony win.

### What this review found that the comparison review missed or understated

1. `D::View` can carry an actual mutable resource capability; raw `fire` and unrestricted
   spawning make the Supervisor boundary structurally porous.
2. "Explicit discard" defeats total compensation ownership and clean quiescence.
3. Projection prevalidation needs a generation-bound prepared token and union transaction;
   a fallible callback is not the only atomicity hole.
4. The public fact API cannot address exact epochs, and `ProducerRetired` has no typed
   accessor path.
5. `Actual` conflates committed schedule truth with observed physical truth.
6. Hierarchical IDs need exclusive same-parent allocation, batch minting, path limits, and
   identity/priority separation.
7. Lane-1 domain reads need commit-time validation, not only retention safety.
8. The scalar VTM frontier makes the first authority boundary a vertical-slice problem,
   not merely a rebase risk.
9. Appendix A still lacks bounded extension demand, clock-regime choice, ADG acyclicity,
   drift/robustness pricing, and a selected factorization.
10. Same-instant convergence bounds and rig-level credit/obligation accounting are absent.

## Final recommendation after cross-examination

The comparison strengthens, rather than changes, the verdict. The factorization deserves
a v2 revision; the v1.1 API does not deserve implementation yet. Fold both reviews into a
single normative v2, then run a short paper adversary against the repaired traits before
the rebase-feasibility/shadow-kernel PR. The first implementation gate should test the
contracts, not only whether the traits compile.

---

## Rebuttal to Claude Extension 3

Claude's Extension 3 accepts all sixteen substantive findings and aligns on the verdict.
There is therefore no competing architectural recommendation to rebut. The remaining
differences are contract details; those details matter because v1.1's governing principle
specifically rejects obligations left to author memory.

### R1. A constructed task is inert with respect to scheduling, but construction is not infallible

I accept Claude's correction that the post-commit design does **not** require templates.
`SpawnSpec` was intended to mean inert registration input, not restoration of a closed
task enum. A directly constructed `Box<dyn Task>` can satisfy open-world DI.

The word "inert" does not close the failure path by itself. Task construction can allocate,
panic, validate dependencies, or execute arbitrary constructor code. If construction occurs
after the parent fact commits and fails, the kernel now has a committed parent whose required
successor was not registered. That is acceptable only with explicit semantics:

1. the transaction remains committed and is never rolled back;
2. successor construction/registration runs in the deterministic post-commit phase;
3. failure becomes a typed `PostCommitSpawnFault` attached to the parent/root;
4. clean quiescence cannot report success while a required spawn fault is unresolved; and
5. no newly woken consumer is polled until the post-commit queue has either registered its
   children or recorded those faults.

So the agreed implementation can be direct boxed tasks, but "constructed tasks are inert"
is not a substitute for post-commit failure ownership.

### R2. Minimal v2 may amputate condition leaves, but exact facts are not a future-window feature

I accept the minimal repair: v2 can remove `Needs.conds`, retain all-of fact/time needs, and
defer `Level`, `AfterNext`, and `Any` until a concrete driver appears. The full algebra was a
semantic model, not a demand to ship unused variants.

`FactExact`/`NextAfter` cannot be deferred with the windowed program, however. Section 6.6
already makes multi-epoch tasks part of the base Task contract, and a child can otherwise be
woken for epoch *n* but read epoch *n+1*. The minimum coherent v2 surface is therefore:

- exact or next-after fact references whose satisfying value is captured at wake;
- absolute `Deadline`/committed-time prerequisites with the corrected time type; and
- all-of composition.

Level predicates, edge conditions, and any-of composition may be staged. If they are staged,
the spec must say initial mutable-world gating is expressed only as `plan()` level check then
`Park` on the next transition.

### R3. `plan(&mut self)` plus documentation/lint is incompatible with the spec's own directive

This is the principal remaining disagreement. Claude proposes documenting and heuristically
linting the rule that state carried between `plan()` calls must not contain cached world reads.
That is another remember-to-do convention, in the exact area—retry and staleness—that produced
the bakeoff's failures.

The problem is broader than cached reads. A failed tentative attempt can mutate a phase,
consume a queue item, increment a retry counter that affects score, or replace an identifier.
Whether those mutations survive invalidation then depends on author discipline and worker
interleaving. A lint cannot prove semantic rollback, and a doc comment cannot preserve
bit-identical replay.

This need not tax the single-increment ergonomics win. Viable structural choices include:

- `plan(&self, ctx) -> Planned { plan, next_state }`, with `next_state` installed only after
  commit;
- kernel clone/stage of a compact task decision state, with accepted state swapped in; or
- separate immutable task identity/configuration from an explicit `CommitState` returned by
  planning.

A stateful escape hatch can be named in §10 for exceptional tasks, but the default trait should
not make persistent failed-attempt mutation the easy path. The packet therefore maintains
finding 3's staged-state recommendation.

### R4. Value-level spawning capability is the better open-world implementation

Claude is right that `Spawner<AllowedTaskKinds>` would be too closed if interpreted as a
compile-time enum of permitted concrete kinds. The requirement is scoped authority, not a
closed type registry.

Use a rig-minted value capability such as `SpawnPermit { root, claim_scope, demand_scope,
budget }`. It may accept any DI-constructed task, while registration and ratification enforce
that descendants stay within the permit's declared resource/demand scope. This preserves
open-world construction without reducing "resource-incapable Supervisor" to a convention.

### R5. There is no disagreement about `TaskHandle`

Claude disputes making `TaskHandle<F>` mandatory, but the independent finding explicitly
offered two valid repairs: a generative/non-forgeable handle **or** validation of every
dependency in the spawn transaction. `SpawnCtx::reserve()` plus validate-at-registration is
a good value-level implementation, provided reserved sibling IDs cannot escape after a
failed batch and facts remain epoch-typed.

### R6. Source-scoped read stamps are the production answer; coarse generation is only a correctness floor

Agreed: bumping one world generation on every playback mutation would cause near-total
invalidation and could erase the winner's planning-density gains. It was listed as a coarse
correctness fallback, not the preferred design. Production authority needs source-scoped
tokens for inventory, topology, ownership, and other decision-bearing reads, with a measured
fan-out/cost bound.

It is reasonable for v2 to state that contract and leave exact token granularity to the
pre-cutover design spike. It is not reasonable to expose unrestricted `&World` in the
ratified author API and hope granularity can be added without changing it.

### R7. Clarify what is being ratified

Claude's union places the four primary blockers plus armless Park before ratification, and
the remaining HIGH findings before production cutover. That matches this packet only if
"ratification" means **ratification of the two-lane factorization**, not ratification of
v1.1 as an implementable API specification.

The distinction should be explicit in the verdict record:

- **Factorization ratification:** allowed after the primary blockers, armless-wait rule, and
  open-world capability direction are resolved.
- **Trait/API ratification for implementation:** also requires `PlanCause`, exact fact refs,
  corrected time types, post-commit failure semantics, projection transaction shape, and ID
  identity/order rules. Otherwise the first implementation necessarily invents normative
  behavior.
- **Production-authority gate:** additionally requires source-scoped read validation, demand
  licensing/dynamic policy, the vertical-slice migration boundary, credit/obligation audits,
  and the Appendix A/noise items relevant to the slice being cut over.

### Rebuttal conclusion

The two reviews now converge on the design direction and nearly all mechanics. Fold the
agreed union into v2, resolve R3 explicitly rather than leaving mutable retry state as a lint,
and label the three ratification levels above. With that, the next useful adversary pass can
focus on the repaired trait definitions instead of reconciling reviewer vocabulary.
