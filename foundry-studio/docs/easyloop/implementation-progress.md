# Cencora / Walmart task implementation

Implementation started from `4509f5ea7` after Daniel approved proceeding on
2026-09-07. The map stays at the reviewed geometry checkpoint. Format migration
and the broader worldlens migration remain parked.

## Work sequence

1. Establish the Cencora baseline and preserve existing failures.
2. Register tasks on their structs using `#[task]`, with conventional `new`
   constructors and run-lived defaults for later children. Remove the central
   task catalog and handwritten root specs.
3. Move population work into finite setup roots with explicit completion
   dependencies. Replace `ModelState` with individual shared service handles.
4. Implement reusable tote supply, presentation, item transfer, clearing,
   recycling, collection and dispatch tasks for Walmart.
5. Verify real custody, paired picks, conveyor capacity and retained inventory
   HTMs; honor curved links in motion and collision timing. Conveyor tote
   animation is secondary.

## Checkpoints

The initial baseline passes six composition tests (including the real 20-pick
run) and six Cencora map checks. `world_spec_run` has two passing tests and one
existing observer failure: dwell `[19.509s, 37.509s)` escapes Processing
`[20.841s, 20.841s)` for robot 8. That failure occurred before this implementation.
The older putaway probe also fails on both prior checkpoints; neither constitutes
putaway parity. Failures must be fixed in task/plan timing, not hidden in observers.

The registration layer now uses `linkme` static descriptors. `#[task]` infers
its JSON name from the struct and adapts its async `new(&TaskContext)` constructor.
No task or manager is constructed by registration. Typed child results survive
the adapter. `TaskDefaults` merges fields with instance overrides and remains
available to children spawned later. Constructors receive a context and retain
individual manager handles. Checkpoint `048b51d32` established this layer. The next chunk removes the
central root enum/factory and the `TaskSpec` interface; all nine production roots
now register beside their structs and construct through `new`. Composition JSON
accepts `task_defaults`, including field-level instance overrides. Cencora real
pick events match the equivalent legacy settings with either root config or
composition defaults.

Validation: two real-scheduler registration tests, four manager resolver tests,
and six composition tests pass. Invalid task names, unknown parameters and wrong
parameter types are refused; late children share one lazily constructed manager
and honor both composition defaults and explicit overrides.


Checkpoint `1723a9155` removes root specs and the central task catalog.
The next chunk adds explicit JSON `after` dependencies and finite `FillInventory`
and `LoadOrders` roots to both presets. Initial stock, orders, arrival queues and
striping announcements are now task work; order/arrival managers construct empty
state. Typed setup results remain available while several consumers independently
wait for its completion. Missing dependencies, cycles and perpetual prerequisites
are refused. Cencora physical pick events remain unchanged when roots are reversed
and when equivalent parameters are supplied through defaults.

Validation for the setup chunk: 22 checks pass (eight composition, six Cencora
map, three registration, four resolver, one injected-composition check). A setup
probe confirms assembly leaves inventory empty and the root populates the exact
declared tote contents. The probe uses the same feasible one-SKU fixture as the
existing 20-pick checks; the original ten-SKU fixture cannot fulfill its orders.


Checkpoint `16cbdcbd3` contains the finite setup roots and completion dependencies.
The next service-extraction chunk removes ModelState from bot-plan mutation,
VtmForward, VtmReturn, Presentation and the running PutawayInduction task. VTM
endpoint cell lookups belong to VtmService; event output and cycle identifiers
belong to EventJournal. Parents pass the specific handles to these children.
ModelState remains in the HTM transport tasks, several demand tasks and runner
reporting, and its legacy construction still needs to become manager factories.

Validation: three bot-plan unit tests and 21 VTM-service unit tests pass. Eight
composition tests, three VTM timing fixture tests and the injected-composition
check pass after extraction. The old oversized bot-plan test setup was removed;
its three actual tests remain and use only inventory and optional parking.


Checkpoint `d20161766` contains the service extraction above. The next behavior
chunk fixes the inherited induction/observer failures: a fill starts no earlier
than HTM readiness, the preceding fill's end and the arrival's net time, then takes
the full configured dwell. Arrival itself is waiting for the operator; actual
Presentation/PutawayInduction work opens Processing. A StationWork plan leg
advances robot readiness through the operation while retaining the cell/tote.
Empty-tote supply stops after its arrival list drains (ReturnTotesToRack already
assumed this); both presets make supply wait for LoadOrders.

The 12-induction parameter regression now completes naturally under both equivalent
configurations; it is committed as `tests/putaway_tasks.rs`. All three world_spec_run
tests pass, including the original observer failure. Eight composition checks and
97 unchanged observer unit tests pass. The five zero_gap tests fail on the existing
picking fixture. A detached d20161766 worktree reproduces ghost_qf1's five-pick,
139251ms deadlock, so that representative failure predates this timing fix; the
other four were not separately replayed on that checkpoint. The branch still lacks
master's 87ba106fd Cencora VTM isolation fixture change (teleport to off); that is
unrelated to this minimal picking fixture and has not been silently imported.


Checkpoint `b938c9df4` contains the station-work timing and draining-supply fixes.
The next chunk extracts Navigation as a shared routing service owning geometry,
path estimates, the motion model and reservation planning. ModelState temporarily
holds its Arc while the HTM transport tasks are being converted to receive the
specific services directly. This checkpoint changes ownership, not map geometry
or task behavior.

Validation: cargo check passes; all eight composition checks, the putaway
regression and all three world-spec run tests pass. The remaining ModelState
removal and correct Walmart task tree are still outstanding.


Checkpoint `9823f9bb4` contains Navigation extraction and was pushed to
`origin/agent/walmart-map-corrections` at Daniel's request.
The next service chunk removes ModelState from HTM pickup, delivery and storage,
PickDemand/PickItem, putaway staging/supply, ReturnTotesToRack/ReturnTote, and
pressure-relief parking. Parents pass the specific service handles to children;
StationService is shared by Arc. Construction temporarily still reads the old
equipment assembly, which is the next boundary to remove.

Validation: eight composition, one putaway, three world-spec and one injected-run
checks pass. Parking extraction also passes seven parking and seventeen relay
deposit unit tests. These are ownership changes; the reviewed geometry and
existing Cencora task semantics stay at their prior checkpoint.

## 2026-09-07 — synchronous construction and physical fulfillment integration

State managers now instantiate synchronously, once per type per run. Tasks can
request new managers later. The resolver no longer has resource scopes or waits
for configuration. Root task factories are synchronous `Task::create`; `#[task]`
registers only the JSON root descriptor. Children use ordinary constructors and
can resolve task defaults without registration.

Production `ModelState` is deleted. Initial fleet, inventory and orders are
finite setup tasks. Reporting reads existing managers. A relay index first
constructed after inventory changes initializes from authoritative tote history.
Cencora remains the default named JSON composition; Walmart selects its own
`FulfillOrders` root.

The first Walmart flow retains order totes on HTMs, moves inventory totes onto
a shared conveyor, routes delivering HTMs through the one-way holding lane,
pairs both actual totes for each pick, changes contents at pick end, returns
inventory through VTM/relay storage, and extracts completed orders. Curved
navigation preserves heading through presentation dwell. Revocable collection
claims prevent two retained HTMs selecting the same return.

This is an integration checkpoint, not end-to-end certification. Component
checks pass for synchronous construction, late service access, Cencora task
configuration, transfer custody/timing, collection capacity and curved motion.
The full-map Ghost smoke reached all 10 picks and all five orders/extractions;
its final physical audit still needs correction/investigation. The first
minimal run hit its old 30-minute simulation limit after 13 picks; the dedicated
physical-flow fixture now allows four hours and still requires natural
completion. Collision runs and the larger full-map workload remain acceptance
work. A swallowed task panic was identified during the audit and is being fixed
so failures terminate clearly instead of appearing as timeouts.

Current machine scheduling admits one order at a time per active machine; this
exercises repeated presentation and side changes, but does not establish
saturated eight-tote throughput. The conveyor capacity service is independently
checked at its eight-tote limit. See `workload-checks.md` for the durable fixtures
and run commands.

### September 7: complete serial collision checkpoint

`399fb09e1` saves the first full physical Walmart run and is pushed to
`agent/walmart-map-corrections`. Both Ghost and CollisionDes completed the full
500-order / 2,000-pick generated workload with all 500 extractions and inventory
returns. The headed CollisionDes run also completed naturally (104.8 seconds
wall time, 84,231.595 simulated seconds). This checkpoint still admitted one
order and one conveyor tote per machine at a time; subsequent work removes that
restriction. The full fixture now explicitly selects collision kinematics and a
48-hour horizon, avoiding its inherited four-hour truncation.

The live view shows the latest calculated slice and can visibly skip a short
presentation/U-turn. Minimal and full-map audits verified the recorded playback
retains the order HTM at its physical presentation during each pick.

Daniel clarified the Walmart VTM is floor-running, unlike Cencora's attached
library-ladder VTM, and its make/model is unspecified (not DA50). The world
already declares floor locomotion, two inventory VTMs per aisle and one buffer
gripper per aisle: 48 total. Runtime movement still uses the shared VTM model;
separate floor-VTM behavior remains required later. The serial run's VTM busy
fraction averaged 2.6% (maximum 12%); fleet count was left unchanged pending
measurement with concurrent station work. Existing DA50 report labels must not
be interpreted as Walmart equipment identification.

### September 7: concurrent station flow

Order workers now own distinct waiting berths. Inventory queue travel and
collection/return run as separate children alongside picking, and the conveyor
selects the earliest available presentation with a deterministic side preference.
Completed orders leave for dispatch without returning unnecessarily to waiting.
The full CollisionDes workload completes all 500 orders and 2,000 picks in
17,029.804 simulated seconds (4h 43m 49.804s), about 4.95 times faster than the
serial checkpoint. Every machine reached eight conveyor totes and multiple
simultaneous order HTMs. The second headed run also completed all work in
111.4 wall seconds and remains open for inspection.

The minimal collision case exposed a shared relay-selection bug during final
cleanup: temporal prescreening discarded open-held relay accesses before their
blockers could reach parking relief. Classifying those blockers first restores
normal parking relief. The regression fails without the fix; the concurrent
minimal map now completes 24 picks, eight orders and all 64 VTM cycles in
768.881 simulated seconds, with six order HTMs and six conveyor totes observed.
No capacity increase or concurrency reduction was used.

In the concurrent headed full run, VTM utilization averaged 11.3%, with the
busiest at 47.9%. Counts remain unchanged; the current measurements still do not
identify VTM fleet capacity as the bottleneck. Full floor-VTM behavior remains
separate work.

### September 7: limited visual correction and whole-flow audit

The renderer preserved north-up positions but still negated Y when inferring
heading. Removing that sign inversion aligns HTM yaw with rendered travel.
Carried totes now use the same HTM yaw in their instance data and shader, so
they rotate together in live view and playback. Six heading checks, three
renderer-instance checks and two shader checks pass. Other motion/playback
fidelity work is deferred while throughput and missing flows take priority.
Daniel confirmed that both HTM rotation and tote rotation look correct in the
updated headed run, which completed all 500 orders with CollisionDes enabled.

The audit distinguishes the generated tote-only picking workload from the
whole operation: Walmart inbound roots and carton acquisition are absent;
buffer handling is separately tested, and extraction currently empties reusable
totes. The physical input, buffer release and container extraction questions
are recorded in `remaining-flows.md` and awaiting Daniel's answers.

### September 7: shared order dispatch and corrected circuit duration

Waiting-berth workers now claim from a shared finite order set at their physical
availability time. Claims remain provisional until the scheduler accepts them;
competing claims are invalidated, and future arrivals are respected. The full
500-order CollisionDes workload improved from 17,029.804 to 14,437.316 simulated
seconds. A separate correction restores the prior full-circuit timing contract,
splitting the configured 45 seconds across inbound and outbound travel instead
of charging it twice. The same full workload then completes in 13,871.701 seconds,
with all picks, extractions and returns: 519.04 picks/hour, 22.77% above the
concurrent fixed-batch baseline. This does not establish the physical belt speed.

Daniel deferred putaway until the outbound flows are handled. The selected
putaway sequence is empty-tote retrieval, filling, then rack storage; quantity
and timing remain open. He approved simplifying separate downstream bagging/
packing to emptying the order tote at extraction and returning it to order-tote
storage. Buffer routing/release and the carton/tray lifecycle remain questions.

### September 7: physical queueing and extraction selection

Mixed-container picking, box induction, physical filled-buffer round trips and
distinct carton/reusable extraction outcomes now pass both full-map acceptance
cohorts. The current provisional routing and site questions are recorded in
`questions.html`; inbound putaway remains deferred.

The FIFO inventory HTM queue now collects the next ready inventory tote rather
than requiring each HTM to recover the one it delivered. The full retained queue,
eight-tote capacity, conveyor timing and fork dwells remain intact. A typed
`PresentTote` child waits for physical inventory readiness before claiming a
presentation; it cannot reserve ahead of earlier-ready work.

The previous direct dispatch from the final presentation could hold that cell
for thousands of seconds while downstream work blocked. Completed orders now
return to their owned waiting berths before dispatch. A new order can use the
berth only after the prior HTM's exact physical reservation clears. Return and
dispatch children still run independently. Storage waits observe reservation-only
departures and refresh departure time after waking.

Those local corrections reduced presentation blocking but did not alone improve
the source batch. Per-order custody milestones exposed a large tail between
buffer HTM pickup and extraction. Generic `Navigate` now optionally chooses
among declared destinations when the HTM is ready, reselecting after reservation
changes if blocked. `DispatchOrder` uses the actual selected port for extraction
and empty-container return. Normal collision planning remains responsible for
the entire route; no capacity, fleet or service-time shortcut was introduced.

With these changes the 500-order source sample completes in 5,378.043 reported
seconds versus 7,744.366 at `4c73c945f`; the 500-order / 2,000-transfer generated
mixed cohort completes in 9,441.955 versus 12,217.351. Completed orders/hour
increase by 44.00% and 29.39%, respectively. All physical audits pass, including
246 source cartons, 316 filled buffer round trips and 2,292 source VTM cycles;
generated work preserves 167 cartons and 5,002 cycles. Targeted Cencora putaway
and minimal/full-map compatibility checks also pass.

These are finite-batch measurements under provisional site rules, not sustainable
site capacity estimates. Independent calendar/custody accounting verifies full
and clipped-window occupancy. `throughput.md` records sample-path Little's law,
resource demand bounds, denominator limitations, failed admission experiments,
the moved bottlenecks and the remaining outbound tail. Order approach currently
starts after inventory readiness; safe overlap is a future scheduling experiment.
