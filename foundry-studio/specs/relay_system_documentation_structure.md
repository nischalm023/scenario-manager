# Relay System - Company Documentation Structure

**Purpose:** Template and structure for the company-wide Relay System Google Doc (single source of truth)

**Date:** 2025-11-21

**Audience:** Engineering, Product/Program Management, Executives

---

## Design Principles

1. **Single doc with clear sections** - One Google Doc with 6 tabs for easy navigation
2. **Current state focus** - Minimal history, living reference for current design
3. **Consistent algorithm templates** - Every algorithm follows same structure
4. **Control flow as foundation** - System architecture (Tab 1) is the base that everything references
5. **Separation of design and data** - Algorithm specs separate from simulation results
6. **Lightweight weekly updates** - Bullet points, not comprehensive reports

---

## Document Structure (6 Tabs)

```
📄 Relay System - Single Source of Truth

├─ Tab 1: OVERVIEW & ARCHITECTURE ⭐ (Start here - Exec/PM/Eng)
│  ├─ Executive Summary (1 page: what, why, status, key metrics)
│  ├─ System Architecture
│  │  ├─ Control Flow (VTM → Relay → HTM → Station → Return)
│  │  ├─ Component Overview (Robots, Relay Positions, Zones, Totes)
│  │  ├─ Key Design Tenets (from your doc: operators don't wait, navigation-first, etc.)
│  │  └─ System Diagram/Visual
│  └─ Quick Links (jump to algorithms, program updates, simulation baselines)
│
├─ Tab 2: ALGORITHMS & DESIGN ⭐ (Deep technical - Eng heavy)
│  ├─ Order Selection (nested sub-tab)
│  │  ├─ Problem & Current Approach (TPQ-based)
│  │  ├─ Key Design Decisions (why TPQ over PPF)
│  │  ├─ Algorithm Insights (queuing theory, formulations)
│  │  ├─ Alternatives Considered (utilization, cycle time minimization)
│  │  └─ Simulation Baselines (performance data, comparison to PPF)
│  ├─ Task Scheduling (nested sub-tab)
│  │  └─ [Same template as above]
│  ├─ Path Planning (nested sub-tab)
│  │  └─ [Same template]
│  ├─ Put Algorithm (nested sub-tab)
│  │  └─ [Same template: Greedy vs Min Weighted Cycle Distance]
│  ├─ Set Cover Order Fulfillment (nested sub-tab)
│  │  └─ [Same template]
│  └─ Zone Capacity & Backpressure (nested sub-tab)
│     └─ [Same template]
│
├─ Tab 3: OBSERVABILITY & TESTING ⭐ (How we know it works - Eng/PM)
│  ├─ Key Metrics & Monitoring
│  │  ├─ Throughput (picks/hr/station)
│  │  ├─ Utilization (VTM, HTM, Operator)
│  │  ├─ TPQ tracking (target vs actual)
│  │  ├─ Zone capacity (congestion indicators)
│  │  └─ Dashboard links/screenshots
│  ├─ Testing Strategy
│  │  ├─ Unit test coverage
│  │  ├─ Integration test scenarios
│  │  ├─ Performance test baselines
│  │  └─ Validation criteria
│  └─ Known Issues & Limitations
│     └─ Current constraints, edge cases, future work
│
├─ Tab 4: PROGRAM & UPDATES ⭐ (Weekly tracking - PM/Exec)
│  ├─ Current Sprint/Iteration (what's in flight)
│  ├─ Weekly Updates (reverse chronological)
│  │  └─ Format: Week of [Date]
│  │     ├─ What shipped
│  │     ├─ Key findings/insights
│  │     ├─ Blockers/risks
│  │     └─ Next week priorities
│  ├─ Roadmap (planned features/improvements)
│  └─ Team & Resources
│
├─ Tab 5: SIMULATION RUNS & BASELINES ⭐ (Data - All audiences)
│  ├─ Baseline Runs (standard configs, performance benchmarks)
│  ├─ Micro-Simulations (targeted experiments)
│  ├─ Algorithm Comparisons (Greedy vs MinWeighted, PPF vs TPQ)
│  ├─ Scenario Library (test configs, edge cases)
│  └─ How to Run Simulations (guide for new engineers)
│
└─ Tab 6: APPENDIX
   ├─ Configuration Reference (all parameters documented)
   ├─ Glossary (VTM, HTM, TPQ, PPF, relay position, etc.)
   ├─ Academic References (papers, queuing theory)
   └─ Links to Code (GitHub, design docs in repo)
```

---

## Template 1: Algorithm Section (for Tab 2 nested sub-tabs)

Use this for each algorithm under Tab 2:

```markdown
# [Algorithm Name] (e.g., TPQ-Based Order Selection)

## Problem & Current Approach

**What problem does this solve?**
- [1-2 sentences on the problem]

**Current approach:**
- [Algorithm name + 1 sentence description]
- Status: [In production / In development / Planned]

**Key metrics we optimize for:**
- [Metric 1: throughput, utilization, etc.]
- [Metric 2]

---

## Key Design Decisions

**Decision 1: [e.g., Why TPQ over PPF?]**
- **Rationale:** [Why we chose this]
- **Tradeoff:** [What we gave up]
- **Impact:** [Performance/complexity change]

**Decision 2: [e.g., Work-weighted vs count-weighted]**
- **Rationale:** [Why we chose this]
- **Tradeoff:** [What we gave up]
- **Impact:** [Performance/complexity change]

[Repeat for 3-5 major decisions]

---

## Algorithm Insights

**Academic framing:**
- [Theoretical foundation: queuing theory, optimization, etc.]
- [Key equations/formulations - can be high-level or detailed]

**How it works (simplified):**
1. [Step 1]
2. [Step 2]
3. [Step 3]

**Critical implementation details:**
- [Detail 1 that engineers need to know]
- [Detail 2]

**Configuration parameters:**
| Parameter | Default | Purpose | Tuning guidance |
|-----------|---------|---------|-----------------|
| [param1]  | [val]   | [why]   | [when to change]|

---

## Alternatives Considered

| Approach | Pros | Cons | Verdict |
|----------|------|------|---------|
| [Option 1] | [✅ Benefit] | [❌ Problem] | [Why rejected/chosen] |
| [Option 2] | [✅ Benefit] | [❌ Problem] | [Why rejected/chosen] |

**Why we chose [Current Approach]:**
- [Reason 1]
- [Reason 2]

---

## Simulation Baselines

**Standard configuration:**
- [Config details: robot count, station count, etc.]

**Performance results:**
| Metric | Value | Target | Status |
|--------|-------|--------|--------|
| Throughput | [X picks/hr] | [Y picks/hr] | ✅/⚠️/❌ |
| Utilization | [X%] | [70-85%] | ✅/⚠️/❌ |
| TPQ | [X] | [Y ±0.5] | ✅/⚠️/❌ |

**Comparison to alternatives:**
- [vs PPF: +20% consistent throughput]
- [vs baseline: -15% early phase, +40% late phase]

**Link to detailed simulation data:** [Tab 5: Simulation Runs]

---

## Known Issues & Future Work

- ⚠️ [Issue 1: doesn't handle priorities yet]
- 🔮 [Future: adaptive weight tuning]
```

---

## Template 2: Weekly Updates (for Tab 4)

```markdown
# Weekly Updates (Reverse Chronological)

## Week of [Date] - [Optional: Sprint Name/Milestone]

**Shipped:**
- ✅ [Feature/fix 1]
- ✅ [Feature/fix 2]

**Key Findings:**
- 💡 [Insight from simulation/testing]
- 📊 [Metric improvement: +X% throughput]

**Blockers/Risks:**
- 🚧 [Blocker 1 - severity, owner]
- ⚠️ [Risk 1 - mitigation plan]

**Next Week:**
- 🎯 [Priority 1]
- 🎯 [Priority 2]

---

## Week of [Previous Date]

[Same format...]
```

---

## Template 3: System Architecture (for Tab 1)

```markdown
# System Architecture

## Control Flow (End-to-End)

### Forward Journey (Storage → Customer Order)

```
Order Created
   ↓
[Order Selection Algorithm] selects which orders to fulfill
   ↓
VTM picks tote from rack (in assigned zone)
   ↓
VTM delivers tote to relay position
   ↓ [Queue if relay full - backpressure]
HTM picks tote from relay (FIFO)
   ↓
HTM delivers tote to station
   ↓
Operator picks items (reuses StationProcessor)
   ↓
HTM picks up empty tote
```

### Return Journey (Tote back to Storage)

```
HTM returns tote to relay position
   ↓
VTM picks up empty tote (when zone capacity allows)
   ↓
VTM returns tote to rack
   ↓
Tote available for next order
```

---

## Component Overview

**Robots:**
- **VTM (Vertical Tote Mover):** Zone-bound, retrieves totes from racks
- **HTM (Horizontal Tote Mover):** Global, transports totes between relay and stations

**Infrastructure:**
- **Relay Position:** Handoff point with capacity limits (causes backpressure)
- **Zone:** Spatial division with VTM assignments
- **Station:** Operator workstation for picking

**Work Units:**
- **Tote:** Container (6 tasks capacity) carrying PickingTasks
- **Order:** Customer request with multiple SKUs
- **PickingTask:** Single SKU pick from specific storage space

---

## Key Design Tenets (Guiding Principles)

1. **Operators should not wait** - North star metric, optimize for operator utilization
2. **Navigation guides system design** - Order processing and task scheduling create pathfinding opportunities
3. **Every module meets north star and creates opportunity for downstream**
4. **Consistency over peaks** - Acceptable variance (±5%), predictable performance
5. **Diagnosis should be easy** - Observability built-in, black boxes discouraged

---

## Critical Mechanisms

**Zone Capacity Management:**
- Tracks: `currentOccupancy` (totes at relay) + `reserved` (in-transit)
- Utilization = `(occupancy + reserved) / physicalCapacity`
- VTMs must check zone-specific capacity (not global)

**Backpressure Control:**
- VTMs idle when zone >80% utilized
- Prioritize return journeys to clear congestion
- Prevents relay deadlock

**Set Cover Order Fulfillment:**
- Greedy algorithm minimizes storage spaces per order
- Active space bonus (+1000 score) prefers in-transit totes
- 20-40% PPF improvement from multi-order sharing

[Link to detailed algorithm docs: Tab 2]
```

---

## Template 4: Simulation Baseline Entry (for Tab 5)

```markdown
## [Baseline Name] - [Date]

**Purpose:** [Why we ran this simulation]

**Configuration:**
- System: RTP / Relay
- Robots: [X VTMs, Y HTMs / Z RTP bots]
- Stations: [N]
- Orders: [Count]
- Duration: [Hours]
- Algorithms: [Order selection, Put algorithm]

**Results:**

| Metric | Value | Target | Status |
|--------|-------|--------|--------|
| Orders Completed | [X] | [Y] | ✅ |
| Picks/Hr/Station | [X] | [Y] | ✅ |
| Operator Utilization | [X%] | [70-85%] | ⚠️ |
| Picks Per Face | [X] | [>2.0] | ✅ |

**Key Findings:**
- 💡 [Finding 1]
- 📊 [Finding 2]

**Configuration file:** `[path/to/config.json]`

**Raw output:** `[link or attachment]`
```

---

## Implementation Roadmap

### Phase 1: Foundation (Week 1)

1. Create Google Doc with 6 tabs
2. Fill out Tab 1 (Overview & Architecture) using Template 3
   - Control flow diagram
   - Component overview
   - Design tenets
3. Set up Tab 4 (Program & Updates) structure

### Phase 2: Algorithm Deep-Dives (Week 2-3)

4. Port TPQ design.md into Tab 2 using Template 1
5. Create sections for other algorithms (even if incomplete):
   - Task Scheduling
   - Path Planning
   - Put Algorithm
   - Set Cover
   - Zone Capacity & Backpressure
6. Link simulation results from Tab 5

### Phase 3: Operationalize (Week 3-4)

7. Add observability section (Tab 3)
   - Key metrics dashboard
   - Testing strategy
   - Known issues
8. Start weekly updates cadence (Tab 4)
9. Share with team for feedback

---

## Content Guidelines

### For Each Audience

**Executives:**
- Read: Tab 1 (Executive Summary only)
- Skim: Tab 4 (Weekly Updates - shipped items)
- Use: Quick metrics, high-level status

**Product/Program Managers:**
- Read: Tab 1 (Full architecture), Tab 4 (Weekly Updates)
- Reference: Tab 3 (Observability), Tab 5 (Simulation results)
- Use: Decision context, status tracking, roadmap

**Engineers:**
- Read: All tabs
- Deep dive: Tab 2 (Algorithms), Tab 3 (Testing)
- Reference: Tab 5 (Simulation baselines), Tab 6 (Config reference)
- Use: Implementation details, troubleshooting, design rationale

---

## Maintenance Guidelines

**Weekly:**
- Add entry to Tab 4 (Program & Updates) every Friday
- Update Tab 3 (Known Issues) as bugs discovered

**Per Algorithm Change:**
- Update relevant Tab 2 section (design decisions, insights)
- Add simulation baseline to Tab 5
- Update Tab 1 architecture if control flow changes

**Monthly:**
- Review and archive old weekly updates (keep last 8 weeks)
- Update Tab 1 executive summary with latest metrics
- Prune Tab 6 appendix of stale links

**As Needed:**
- Add new algorithm sections to Tab 2 when designed
- Update configuration reference in Tab 6
- Add new entries to glossary

---

## Example: TPQ Algorithm Section

For reference, here's how the TPQ algorithm from `specs/tpq_order_selection/design.md` maps to this template:

**From TPQ design.md → To Template 1:**

- **Problem Statement** → "Problem & Current Approach"
- **Proposed Solution** + **Metrics and Formulations** → "Algorithm Insights"
- **Theoretical Foundation** → "Algorithm Insights" (Academic framing)
- **Alternatives Considered** → "Alternatives Considered" (table format)
- **Implementation Approach** → Extract decision rationale → "Key Design Decisions"
- **FAQ** → Distribute across sections or Tab 3 (Known Issues)
- **Mathematical Derivations** → Tab 6 (Appendix) or link to original doc

**What to preserve:**
- Academic rigor (queuing theory, formulations)
- Design rationale (why TPQ over PPF)
- Simulation results
- Configuration parameters

**What to simplify:**
- Remove phase-by-phase implementation plan (that's for code, not company doc)
- Condense 100-page design into 10-page section
- Move deep math to appendix
- Focus on "current state" not "how we'll implement"

---

## Tools & Links

**Document Location:**
- [Google Doc URL - to be created]

**Related Code Documentation:**
- `CLAUDE.md` - Developer guide for codebase
- `specs/relay_system/design.md` - Original relay system design
- `specs/tpq_order_selection/design.md` - TPQ algorithm deep-dive

**Simulation Tools:**
- Backend API: `/simulation/run`
- Algorithm Comparison: `/comparison/run`
- Configuration examples: `apps/backend-java/src/test/resources/`

---

**End of Documentation Structure Guide**
