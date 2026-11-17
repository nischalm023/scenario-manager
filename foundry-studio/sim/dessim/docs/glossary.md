# Glossary

Domain vocabulary for the simulator: robots, locations, the movement/id
vocabulary, the metrics on the output sheets, and the run levers. Terms are
alphabetical. Where a term has three names (a code type, a customer-visible sheet
column, and a debug-print prefix), all three are given.

The one relationship to internalize: a **Tote** is reused by many **Trips**; a
Trip contains one or two **Cycles** and a set of **Legs**. Trip is the
correlation key, Tote is not.

---

**Activity** - The kind of journey-leg row: Travel (moving), Wait (stationary, holding for something), or Mechanical (a fork/load/unload action).

**Cargo / Carrying Tote** - Whether a robot physically has a tote aboard for a given journey leg. Distinguishes loaded travel from empty repositioning.

**Cycle (VTM cycle) / Cycle ID** - One VTM movement within a Trip: a Forward cycle fetches a tote from a rack to a relay, a Return cycle stores it back. Code type `VtmCycleId` (prefix `vcyc`), sheet column *Cycle ID*; unique per cycle, so overlapping cycles under Teleport stay separable. A Trip has one or two. Bigger than a Leg, smaller than a Trip.

**Cycle Type** - Label on VTM rows: PickSupport or PutawaySupport. A relabel of Direction (Forward = PickSupport, Return = PutawaySupport).

**D30** - The HTM robot model; "D30 Idle %" in the summary is HTM idle time. See HTM.

**DA50** - The VTM robot model; "DA50 Idle %" in the summary is VTM idle time. See VTM.

**DC (Distribution Center)** - The forward distribution center for a bulk pick. Code `DistributionCenterId` (a string), sheet column *DC*. Blank for pre-formed-order picks and for putaways.

**Direction** - Label on VTM cycles: Forward (outbound fetch to a relay) or Return (store the tote back to a rack).

**Dwell** - The operator's processing time at a station during a presentation (`dwell_ms`). The station is busy for this window; accounted separately from travel and wait.

**Embargo (pick embargo)** - The earliest sim time at which picking may begin (`pick.embargo_secs`). Putaway runs during the embargo so the storage buffer fills before pick stations start; a 2h embargo on an 8h run yields 6h of picks.

**Empty / Loaded / Fork / Buffer (ms)** - The four time components a VTM cycle is broken into on the VTM Cycles sheet: Empty positioning travel, Loaded (tote-carrying) travel, rack-side Fork time, and relay-side Buffer time. They sum to the cycle total.

**Fidelity / Ghost mode** - The movement model (`--fidelity`: ghost, collision-des, tick-stepper). In ghost mode robots phase through one another with no collision or traffic, running on pure kinematics to establish a lower-bound travel time; collision-des adds reservation-aware avoidance.

**Flow Score** - Fleet-wide metric: the fraction of inter-presentation gaps at a station that are at or below the pick time. High means stations are rarely starved between presentations.

**HTM (Horizontal Transfer Module)** - The floor robot that carries a tote horizontally between a relay and a station and back (the relay-station-relay cycle). Model D30. Pairs with the aisle-bound VTM at the relay.

**Leg (journey leg)** - One segment of a robot's journey, emitted as a journal row (a Travel, Wait, or Mechanical span). Legs group under a Trip ID and are finer than a Cycle. A dedicated "LegId" was considered and dropped as redundant with existing ids.

**Motion Model** - How robot travel time is computed: kinematic (trapezoidal accel/cruise/decel) or a flat constant-speed approximation. Reported in the run settings.

**Nav factor** - Config multiplier on A* path distance in ghost mode (`nav_factor`, default 1.8), modelling congestion inflation. In analysis the same term is used for the ratio of collision-aware travel time to ghost travel time, quantifying a layout's traffic penalty.

**Order / Order ID** - A customer order, satisfied by many picks. Code `OrderId` (prefix `order`), sheet column *Order ID*.

**PF Fail Rate (pathfinder failure rate)** - Summary metric: pathfinder failures divided by pathfinder queries. Non-zero means the router could not find a path for some requests.

**Pick** - Removing a unit from a tote at a station to fulfil an order. One pick corresponds to one Trip.

**Presentation (presentation cycle)** - The act of presenting a tote at a station for a single pick or putaway; the unit a Trip corresponds to. The id minted per presentation is the Trip ID, which is why its code type is `PresentationId`. The word is overloaded in code (the event, `PresentationOp`, `presentation_time`).

**Presentations per station per hour (Pres/Hr, Avg Pres/Stn/Hr)** - Station throughput: how many totes are presented at a station each hour, per station and fleet-averaged.

**Putaway** - Storing a unit into a rack, the reverse of a pick. Runs during the pick embargo to fill the buffer.

**Queue factor** - Station-pipeline tuning (`queue_factor`) controlling when the next HTM arrives relative to the current pick finishing: 1.0 = zero gap, >1.0 = arrives early (buffer absorbs variance), <1.0 = arrives late (deliberate gap).

**Rack** - A storage rack holding totes. Code `RackId` (prefix `rack`), sheet column *Rack*.

**Rack fill % (Peak / Final / Min)** - Rack-occupancy metrics on the Rack sheet: the share of rack slots filled, tracked over the run (peak, final, minimum) with the inventory delta.

**Recording** - A finished simulation run written as one `.worldview` file: the documents it was built from, one keyframe every thirty slices, one delta per slice, and the run's totals at the end. What desvis replays, `compare` reads two of, and the xlsx export is built from.

**Relay** - The handoff position where a VTM deposits a tote for an HTM to collect (and the reverse on the return leg). Code `RelayId`, sheet column *Relay*. Sits within a Zone.

**Robot / Robot ID** - One VTM, HTM, or RTP, persistent across the run. Code `RobotId` (prefix `r`), sheet column *Robot ID*.

**RTP** - A robot type (alongside HTM and VTM) used in the non-relay rack/robot-to-person system archetype; its fleet count appears as *RTPs* in the summary. The sim's primary flow is the VTM+HTM relay system.

**SimEvent (event stream)** - The ordered stream of events the solver emits (CycleComplete, TripStart, and so on); the contract between the solver and every observer, metric, and exporter.

**SKU** - A stock-keeping unit. Code `SkuId` (prefix `sku`), sheet columns *SKU* / *SKU ID*.

**Station** - A workstation where an operator picks from or puts into a tote, typed Pick or Putaway. Code `StationId`, sheet column *Station*.

**Station gap** - Inter-presentation idle time at a station: the simtime between one presentation departing and the next arriving. Long gaps mean the station is starved.

**Teleport (VTM isolation mode)** - A diagnostic lever (`vtm.isolation`: off, ideal_rate, teleport) that swaps the VTM cycle-timing model. Teleport grants infinite VTM capacity (cycle ≈ 0, capacity frontier ignored) to answer "what if VTM transport were free?"; it deliberately breaks physical time budgets, so it is not valid for capacity-ceiling claims. Off is production (full geometric cycle); IdealRate is an M/D/c server at real fleet capacity.

**Throughput** - Completed work per hour: picks/hr (*Throughput*) and putaways/hr (*Putaway Throughput*) in the summary.

**Time Series** - The per-time-slice sheet: metrics sampled at a fixed slice width across the run, for plotting how throughput, idle, and fill evolve over time.

**Tote** - The physical bin carried through the warehouse. Code `ToteId` (prefix `tote`), sheet column *Tote*. Recirculates across many Trips in a run, so it is never a per-pick correlation key.

**Trip / Trip ID** - One tote's round trip to serve a single pick or putaway; the top-level correlation key tying together the VTM cycles and HTM legs of that service. Code type `PresentationId` (prefix `pres`), sheet column *Trip ID*. Contains one or two Cycles and exactly one HTM robot, and legitimately appears on two VTM robots (the Forward and Return cycles).

**Trip Type** - Label on HTM legs: Pick or Putaway, derived from whichever endpoint is a station. The task type, not the Forward/Return phase.

**VTM (Vertical Transfer Module)** - The rail-mounted robot that moves along the aisle to fetch a tote from a rack to a relay (Forward) and store it back (Return). Model DA50. Operates only in the aisle; pairs with the HTM at the relay.

**Wait Reason** - Why a Wait/idle row exists. HTM: *No task available* (idle between cycles) or *Awaiting deposit* (dwelling at a relay for the VTM to drop the tote). VTM pre-cycle gaps: *Awaiting demand*, *Awaiting slot*, *Awaiting tote*, *Awaiting HTM deposit*, or *Continuous* (no gap).

**Zone (aisle)** - An aisle grouping of relays and racks. Code `ZoneId`, sheet columns *Zone* / *Relay Zone*.
