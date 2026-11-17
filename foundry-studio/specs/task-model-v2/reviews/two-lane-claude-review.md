# Two-lane spec — Claude's adversarial self-review (draft v1 @ refactor 08308b0c)

Reviewed 2026-07-20, same criteria as Sol's packet (§12). Verdicts:
CONFIRMED (defect with a concrete path) / GAP (underspecified, must be
defined before implementation) / PLAUSIBLE (needs measurement) / NOTE.

## 1. `on_commit` inside the ratification transaction is a self-inflicted Sol-finding-1

**CONFIRMED — spec defect (§3, OQ-1).** The draft runs user code
(`on_commit`: spawning, `Next` decisions) inside the atomic section whose
own contract is "cannot fail, must not search." A panicking or slow
`on_commit` recreates the partial-transaction hole Sol's blocker 1 just
closed — worse, spawning mid-transaction runs `needs()` (more user code)
against kernel state mid-write. The winner never does this: spawns are
yields, applied by the kernel outside ratification. **Fix: `on_commit` runs
after the transaction commits, same instant, before the next settle pass.**
Atomic-spawn-with-commit is not a real requirement (children are
independent tasks). OQ-1 should be answered "after," and §3's "runs inside
the ratification's apply phase" line deleted.

## 2. `Needs.conds` is an edge-triggered lost-wake class

**CONFIRMED — spec defect (§3.1).** Conds are versioned at registration =
"wake on the NEXT transition." A condition that fired before the task
registered never fires again for it → forever-gated. The winner avoids this
because world-condition gates are always issued from running code that can
check the level first (the Presentation pattern: check `at_cell`, then
park). `Needs` has no level-check hook, and `Domain::Cond` deliberately has
no executable predicate. **Fix: drop `conds` from `Needs` entirely —
`Needs = { facts, after }`.** Facts are level-safe (committed = satisfied,
whenever you look). ALL world-condition gating goes through `plan()` +
`Park::Until`, where the level check is natural and the winner-proven
version-stamp-at-park applies. This also simplifies §6.1.

## 3. Hierarchical ids break "id order == registration order"

**GAP (§5, §6.2, OQ-8).** The winner's determinism leans on stable apply
order and first-registered demand service, both riding monotonic ids.
`(parent, seq)` ids sort lexicographically ≠ registration order (a late
child of an early parent outsorts an early child of a late parent) — which
silently changes fire order, an F4-relevant policy surface. **Fix: separate
identity from order** — keep hierarchical ids for identity/tree/cancel;
keep a kernel-assigned monotonic registration index for apply order and
any registration-order policy. Must be in the spec, not discovered later.

## 4. Lane-1's read surface is undefined and the Presentation example exposes it

**GAP (§3, §8).** The example calls `ctx.view().at_cell(...)` — but `View`
is specced as "counters and flags, cheap," and robot positions would drag
it toward world-sized. The winner's `PlanCtx` already holds `&D::World`
read-only; lane-1 `plan()` is a borrow-scoped call, so exposing a read-only
`&World` is retention-safe by construction. **Fix: `PlanCtx` exposes
`world(): &D::World` (read-only) + snapshot + facts; `View` remains the
supervisor-side owned summary only.** Example updated accordingly.

## 5. Supervisor "resource-incapability" is true directly, overstated transitively

**NOTE (§4).** A flooding supervisor can spawn unbounded lane-1 tasks whose
plans bloat menus and planning compute — commitment stays demand-gated, but
decomp's 720-parent perf wall shows the compute cost is real. Admission
pacing remains authorial (the OrderBook-cap convention), contra the lane's
absolutist framing. Either soften the claim, or add the structural option:
admission as a claimable resource (CONWIP tokens as kernel resources) —
worth an OQ, not a blocker.

## 6. Copy-out staleness survives the borrow

**NOTE (§10).** `plan(&mut self, …)` can't retain `ctx`, but it can copy
ctx data into `self` and reuse it stale next call (the Sol-finding-9 View
class, relocated). The purity convention (§10.1) should name this
explicitly: *state carried across plan() calls must be decision state, not
cached world reads.* Lintable heuristically; not typable.

## 7. Replan-cause delivery is underspecified

**GAP (§3, §6.1).** Multiple invalidations can accumulate between plan()
calls (winner delivers one pulse per poll; lane 1 has no polls). Define:
causes coalesce, latest-wins, `PlanCtx::cause() -> Option<Invalidation>`
documented as "at least the most recent." One sentence, but it must exist.

## 8. §9's banked steps assume strangler patches re-derive on today's master

**PLAUSIBLE.** Strangler seq 1–2 landed off f4332606; master has moved
(metrics/observer churn per the round-3 provenance note, possibly more
since). The first program PR should be a rebase-feasibility spike of
CommitTx, not a promise. Reword "known banked steps" to "banked designs,
re-derivation to be sized."

## 9. What survives review unchanged

The lane split itself (with #5's softening); templates→SpawnCtx direct
construction (with #3's index fix); needs/Park two-stage gating (with #2's
amputation); folding PlanBuilder into Task; pulse deletion for lane 1;
typed instants (§7); epoch facts, typed producer-death, time-typed supply,
kill-returns-obligations (§6.6–6.7); the two-PR-species migration frame
and its gates (§9); the accepted-conventions register existing at all
(§10).

## Disposition

Draft v1 is reviewable but not implementable: findings 1–2 are design
defects, 3/4/7 must be specified before any code. Recommend folding all
fixes into SPEC v2 after Sol's pass lands, so both reviews hit the same v1
text and v2 answers both at once.

---

# Extension: §12.1 corpus disposition sweep (2026-07-20, post-v1.1)

Method: re-read of the decided-finding registers — round-1 SCORECARD (incl.
addendum + D11), ERRATA E1–E4 + the seven convergent findings, SYNTHESIS,
ROUND2 §5 audit A1–A7/T-requirements, round-2 SCORECARD (nine-fault
regression table) + E5–E7, round-3 SCORECARD/VERDICT/E8–E9, round-4 complete
(written from it), both Sol reviews, adversary findings (via the DESIGN
record), planner-async tensions and vtm-schedule-gaps (via the program
record), lit re-review at headline level (flagged as such). Dispositions:
PRESERVED / SUPERSEDED(rationale) / SILENT(+risk) / NEW FINDING.

## New findings from the sweep (continue v1 numbering)

**10. The lane-1 trait is ADJACENT TO A ROUND-2 CONVICTION and must be
distinguished explicitly — LOAD-BEARING.** R2-B was convicted for exactly a
`plan` + `on_commit` (+ stash/take_wait/done) multi-method callback protocol
(round2/SCORECARD: "the same hand-written state machine with a clunkier
protocol"), and SCORING §3a floor-scores "multi-method callback protocols
(plan/on_commit/take_wait/done)" BY NAME. Adjudication: the spec is
distinguishable, but only if v2 does three things. (a) Default `on_commit`
to `Next::Done` — a single-increment kind (the overwhelming majority) is
then `needs` (data) + one `plan` fn: no wait-stash (Park is plan's RETURN
value), no commit-pulse stash (the fact arrives as an argument), no retry
lattice (kernel-owned per the round-3 repeal of in-body loss loops) — every
element of the conviction absent. (b) RECOMMEND parent-supervisor +
one-shot-children over `Continue` for any life with ≥3 increments —
`Continue` phase-state in the struct IS R2-A's "the rich task is still a
state machine" (its own ugliest-point #1); the narrative belongs in lane 2
where round 3's verdict holds. Appendix A must be reframed: windowed
pathfinding's PREFERRED spelling is a supervisor + one-shot window tasks
(Sol f16 row 3); multi-shot `Continue` is the compact fallback, not the
headline. (c) Cite the conviction in §3 so the boundary survives review
drift.

**11. Armless park must be a kernel-rejected error** (round-1 convergent
finding 4, unanimously decided): `Park::Until(vec![])` — a park with an
empty wake set — must fault, not silently zombie. Spec v2: one sentence in
§3.2. Lane 2 inherits the adapter's existing armless-park fault.

**12. The snapshot cost model is a chartered requirement the spec leaves
silent** (round-3 E9.1: "a versioned read-set or copy-on-write snapshot
with a stated cost bound" — resolved empirically by decomp's CoW, R4
sealed). §6.2 must state: per-batch frozen snapshot is CoW with O(delta)
fork cost, and the §12.1-grade evidence (113–249× vs deep clone) is the
bound's provenance.

**13. Provenance strength (not a defect): the two-lane split is the round-1
SYNTHESIS position, corrected by round 3, sharpened to capabilities.** The
synthesis's five arguments for turns-as-data (playback is model-side; every
production property — inspect/serialize/diff/EDIT for repair — lands on the
data side; legibility was about typed seams, not encoding; temporal
discipline runs against free-running bodies; pathfinding needs no bodies)
are lane 1's evidence base verbatim. Round 3 proved narrative value where
lives are long (parents/daemons) — lane 2 keeps exactly that. E6's
corrective (loss-visible planning) is honored post-repeal: the kernel owns
the loop, the typed cause is delivered to `plan()`. v2's §1.1 should carry
this lineage — it is the strongest available answer to "why is this not
just round-1 again."

## Disposition tables (terse; row = finding → disposition)

**Round-1 errata/convergent (E1–E4, C1–C7):** E1 policy-at-simultaneity →
PRESERVED (§6.2 pop-at-fixpoint; §6.4 licensing is its production form).
E2 canonical S5 form → SILENT-compatible (belongs in the migration ladder's
probe definitions — carry it there). E3 historical. E4/T-series → PRESERVED
(substrate + §7). C1 transition-arms-not-conditions → my finding 2's
`Needs.conds` amputation is REQUIRED by this decided rule (edge-only
needs-conds violated it; Park's level-check-then-arm idiom complies).
C2 joint-feasibility replans → PRESERVED via cause delivery; finding 7's
coalescing gap must resolve as "cause consumed, plan-around enabled".
C3 committed-record-outlives-retirement → PRESERVED (§6.6). C4 armless-park
→ NEW finding 11. C5 sim-stable tie-breaks → my finding 3 (identity/order
split) is REQUIRED by it. C6 demands-as-substrate-participants → PRESERVED
(supervisors are table tasks, reported). C7 increment identity owed a
first-class story → ADDRESSED (§6.6 epochs + §6.7 obligations) — the spec
is the redraft's answer; reviewers should judge sufficiency.

**Round-1 scorecard observations:** data-shaped seams dominate encoding →
lane 1 IS that conclusion; apply-as-data (the I1 fork) → PRESERVED (Effect
enum; apply_write a method, never a closure); §7-clock-rule-unexercised →
retired by R4 (pool exercised, enforced per §6.2).

**Round-2 nine-fault regression table:** none resurrected — frontier-hold
(Hold is a table row, unchanged); fleet-outside-substrate (party holds);
zombie-dirty rescore (upsert rescoring unchanged); brick-bypass
(timestamped external mutation unchanged); plan-can-sleep → STRENGTHENED
(lane-1 PlanCtx has no temporal surface AND no awaits exist); lodged
proposals outliving owner (reap-on-Done unchanged; my finding 1's fix keeps
on_commit outside the transaction, preserving the reap boundary);
milestone-leak (source-scoped versions unchanged); retirement-withdraw
(unchanged); apply-as-closure (dead, stays dead).

**Round-2 records:** E5 historical. E6 loss-blindness → HONORED-POST-REPEAL
(see finding 13). E7 naming split → COMPLETED STRUCTURALLY (§7). Doctrine
#9 disjoint ctx capabilities → PRESERVED/strengthened. T1–T7 → PRESERVED
(generic Time; timestamped apply_write; intervals-not-levels via claims;
stamped facts; pop-time expiry + reanchor + Blocked/Stale; typed Hold +
atomic close; window-driven playback — no body-side playback exists at
all).

**Round-3 findings/errata:** F1/E8 baseline-must-run → migration probes use
the fitted A2 book (compatible; ladder doc carries the law). F2 canonical
A2 → superseded by E10 (parity) — probes pin same-physics. F3 unsafe
panic-path → MOOT (no unsafe; deletion doctrine stands). F4 global wake
version → PRESERVED (source-scoped versions are kernel vocabulary).
F5 head-on swaps → MOOT (real FH-SIPP is the planner). F6 reused-source
premature-wake → STRENGTHENED (stamp-at-park is kernel-side in lane 1; no
adapter in the loop). F7 discarded propose outcome → DISSOLVED in lane 1
(no propose await exists; the kernel delivers the cause). F8 snapshot cost
profiles → NEW finding 12. E9.3 quote-gate doctrine → STRUCTURAL (§7).

**Round-4 (SCORECARD 8, E10–E12, GATE):** the spec was written from these;
blockers → §6.5/6.6/6.7; density/licensing → §6.4; E10 parity → probe
pins; E11 provenance → probes are phase2-weighted; E12 rig silent-accept →
subsumed by fail-on-faults. SILENT on: planning-pool underutilization
(perf headroom — acceptable silence) and F9/F10 scoring home (OQ-6, held
open deliberately).

**Sol 20 + 22:** dispositioned in the round-4 SCORECARD map and
CONSOLIDATION_LOG; spec-deltas: f4→`bundle_with`+cycle rejection; f5→§6.2
enforcement; f8→§6.4 licensing; f9→lane-1 structural + finding 6 residue;
f11→§6.6 epochs; f12–f20→Appendix A as reframed by finding 10.

**Adversary findings (round-1 era):** F2 skip-stale → kernel (preserved);
F3 scoring stabilizers MUST port pre-production → STILL OPEN (OQ-6 — the
sweep re-flags it; the spec must not ship a production-authoritative book
without answering it); F4 per-zone replenishment demand → daemons preserved
(and §6.4 gives it a licensing home); F5 clock-rule = pool-compute-only →
preserved; M3 interval index day-one → preserved (resource table).

**Lit re-review (headline-level — full re-read not performed this pass;
flagged per §12.1 honesty):** commit-at-start free-given-slack (Eberle) →
the pop IS commit-at-start (preserved); chain-registration-binding = worst
row → per-leg default preserved; ADG/entitlement + SADG dynamic turn order
+ k-robust drift pricing → a future execution-noise program; the spec's E7
typing is its prerequisite and nothing forecloses it; TPTS
slack-gated reallocation → belongs to the solver/licensing program (§6.4
compatible); K-swap 10–20% stranding → solver program, consistent.

**planner-async (T1 defer-vs-greedy, T2 poll-order-as-policy):** PRESERVED
— both tensions become explicit policy surfaces (accumulator licensing +
SequencingPolicy) rather than emergent properties. **vtm-schedule-gaps:**
the motivating defect; §1 carries it; the density evidence is the spec's
reason to exist.

## Sweep verdict

Nothing in the corpus contradicts the lane split; the synthesis lineage
positively supports it (finding 13). The spec is NOT implementable as v1.1:
v2 must fold v1 findings 1–4/7 plus sweep findings 10 (conviction
distancing: default on_commit, prefer parent+children, cite), 11 (armless
park), 12 (snapshot cost clause) — and re-frame Appendix A per 10(b). The
two obligations the corpus keeps alive that no spec section yet answers:
scoring stabilizers (adversary F3 / OQ-6) before any production-
authoritative book, and the migration ladder inheriting E2/E8's
baseline-law when it is written.

---

# Extension 2: the ergonomics pass (Daniel's probe exposed its absence; run against SCORING §3 — the #1-weighted dimension)

**Meta-finding: §12's review criteria omit ergonomics entirely** — a packet
defect; v2's criteria must add "grade the §8 shapes by SCORING §3 cold-read
and ceremony budget." (Sol is mid-run on the defective criteria; this pass
compensates.)

## 14. Per-kind artifact count: single-increment kinds get MATERIALLY lighter

**Measured against the winner's real HtmDeliver** (task struct 5 fields +
manual Debug + Task impl with Box::pin wrapper + separate DeliverLeg builder
struct with NINE fields + PlanBuilder impl ≈ 150 lines, 5 artifacts): the
two-lane equivalent is one struct (2 fields) + one impl (label, needs, plan;
on_commit defaulted per finding 10a) ≈ 50 lines, 2 artifacts. Three ceremony
classes DISSOLVE: (a) the field-shuttle — the winner copies
tote/slot/access_cell/deposited out of the awaited fact into builder fields;
plan() reads the fact from ctx directly; (b) the `services: Arc<WhServices>`
plumbed through every struct — PlanCtx carries world/services access;
(c) the Box::pin/async wrapper. This is an ergonomic WIN the v1 review never
stated.

## 15. Cold-read grades of the §8 shapes (SCORING §3a: narrate the life from the code alone)

- `VtmForward` (no needs): one breath. PASS.
- `HtmDeliver`: "needs the pickup fact; plans the carry leg." PASS.
- `Presentation`: the two-stage gate (fact need → level check → Park on the
  arrival transition → dwell) is now COLOCATED in one function — arguably
  more narratable than the winner's body-await + separate-builder split.
  PASS.
- `OrderLine`/`FlowPump` supervisors: read as the winner's parents/daemons.
  PASS (round-3's win preserved where it lives).
- Multi-increment `Continue` in lane 1: cold-read FAIL at ≥3 increments
  (phase-in-struct) — already adjudicated; finding 10(b)'s routing rule is
  the mitigation, and the ergonomics pass CONFIRMS it is load-bearing, not
  optional.
- Residual reader burden: "plan() may be called many times" lives in trait
  docs, not at call sites (the winner's `ctx.commit` made retry locally
  visible). Mild; the §6.1 lifecycle table is the mitigation.

## 16. The needs-vs-Park author decision rule is teachable in one sentence

"If you can name the gate without planning, it's `needs`; if you discover it
while planning, it's `Park`." Wrong-direction errors are loud, not silent:
omitting a fact from needs then calling `ctx.fact(unlisted)` is a kernel
fault (§3.1); putting a computable gate in needs is impossible (finding 2
amputated conds). Cost: low. Should be stated verbatim in §3.2.

## 17. Report ergonomics improve pre-plan

A gated task renders its `needs` as data BEFORE plan() has ever run — the
winner's never-polled task showed only its spawn record. Parked rows carry
typed wake sets as before. Net win; no action.

## 18. Micro-ceremony still unaddressed (carry to v2 as small OQs)

(a) `home: AccumulatorId` plumbing survives in every Plan::new — could move
to registration-time metadata (task→accumulator binding declared once);
(b) fact-shape destructure `else Fault` arms persist exactly as in the
winner (typed per-edge facts remain future work — same accepted position);
(c) `Score::by_time` and `label()` ceremony unchanged.

## Ergonomics verdict

Single-increment kinds (the corpus majority): clear improvement (14, 15,
17). Supervisors: parity with the round-3 win. Multi-increment lane-1: the
one regression, contained by finding 10(b). The pass RAISES finding 10 from
"distancing required" to "distancing is also the ergonomic optimum." New v2
obligations: add ergonomics to §12 criteria; state 16's rule; consider
18(a).

---

# Extension 3: cross-examination of two-lane-sol-review.md

Read after Sol's file landed (its own cross-exam of this file honored the
blind protocol). Dispositions of its 16 findings:

| Sol | Disposition | Notes |
|---|---|---|
| 1 on_commit in txn | **CONFIRM (convergent with my 1)** | Accept both refinements: post-commit queue drains before any newly-woken poll; on_commit sees fact + task-local only. One guard on its "inert SpawnSpec" phrasing: constructed `Box<dyn Task>` IS inert until registered — staging constructed tasks post-commit satisfies this WITHOUT re-inventing templates. |
| 2 Needs lost-wake | **CONFIRM (convergent with my 2); accept its remedy refinement** | Amputation is the v2 repair; record the future `Level` leaf as sound iff sampling+arm are one settle operation. Its rule-statement ("an after-arm edge cannot stand in for a prerequisite level") is better than mine — adopt verbatim. DISPUTE scope: shipping the full readiness algebra (Level/AfterNext/FactExact/All/Any) in v2 adds surface without a present driver — stage it with the windowed program. |
| 3 PlanCause absent from API | **CONFIRM** — sharpens my 7; accept its dispute of latest-wins (deliver a deterministic cause SET or ordered reduction). On staged-vs-`&mut self` plan state: document the hazard + lint (my 6); a staging API would tax the finding-14 ergonomics win. |
| 4 Supervisor capability smuggling | **CONFIRM — Sol's biggest add** (my 5 understated it: unconstrained `D::View` can carry a live resource capability; raw fire; spawn-proxy). Accept sealed `SupervisorView` + licensed fire + scoped spawner, with one load-bearing dispute: implement scoping VALUE-LEVEL (rig-minted capability values), not type-level allowed-kind lists — `Spawner<AllowedTaskKinds>` re-closes the world Daniel's DI directive just opened. |
| 5 discard defeats obligations | **CONFIRM — my §6.7 wrote the escape hatch back in; embarrassing and correct.** `#[must_use]` KillOutcome, kernel-held obligations, typed dispositions incl. domain-VALIDATED SafeAbandon, quiescence fails while ownerless. Full adoption. |
| 6 prevalidation not atomic | **CONFIRM** — `PreparedProjection` generation-bound token, union-validated, consumed at apply. The mechanical completion of §6.5. |
| 7 fact API can't say epochs | **CONFIRM** — `FactRef{Exact,NextAfter,Terminal}`; wake captures (producer, epoch, value); retention until unreferenced; ProducerRetired typed, never a panicking accessor. |
| 8 Actual conflates committed/observed | **CONFIRM — the best catch against my §7.** Four-way `Quote/CommittedAt/ObservedAt/Deadline` supersedes my two-way split; future supply uses `CommittedAt` + atomic consumption (two-consumer race named). |
| 9 hierarchical-ID details | **CONFIRM adds over my 3** (exclusive non-Send SpawnCtx; path limits; identity/priority separation). On batch-mint loss: restore it value-level (`SpawnCtx::reserve()` mints before construction) where mutual sibling refs need it; linear chains don't. DISPUTE non-forgeable `TaskHandle<F>` as mandatory — validate-at-spawn achieves the guarantee without fighting Copy ergonomics. |
| 10 non-slot read validation | **CONFIRM (its original F7, still the deepest open thread).** DISPUTE feasibility of my coarse world-generation fallback (bumps every playback → near-total invalidation); source-scoped stamped reads are the real design. v2 names the contract and granularity question honestly as pre-cutover work, not solved text. |
| 11 demand licensing unenforced | **CONFIRM** — merges with 4; DemandHandle + one policy epoch per same-instant fires. |
| 12 one-leg cutover unsound | **CONFIRM — corrects my §9 deeper than my finding 8.** Adopt the migration ruling wholesale: shadow/no-authority kernel → mechanical extraction → ONE VERTICAL SLICE (resource + both handoffs) → gates → retire legacy truth; never two writable authorities for one interval. |
| 13 Appendix A overclaims | **CONFIRM; accept its C10 dispute of my ≥3-increment count** — the routing rule is semantic: one persistent entitlement/owner → multi-shot Task (with a cold-read gate); narratively-distinct increments → supervisor + children. |
| 14 zero-time convergence | **CONFIRM** (round-1 finite-rooted-work reopened; my sweep missed it). Budgets + typed NonConvergentInstant fault. |
| 15 credits/termination | **CONFIRM** (closes its round-4 finding 3 + E12 at the right layer: rig CreditPool + quiescence enumerating leases/claims/obligations). |
| 16 lane-2 foreign futures | **CONFIRM** — seal the awaitable surface to SupCtx futures (suspension token, fault otherwise); state explicitly that async selected-candidate preparation is unsupported. |

Its miss-lists run both directions and are accurate. Verdicts align:
REVISE-BEFORE-RATIFICATION == my not-implementable-as-v1.1.

## Union for SPEC v2 (the fold list)

Blockers (normative API/lifecycle before ratification): Sol 1/2/4/5 +
armless-park (my 11) — with my 1/2 folded into Sol's 1/2. Pre-cutover
HIGHs: Sol 3/6–13 + my 3/4/7 (as refined above) + sweep 10 (conviction
distancing, semantic routing rule) + 12 (cost model, cite-not-mandate CoW).
Incremental MEDIUMs with named gates: Sol 14–16. Ergonomics criterion +
findings 14–18 carried into §12/v2 text. Then Sol's closing protocol:
paper-adversary pass on the repaired traits BEFORE the
rebase-feasibility/shadow-kernel PR.

---

# Extension 4: disposition of Sol's rebuttal (final round — convergence)

R1 ACCEPT: direct construction stands; adopt the five post-commit
spawn-failure semantics (typed PostCommitSpawnFault; quiescence cannot
succeed unresolved; queue drains before woken polls). R2 ACCEPT: FactRef
{Exact, NextAfter, Terminal} is v2-mandatory (the wake-for-n-read-n+1 race
is base-contract); Level/AfterNext/Any stage with the windowed program,
with the mutable-world-gating rule stated. R3 CONCEDE THE SUBSTANCE,
COUNTER THE MECHANISM: adopt `plan(&self)` — purity by signature — with
ALL task mutation confined to `on_commit` (post-commit, once per committed
increment, deterministic). Structurally kills failed-attempt mutation and
replay hazard without Sol's Planned-wrapper or staged-clone options; zero
ceremony for single-increment kinds; attempt-sensitive scoring, if ever
wanted, becomes kernel-provided PlanCtx data, not self state. R4/R5/R6
CONVERGED (SpawnPermit value capability; reserve+validate-at-registration;
stamped source-scoped reads stated in v2, granularity to the pre-cutover
spike, no raw &World in the ratified API — my finding-4 fix is withdrawn
in favor of typed oracles). R7 ADOPT: the three-level ratification ladder
(factorization / trait-API / production-authority) goes in v2's status
header verbatim.

No open disagreements remain. SPEC v2 proceeds on this union.
