# Task-model v2 — master evolution plan (the operative punchlist)

**Authority**: `TWO_LANE_SPEC.md` (the design; this file is the execution
ladder its §9 defers to). **Managed phase by phase**: exactly one open
PR/branch at a time; each phase's punchlist is groomed when the previous
phase merges; this file is updated IN the PR that completes each phase.

## Operating rules (Daniel, 2026-07-20 — binding)

1. **Serial**: never more than one PR/branch queued at a time.
2. **Small**: smallest reasonable pieces that still advance to the goal;
   ideally **< 2 kloc per PR** (phases that risk exceeding are pre-split
   below; "kloc" counts hand-written diff, not lockfiles/goldens).
3. **Gated**: **every PR** passes the phase2 gate (below) before merge.
4. **Spec-first**: the spec + this plan land in PR 1 and evolve in-tree.
5. **Halt rule**: any performance regression on the pinned probes stops
   the program until explained — no "we'll fix it forward."

## The phase2 gate (applies to EVERY PR)

- **G1**: full test suites green under the recorded baseline carve-out
  (`bakeoff/round4/MASTER_BASELINE.md` — master lib 465/465 and the
  exactly-12 pre-existing deterministic integration failures; anything
  beyond the ledger is the PR's regression).
- **G2**: headless phase2 runs clean end-to-end (no faults, no wedge,
  honest completion).
- **G3**: sim-makespan and throughput counters do not regress vs the
  pinned baseline (recorded in `perf/phase2-baseline/` by PR 1).
- **G4** (transformation PRs): phase2 **event digest byte-identical**
  (method: the winner's digest + stride-join A/B discipline —
  `bakeoff4/seam-live-gui` `perf/gate-evidence/2026-07-20/README.md`, the
  measured record of exactly this kind of invisibility proof). Authority
  PRs instead declare their re-pin explicitly and re-record the baseline
  in the same PR.
- **G5 — THE FINAL GATE, run by Daniel on every PR**:

      cargo run --release -p desvis -- sim/desmap/tests/fixtures/cencora_phase2.json --run

  (fixture path per repo layout — the tracked tree spells it
  `sim/desmap/tests/fixtures/cencora_phase2.json`; machines carrying
  the untracked `crates/` sync mirror can use either — verified
  byte-identical 2026-07-20). Acceptance: the
  headed run **goes to completion** and **does not regress in
  performance** — neither the simulated day's outcome (makespan,
  throughput) nor the live wall-rate experience. Reference for what
  "good" looks like: the winner's R4.24 gate table (seam `SHAPE.md`,
  "R4.24 — the ≥10:1 gate": 17.7:1 wall rate, 680 MB, faults 0).

## Source-material index (grounded; cite from here per phase)

> Path note: `bakeoff/…` and `perf/gate-evidence/…` paths live on the
> `refactor` branch (dossier PR #235) and the winner branch respectively
> until those merge; `reviews/…` and the spec/plan travel with this
> branch.

- **Design**: `TWO_LANE_SPEC.md` (the two-lane model, v2);
  `reviews/two-lane-claude-review.md` + `reviews/two-lane-sol-review.md`
  (the two adversarial reviews + rebuttal round — every spec contract
  traces to a finding via the spec's §12 register);
  `bakeoff/round4/CONSOLIDATION_LOG.md` (Daniel's design-review rulings
  4a–4e, 17–18).
- **The bakeoff record** (all rounds): `bakeoff/INDEX.md` (reading order);
  round-4 dossier `bakeoff/round4/{SCORECARD,VERDICT,GATE,ERRATA}.md` +
  `EVALUATION.html` (diagrams D1–D5) + `MODEL.html`/`OVERVIEW.html` (the
  explainers); `bakeoff/round4/SCORING.md` (the pre-registered standards,
  §2a VTM-density criterion, §3 readability doctrine); prior rounds
  `bakeoff/{SCORECARD,ERRATA,SYNTHESIS}.md` (round 1: the six-shape
  contest, errata E1–E4, the seven convergent findings),
  `bakeoff/round2/` (the kernel ratification: nine-fault regression
  table, temporal audit T1–T8, errata E5–E7), `bakeoff/round3/` (the real
  port: SCORECARD nine findings, ERRATA E8–E9, EVALUATION §2 the
  canonical instant walkthrough).
- **The winner (design oracle)**: branch `bakeoff4/seam-live-gui`
  @ 5e17612d, worktree `~/vt/greyorange/bakeoff4-seam`; `SHAPE.md`
  R4.1–R4.24 (the complete design ledger incl. the five gate root causes
  R4.19–R4.23); `perf/gate-evidence/2026-07-20/` (committed run logs,
  profiles, determinism pairs).
- **Sibling lineages** (worktrees `~/vt/greyorange/bakeoff4-*`):
  narrative `SHAPE.md` (§2a density both scales; the 99.0 % VTM-tight pin
  `l3_density_legacy_vs_v2_same_physics`); decomp `SHAPE.md` (chain-atomic
  overlay, proximity-binding A/B, handoff-coupled instrument, CoW
  economics); solver `SHAPE.md` + `BRIEF.md` (F4 witness, K-swap optimum,
  the 4+4 liveness pin, phase0-scale latency); strangler `SHAPE.md` (the
  wall inventory, Row V, the two banked mechanical seams).
- **External reviews**: `reviews/sol-findings.md` (Sol's 20 original
  design findings), `reviews/sol-round4-response.md` (its 22-finding
  winner review — the three pre-ratification blockers),
  `reviews/lit-rereview-entitlement.md` (14 literature findings:
  entitlement/ADG, k-robust, TPTS, commitment pricing).
- **Program context**: `specs/vtm-schedule-gaps/HANDOFF.md` (the
  motivating production defect — eager VTM commitment → uncompactable
  gaps); `specs/planner-async/` (the T1 defer-vs-greedy and T2
  poll-order-as-policy tensions this design resolves explicitly).

## Punchlist

### ☑ PR 1 — the spec, the plan, the gate  *(MERGED — #241, master 7cdf3b16)*  *(docs + instruments; ~1 kloc)*

**Why**: the gate must exist before anything can be gated, and the spec must
be reviewable in the tree where the code will land. The pinned baseline is
the ruler every later PR is measured with — without this phase, every
subsequent invisibility or non-regression claim is unverifiable assertion.

- [x] SPEC v2.1 folded (4c payload registries, 4d Demand/Supervisor
      split, 4e this ladder, 4f async_trait/no-aliases, 4g CONCRETE
      kernel — Domain/App trait excluded, T1 retracted with it).
- [x] `README.md` + `TWO_LANE_SPEC.md` + `PLAN.md` + the review pair on
      the branch under `specs/task-model-v2/`.
- [x] G1–G5 invocations documented in this file.
- MOVED → PR 2 head (Daniel's merge-now flow): the digest instrument +
  makespan/counter probe + `perf/phase2-baseline/` pin become PR 2's
  FIRST commits, so PR 2's own digest-identical claim is provable.
- Gate: G1–G3, G5 (G4 trivially — docs only).
- Refs: everything in the source index; specifically the gate-evidence
  README (the measured record this baseline extends).

### ▶ PR 2 — Phase 1: Demand DI  *(IMPLEMENTED — branch tmv2/pr2-demand-di, PR #242; G1–G4 green, awaiting Daniel's G5)*  *(transformation; target < 1.5 kloc)*

**Why**: establishes the DI pattern and the construction boundary on master
at near-zero risk BEFORE any machinery needs it; turns module registration
order (today an invisible policy) into an explicit reviewed property; and
is the cheapest possible detector of construction-time coupling — any
module secretly reaching into planner internals surfaces as a compile
error now instead of a mid-ladder surprise. Construction should hoist as
high as practical — arguably into desvis itself (the single entry point);
Daniel notes the planner-vs-runner split is itself suspect long-term, an
observation parked for a later phase, not this one.

- [x] FIRST: instruments ported as read-only observers on the legacy
      path — `observer/digest.rs` (per-kind counts always; FNV-1a hash
      under `DES_DIGEST=1`), `planner/heartbeat.rs` (`DES_HEARTBEAT`
      stride lines keyed on the planning frontier), and the `[gate]`
      identity block printed by the solver on BOTH exits (the
      planner-error path now joins the observer instead of discarding
      MetricsOutput, so bailing fixtures stay A/B-comparable — the
      strangler's "G2 as executed" lesson). `perf/phase2-baseline/`
      pinned with invocation + identity + stride logs.
- [x] Demand-module CONSTRUCTION out of the planner build — final form
      after Daniel's hard-DI rulings (see the second rider below):
      `flows/ttp/composition.rs::cencora` owns the ENTIRE assemblage
      (world state + demand modules + planner); callers invoke it
      directly and hand the solver a built `Planner`. The solver cannot
      name `ModelState`.
- [x] Trait `DemandModule` → `Demand` (+ impl sites; companion types
      `DemandOutcome`/`DemandRegistration`/`DemandGate` unchanged).
- [x] **Registration order preserved byte-for-byte and documented as
      policy** — the composition's module doc states the
      order-is-policy contract with provenance; preservation proven by
      G4, not by inspection.
- [x] Injected-fake test: `sim/dessim/tests/demand_injection.rs` — a
      test-defined `Demand` as the entire composition; planner polls
      it, consumes its Done, halts clean, zero picks.
- Gate: G1–G5 (digest identical).
- **As-landed record (2026-07-20)**: G1 exactly the 12-failure carve-out
  (names one-for-one vs MASTER_BASELINE; scope note — the recorded
  invocation is `-p dessim -p desmap`; `-p desgraph`'s test target has
  PRE-EXISTING compile breakage from the old float→u64 distance change,
  filed separately). G2 phase2 headless completes the full 10 h day,
  `stop=SimTimeLimit`. G3/G4 pinned + byte-identical across the whole
  transformation: digest `fnv1a=89308db1dda4b043`, 1,169,536 events,
  picks 12,301 / putaways 22,321 / orders 328; wall 668 s ≈ 53.9:1
  solo (see `perf/phase2-baseline/README.md`). Also learned: phase0
  headless on master terminates in the known Signature-A deadlock bail
  at sim≈3679 s (pre-existing, reproduced on the pristine branch head)
  — phase0 is A/B-comparable only via the error-path gate block.
- **Rider (Daniel's rulings, same day, gates re-run)**: (a) the
  quarantine purge — `sim/dessim/{src,tests,src/planner}/quarantine/`
  + stragglers (`quarantine_tests/`, `mod-quarantined.rs`,
  `tote_sources.rs`), ~22.6 k lines of tracked-but-dead files, zero
  recompilation on deletion; (b) the solver tidy — entry points
  collapsed 5 → 2 (`resolve_scenario` pub + one
  `run_solver(data, config, pathfinder, demand, hooks)` with
  `SolverHooks`), dead `run_solver_from_map` removed, both
  `ReservationCtx` structs and the dead observer test files retired,
  stale quarantine-era comments scrubbed. Post-tidy G1 = the same 12
  carve-out names; post-tidy phase2 digest byte-identical vs the pin.
  The solver-vs-planner naming/boundary question stays parked per the
  phase ladder (deferred by Daniel, this section's "why").
- **Rider 2 (Daniel's hard-DI rulings, same evening, gates re-run)**:
  the first DI cut stopped a layer short — the solver still built
  `ModelState` internally, which forced demand in as a factory callback
  (`DemandSeeds` was the symptom: a bag exists only when the caller
  can't hold real dependencies). Rulings, all landed: `ModelState` has
  nothing to do with the solver; no `solver::prepare` staging; the
  composition is named **`cencora`** (there is NO "standard"
  composition) and lives at **`flows/ttp/composition.rs`** (it composes
  the whole ttp system, not just demand); the model build moved INTO
  `cencora` (annotated: the assemblage will be BUILT FROM JSON CONFIG
  per deployment later — nothing may copy it as a default);
  `DemandSeeds`/`DemandComposition` deleted. Callers own the event
  channel, invoke `cencora(&mut data, &config, pathfinder, sink)`
  directly, and hand `run_solver(planner, &data, config, tx, rx, bias,
  hooks)` a finished planner. World bookends moved with the world:
  `InitialPositions`/`ToteInitialStates` emit at the end of
  `build_model`; terminal idle + the status report moved into
  `Planner::run` (the planner owns the world). The injection test is
  now the direct-construction idiom itself (`build_model` +
  `Planner::new` with a test-defined `Demand`). Re-gated: G1 the same
  12 carve-out names; phase2 digest byte-identical vs the pin
  (`89308db1dda4b043`) across the restructure and the module move.
- Refs: CONSOLIDATION_LOG 4d (the Demand/Supervisor nomenclature ruling)
  + 4e (this phase's ruling, with the order-is-policy caveat); round-1
  `bakeoff/ERRATA.md` E1 (policy authority lives at simultaneity — why
  module order is policy) and Sol original finding 4
  (`reviews/sol-findings.md` — registration-order priority inversion);
  `specs/planner-async/` T2 (poll-order-as-policy); the strangler's
  G1/G2 golden discipline (strangler `SHAPE.md`, checkpoint-1 rulings)
  as the transformation-PR method.

### ☐ PR 3 — Phase 2a: CommitTx  *(transformation; rebase spike first)*

**Why**: every later phase needs exactly ONE typed write path to
intercept — shadow recording (PR 6) and authority bridging (Phases 5–9)
hook where writes are centralized. Also retires master's take-then-
rollback compensation machinery by shape. Proven cheap by the strangler;
master's tip is still the strangler's fork commit, so the banked design
re-derives nearly clean.

- [ ] Rebase-feasibility spike (the strangler landed this off master
      f4332606; master has moved — banked DESIGN, not a banked patch;
      size after the spike, split if > 2 kloc).
- [ ] All demand-side writes route through the typed write capability;
      census target per the strangler's record (15 CommitTx methods; the
      take-then-rollback compensation machinery retired by shape).
- Gate: G1–G5 (digest identical).
- Refs: strangler `SHAPE.md` seq-1 record ("CommitTx write seam across
  the entire task/relief surface at linear cost, zero walls") and its
  final brief; round-2 `bakeoff/round2/SCORECARD.md` doctrine-#9
  verification (disjoint PlanCtx/CommitTx/PlaybackCtx — the capability
  pattern this instantiates); spec §6.5 (where this write path is
  eventually bound to PreparedProjection).

### ☐ PR 4 — Phase 2b: plan/apply split  *(transformation)*

**Why**: separates pure planning from mutation — the structural
precondition for v2's validate-then-apply ratification. The seam already
exists in master (`allocator.rs:29`); this exposes it rather than
inventing it, and fixes a read-inside-ratification wart while doing so.

- [ ] `reserve_path` → pure `plan_path` + `PlannedPath::apply` behind
      CommitTx; production callers 4→0 (htm_deliver, htm_pickup,
      htm_store, park_idle_htms per the strangler's census).
- Gate: G1–G5 (digest identical).
- Refs: strangler `SHAPE.md` seq-2 record (the seam was already
  structural at `allocator.rs:29`; the split FIXED a
  read-inside-ratification wart and retired CommitConflict forensics);
  Sol round-4 finding 1 (`reviews/sol-round4-response.md` — the
  partial-projection hole this split is the first step toward closing);
  spec §6.5 (prepare/apply is this split's destination shape).

### ☐ PRs 5a–5e — Phase 3: the kernel lands dormant  *(landings; pre-split)*

**Why**: the semantic walls (Row V) cannot be crossed by transformation —
the machinery must exist before any authority cut. Landing it dormant and
unit-tested decouples "is the kernel right?" from "is the cutover right?"
so the two risks are never taken in the same PR. Split five ways for the
2 kloc rule and reviewable units.

- [ ] 5a: generic time + resources/claims (windows, holds, T6 closes,
      generations, prune+floor) + property tests.
- [ ] 5b: accumulators + policy epochs + `SequencingPolicy`.
- [ ] 5c: settle/ratify engine (PreparedProjection, post-commit phase,
      convergence budgets) + determinism tests.
- [ ] 5d: lane-1 `Task` (needs/plan/on_commit) + `SpawnCtx`/permits +
      the epoch-addressed fact store.
- [ ] 5e: lane-2 `Demand`/`Supervisor` + the sealed adapter +
      `DemandHandle`.
- Nothing wired; each PR is code + its own tests only.
- Gate per PR: G1–G5 (G4 trivially — dormant).
- Refs: the winner's kernel as reference implementation — seam
  `kernel/plan.rs` (SlotClaim/Hold/HoldClose — T6 typed holds, ratified
  in round 2), `kernel/resources.rs` (generations; R4.19 prune+floor with
  its 86/86-stride invisibility proof), `kernel/engine.rs` (settle to
  quiescence, one fire per pass), `kernel/pool.rs` (stable-id batch
  apply — round-2 S7 clock-rule evidence), `kernel/ratify.rs` (the
  closure walk + overlay validation Sol r4 finding 4 characterized),
  decomp `SHAPE.md` validate_chain (the bundle_with prior art) and CoW
  economics (§1.8 rows: 113–249× — the spec §2 cost clause's
  provenance); round-2 `bakeoff/round2/SCORECARD.md` nine-fault
  regression table (the property-test checklist: every row must stay
  dead); the spec's §12 traceability register (which review finding each
  kernel contract discharges — notably Sol blockers 1/2/4/5, Sol 6/7/9/14,
  Claude 1/2/3/7/11).

### ☐ PR 6 — Phase 4: shadow mode  *(landing)*

**Why**: proves the kernel can plan the real day BEFORE it owns anything.
Parity metrics convert cutover risk from argument into measured
divergence, with zero blast radius (shadow reads, never writes) — this is
how the no-two-writable-authorities rule is honored while still learning
from production-shaped load.

- [ ] v2 shadow-plans phase2 alongside legacy (zero authority);
      parity/divergence metrics as CI artifacts; wall overhead bounded.
- [ ] Port remaining instruments as needed (heartbeat, wedge forensics,
      density report).
- Gate: G1–G5 (legacy digest identical; shadow read-only).
- Refs: Sol round-4-spec-review migration ruling
  (`reviews/two-lane-sol-review.md` finding 12 + "Migration ruling" —
  shadow/no-authority first, never two writable authorities); the
  A/B stride-join method (seam R4.19 baselines in
  `perf/gate-evidence/2026-07-20/`); Sol Q22 retained-state probe
  (`reviews/sol-round4-response.md` finding 12) — its RSS/line slope
  lands with these metrics.

### ☐ Phase 5: first authority slice — station presentation windows  *(authority; ~2 PRs)*

**Why**: the full authority-cut mechanics (knob, one-way bridge, re-pin,
conservation audits) get their first rehearsal where a mistake is least
costly — the smallest real resource with both handoffs, nearly decoupled
from the rest of the flow. The subject of this phase is the CUTOVER
PROCESS itself, more than the station.

- [ ] Deliver→present→depart cut to v2 authority behind the planner
      knob; one-way legacy oracle bridge; declared re-pin; conservation
      audits; clean-quiescence enumeration.
- Gate: G1–G3 + declared re-pin + G5 (expect makespan ≈ baseline).
- Refs: seam `tasks/station.rs` (Presentation dwell + the E7
  quote-vs-arrival gate — round-2 `bakeoff/round2/ERRATA.md` E7 and
  round-3 E9.3, the doctrine made structural in spec §7); SCORING §2a
  (station density is the named secondary tier).

### ☐ Phase 6: relay-slot slice  *(authority)*

**Why**: introduces the harder state shapes — party-owned holds, T6
handoffs, time-typed future supply — on a modest resource before the
choke point. The tote-634 double-use class gets its structural fix here,
where it lives.

- [ ] Tote holds + T6 deposit/pickup handoffs; time-typed future supply
      (`CommittedAt` + tote-timeline claim + atomic consumption) lands
      here — this is the tote-634 class's home.
- Gate: as Phase 5.
- Refs: the tote-634 forensic trace (seam `SHAPE.md` R4.21 + round-4
  `GATE.md` §4.3, the commit-time publication race); Sol round-4
  finding 2 + Q16 (the AvailableAt design this implements); spec §6.6;
  the R4.18 return-board partition mask record (seam `SHAPE.md` — the
  relay-partition wedge this slice's audits must keep dead).

### ☐ Phase 7: the VTM slice — the wall  *(authority; shadow-parity PR then cutover PR)*

**Why**: this is the program's purpose — lazy commitment at the system
choke point (the vtm-schedule-gaps defect; legacy structurally cannot do
it per Row V, twice verified). Sequenced late deliberately: only after
the mechanics are rehearsed (Phases 5–6) and shadow has proven parity.
Vertical because the scalar frontier and a lazy timeline cannot share
authority over one interval. The stairstep class is expected to surface
and die here or in Phase 9 — per Daniel's ruling it is handled when the
G5 throughput gate flags it, whichever phase that is.

- [ ] Timeline replaces `available_at`; forward + return handoffs
      together; lazy commitment live; scalar frontier retired for VTMs.
- [ ] §6.9 stamped-read spike lands before or with this slice.
- [ ] §2a density rows recorded before/after — this phase exists to move
      them.
- Gate: G1–G3 + re-pin + G5; **expect improvement** (the prize).
- Refs: `specs/vtm-schedule-gaps/HANDOFF.md` (the motivating defect);
  strangler `SHAPE.md` Row V (the wall verified in master code:
  `vtm/mod.rs` scalar frontier, no reverse promotion, deposit_time
  hard-read ×30, RAII relay claims) and Sol's independent verification
  (`reviews/sol-round4-response.md` finding 13); the density verdict —
  narrative `SHAPE.md` §2a (VTM-tight 99.0 % vs legacy 86.0 %, the pin
  `l3_density_legacy_vs_v2_same_physics`; phase0 inversion legacy 43.7 %)
  + seam `SHAPE.md` R4.15 (31.1 %, attribution 0 % dispatch / 3.5 %
  handoff / 96.5 % feed) + SCORING §2a (the >98 % target and its
  provenance); the stairstep record (round-4 `GATE.md` §4.1 — the two
  strict-< mechanisms Daniel observed live); Sol spec-review finding 12
  (why this cut is a vertical slice, never leg-local).

### ☐ Phase 8: HTM legs + parking  *(authority)*

**Why**: widens authority to the 350-strong fleet only after the choke
point is proven. Parking-as-leg + kernel-held obligations replace the
relief demand module (demand-side pathfinding dies here). FH-SIPP itself
is untouched — the search was never the problem; the commitment model
around it was.

- [ ] Bind/leg claims through the kernel; FH-SIPP unchanged as the pure
      planner; parking as leg-on-store + §6.7 obligations.
- Gate: as Phase 5.
- Refs: the proximity-binding convergence (decomp `SHAPE.md` R4-20 —
  ruled, landed, A/B-won 89→23 turns/completion; narrative converged
  independently; seam adopted with credit in `tasks/htm.rs`); the
  leg-on-Store parking ruling and its measured cost (seam `SHAPE.md`
  R4.11: A2 always-park row +0.6 s); the R4.18 startup-parade record
  (fixture seeds resters on working cells — the audit this phase keeps);
  the parking-deconfliction residual (seam R4.15 tail standoff; the
  CO-WHCA* program `project_deconfliction` — explicitly OUT of this
  phase's scope).

### ☐ Phase 9: demand cutover  *(authority)*

**Why**: demand goes last because the legacy modules keep working over the
bridges until every resource they command is v2 — cutting them earlier
would mean bridging in both directions. Licensing and credits land here
because this is the first moment plural policy has real ends to
arbitrate; the stabilizer gate is what makes production authority
prudent rather than hopeful.

- [ ] Books/daemons → `Demand`/`Supervisor` impls under permits; licensed
      per-station/zone ends; `CreditPool` for the putaway manifest.
- [ ] Scoring stabilizers resolved BEFORE any production book is
      authoritative (spec §6.4).
- Gate: as Phase 5.
- Refs: Sol round-4 findings 8 + Q17 (JointSolve degenerates on the
  single-FLOW wiring — the licensing this phase creates) and finding 3 +
  Q19 (the StageLoop global-manifest error CreditPool kills); decomp
  `SHAPE.md` per-station arenas (the multi-end prior art); solver
  `SHAPE.md` (the F4 witness, the two-book contest, the 4+4 liveness pin
  `two_books_at_four_lines_greedy_stalls_where_joint_solve_completes`,
  K-swap 30 % — the composition that becomes REACHABLE once ends are
  licensed); the round-1 adversary review F3 (master `scoring.rs`
  stabilizers: pressure-squared, relief caps, anti-windup — the
  port-before-production obligation, standing since the DESIGN.md
  review); the round-4 batch-soundness doctrine (round-4 architect
  record: policy surviving batch-parallel proposals must be
  reservation-shaped or static).

### ☐ Phase 10: retirement  *(transformation + cleanup)*

**Why**: minimum-change discipline means the old truth is deleted exactly
once, when nothing reads it. Re-deriving the 12-red baseline ledger
closes the loop — several of those failures should die with their
causes, and the ones that don't get honest new owners.

- [ ] Legacy planner paths, conduit machinery, scalar remnants deleted;
      knob default flips; docs swept; the 12-failure baseline ledger
      re-derived (several entries should die with their causes).
- Gate: G1–G3 + G5; makespan ≥ best prior phase.
- Refs: `bakeoff/round4/MASTER_BASELINE.md` (the 12-red ledger and its
  three families); round-3 `bakeoff/round3/ERRATA.md` E8 (the zero_gap
  parking-precondition defect, chip `task_ca2fb215`); the dead
  parking-cancel machinery lead and Signature-A quiescence-recognition
  taxonomy (strangler `SHAPE.md` master-side discoveries) — all three
  are candidates to fix-or-close during this sweep.

## Standing items (tracked here, scheduled into phases)

- Paper-adversary pass on the v2.1 traits (spec ratification-ladder
  gate 2) — before PR 3 (PRs 1–2 don't touch the new traits).
- 10× retained-state probe (Sol r4 finding 12 / Q22) — with Phase 4.
- Windowed-pathfinding charter (spec Appendix A; constraints from
  `reviews/lit-rereview-entitlement.md` F2.1–F2.5) — opens only after
  Phase 7 proves the timeline.

## Status

PR 1 MERGED (#241 → master 7cdf3b16, 2026-07-20). PR 2 (#242,
`tmv2/pr2-demand-di`) IMPLEMENTED same day: instruments + phase2
baseline pin, demand composition injected at the solver entry points,
`Demand` rename, proof test. G1–G4 green — G4 byte-identical across the
whole transformation (digest `89308db1dda4b043`). Awaiting Daniel's G5
headed run, then merge. Paper-adversary pass on the v2.1 traits due
before PR 3.
