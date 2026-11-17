# Task-model v2 — what this is and where it came from

This directory is the culmination of a month-long design program: a
specification (`TWO_LANE_SPEC.md`) and an execution plan (`PLAN.md`) for
replacing the simulator's eager offline planner with a lazy,
demand-driven task/resource kernel — evolved onto `master` one small,
gated PR at a time. Tracking issue: **#240**. First PR: **#241** (this
branch).

## The problem

The production-shaped defect that motivates everything:
`specs/vtm-schedule-gaps/HANDOFF.md` — the main-branch planner cannot be
lazy about VTM commitment, so VTM schedules develop uncompactable gaps,
and VTM capacity is the choke point of the whole system. Daniel's
2026-07-01 direction set the shape: schedule/execute split, parent/child
tasks, laziness implemented by the runtime, potentially reusable as the
production runtime.

## The bakeoff (four rounds, 2026-07-16 → 07-20)

Rather than argue about task-model shapes, we ran them. Full record with
reading order: `bakeoff/INDEX.md` on the `refactor` branch (dossier
PR **#235**); every claim there was judge-reproduced.

- **Round 1** — six competing toy shapes (async bodies, actor arms,
  turns-as-data, spawn chains, effect letterboxes, hybrid). Verdicts:
  data-shaped kernel interfaces dominate the encoding question; seven
  convergent findings (transition-triggered waits, joint-feasibility
  replans, armless-park guard, …) were decided by independent multiple
  discovery and bound every later round.
- **Round 2** — the kernel ratified: one shared kernel (tentative plans,
  accumulators, atomic ratification, generic time T1–T8, typed holds
  T6, a real worker pool), all nine round-1 faults regression-killed.
  Also convicted: the multi-method callback protocol
  (`plan`/`on_commit`/`take_wait`/`done`) — a conviction the current
  spec §3 must and does distinguish itself from.
- **Round 3** — the real port, three lenses (narrative / decomp / seam)
  on the actual production services. The async authoring surface won the
  readability contest (nobody hand-wrote a stage machine); the "+30 %
  faster" headline was later retired as a physics artifact (E10 —
  same-physics is parity-class).
- **Round 4** — full send: ALL task + demand modules, five lineages, an
  architect loop, pre-registered scoring (`bakeoff/round4/SCORING.md`).
  The seam lineage won and then carried the REAL Cencora day
  (`cencora_phase2.json`: 66,276 units, 388 robots) live at 17.7:1 wall
  rate, 680 MB, bit-identical double-runs, zero faults
  (`bakeoff/round4/GATE.md`). The density verdict (three independent
  instruments): laziness clears the >98 % VTM-duty bar exactly where the
  VTM binds (99.0 % vs legacy 86.0 %) and the phase0 ceiling is
  allocation/feed, not the task model — the solver lineage's
  jurisdiction. The strangler control arm proved master structurally
  cannot grow lazy VTM commitment in place (Row V: the scalar
  `available_at` frontier).

## The branches (all pushed; may never merge — they are evidence)

| Branch | What it is |
|---|---|
| `bakeoff/{async,actor,multisched,spawn,effects,hybrid}` | round-1 toy shapes |
| `bakeoff2/turns`, `bakeoff2/prose` | round-2 kernel ratification pair |
| `bakeoff3/{narrative,decomp,seam}` | round-3 real ports |
| `bakeoff4/{narrative,decomp,seam,solver,strangler}` | round-4 full-send lineages (each carries `SHAPE.md`, its design ledger) |
| `bakeoff4/seam-live-gui` @ 5e17612d | **the winner / design oracle**: real-data phase2 live, R4.19–R4.24 kernel-hygiene work, committed evidence in `perf/gate-evidence/2026-07-20/` |
| `refactor` (PR #235) | the complete dossier: scorecards, verdicts, errata E1–E12, diagram-led evaluations, `MODEL.html`/`OVERVIEW.html` explainers |
| `tmv2/pr1-spec` (PR #241) | this branch — the program's first PR |

Local worktrees (Daniel's machine): `~/vt/greyorange/bakeoff4-*`,
`~/vt/greyorange/tmv2-pr1`.

## External review

Sol (Codex) reviewed the design twice: 20 findings against the original
DESIGN.md (several — F4 registration-order inversion, F7 read-set, F19
clock rule — became named fixtures of the program) and 22 findings
against the winner (verdict: ratify-with-blockers). Both live in
`reviews/` alongside the two adversarial reviews of the spec itself.

## The refinement conversation (Daniel, 2026-07-20) — high points

The winner's surface was interrogated live; each probe produced a ruling
now baked into the spec:

1. **"The kernel is the executor"** — no tokio, noop wakers,
   readiness-as-data; polling policy IS scheduling policy. Laziness must
   be runtime-enforced, "not a pinky-swear in the task's impl" — the
   original `fn plan`/`fn schedule` split's intent, restored as
   `needs`/`plan`/`on_commit` with the kernel driving.
2. **Naming honesty** — the `Schedule` alias confused more than it
   carried ("std::task::Poll has only ONE arm with a payload"); ruling:
   `#[async_trait]` wherever trait methods return futures, and no alias
   types that just confuse things.
3. **Templates convicted** — "horrific… this has to work with dependency
   injection": the closed `WhTemplate` enum died in favor of direct
   construction with kernel-minted hierarchical ids; the same DI probe
   then convicted the four remaining closed payload enums (the
   module-registry design in the consolidation log, folding into v2.1).
4. **The governing principle** — *"stuff you have to just remember to do
   is not a good abstraction contract."* Every convention became a type,
   a kernel rule, or a named §10 entry; seven of the winner's
   remember-to-dos are now structural.
5. **Demand nomenclature preserved** — `DemandModule` → `Demand` (lane 2
   splits into `Demand`/`Supervisor`; `fire` is a Demand-only power;
   `is_daemon` dissolves into the type).
6. **The delivery pivot** — the winner becomes the design ORACLE, not the
   merge trunk: master evolves by serial, <2 kloc, fully-gated PRs, with
   Daniel's headed release run of `cencora_phase2.json` as the final
   gate on every one (PLAN.md G5).
7. **The review protocol** — two independent adversarial reviews of the
   spec (blind passes, mutual cross-examinations, a rebuttal round),
   converging on the two-lane factorization with all blockers repaired
   in v2. The reviews are kept in-repo as deliberate provenance.

## Reading order

1. `TWO_LANE_SPEC.md` — the design (status header carries the
   three-level ratification ladder).
2. `PLAN.md` — the phases, each with its why, its references, and the
   gate.
3. `reviews/two-lane-claude-review.md` + `reviews/two-lane-sol-review.md`
   — how the spec got its shape.
4. The dossier on PR #235 — start at `bakeoff/round4/MODEL.html` (the
   abstract model) and `bakeoff/round4/OVERVIEW.html` (the winner from
   10,000 feet), then `EVALUATION.html`/`SCORECARD.md`/`GATE.md`.
