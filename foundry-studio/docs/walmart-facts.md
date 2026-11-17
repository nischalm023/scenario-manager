# Walmart DFW6 — facts of record

**Figures only.** Every row carries its source and the date that source was
issued. Judgement, rationale and history live elsewhere; this file exists so
that "which number is current" is answerable in one line.

Two rules for anyone editing it:

1. **A row without a source and a date does not belong here.**
2. **Where sources disagree, record BOTH in §9 and do not pick a winner here.**
   A silently-resolved conflict is worse than a visible one.
3. **Only what the Walmart engagement gave us.** GreyOrange product-generic
   data — Confluence pages, the Relay profile matrix — does not belong here,
   even as corroboration (ruling 2026-09-21).

Last reviewed: **2026-09-21**.

## How this document is kept current

Half of the job can be automated and half cannot. The split is deliberate.

### Automated — a checker, not yet in this repo

Every row whose Source column names **MAP** is recomputed from
`walmart_dfw6.json`, and every row naming **CODE** is read out of the source
file, so both classes are mechanically checkable. A checker that does this
exists and runs clean against the figures below: it re-derives the §8b
reconciliation from scratch, reports the rows that disagree, and checks the
`Last reviewed` date against a 30-day limit.

**It is deliberately not part of this change.** It depends on a rack-template
reorganisation that has not merged, so wiring it into the suite today would
either fail or have to be written against a layout we are about to replace. It
lands in a follow-up, as `scripts/check_walmart_facts.py` wired in as
`sim/dessim/tests/walmart_facts.rs`, and at that point drift fails the suite
and the map can no longer change under this document unnoticed.

**Until then every row here is maintained by hand**, which is exactly the
failure mode the follow-up removes. Treat a MAP or CODE figure as true as of
`Last reviewed` and no later.

It distinguishes three outcomes:

| Outcome | Meaning | Effect |
|---|---|---|
| **DRIFT** | a row disagrees with the source it cites | **test fails** |
| **CONFLICT** | a row whose Source carries `⚠ see §9.x` disagrees | reported, does not fail |
| **UNKNOWN** | a row whose value is literally UNKNOWN | reported, does not fail |

That second case matters: a row the document *knowingly* records as contested
is doing its job, so it must not be able to redden the suite. The `⚠ see §9.x`
marker is what tells the checker the disagreement is deliberate. **Adding that
marker to silence a failure you have not understood defeats the whole file.**

Once it lands, run it directly with `python3 scripts/check_walmart_facts.py` —
exit 0 clean, 1 on drift. To cover a new row, add one function to its `CHECKS`
registry keyed by the exact Figure text. Rows citing SDS, DoO, RACK, MOM,
SLACK or MAIL are not machine-checkable and are not expected to be.

### Manual — the part no script can do

A script cannot know that a new SDS revision landed, or that Vinay answered
§9.1. Those need a person, and the triggers are:

1. **A new source document arrives.** Digest it, then diff its figures against
   §1–§8 here. Anything that disagrees goes to §9 with both values, never
   silently overwritten. Anything superseded goes to §10.
2. **A §9 conflict is answered.** Move the figure into the body with the
   answering source and its date, delete the §9 entry, and note the retired
   conflict in §10 if the losing value is likely to resurface.
3. **The map changes on purpose.** The test will fail; update the row and bump
   `Last reviewed`.
4. **Nothing happened for 30 days.** The staleness check fails. Re-read §9,
   confirm the owners are still chasing, bump the date.

### Ownership

Every §9 entry names an owner. An entry with no owner is not a conflict, it is
a question nobody is asking — give it an owner or delete it.

---

## Sources

| Key | Document | Issued | Authority |
|---|---|---|---|
| **SDS** | Walmart Dallas — Simulation Design Specification (Vinay) | 2026-09-16 | **The brief for this simulator.** Highest authority for what we model. |
| **DoO** | DFW6 Pilot Design of Operations | 2026-09-16 (draft, "WIP") | Site operations. Full-scale reference unless a figure is marked pilot. |
| **RACK** | Rack design + install update (Chad / Steel King) | 2026-09-16 | Physical build and install. |
| **MOM** | DFW6 Pilot Workshop minutes, Days 1–2, Bentonville | 2026-09-14/15 | **Decisions.** Outranks drafts on anything it rules on. |
| **DXF** | `DFW6 Pilot_V.4_09Sept,2026.dxf` (AC1027 ASCII, from the V.4 DWG) | 2026-09-09 | **Measured drawing.** Block names carry real dimensions. |
| **MAP** | `sim/dessim/tests/fixtures/walmart/walmart_dfw6.json` | live | The as-built world document. Authoritative for what we actually run. |
| **SLACK** | Vinay, Slack, bot specs + operator times ("Bots specs to be used") | 2026-09-17 | **Directive.** Vinay authors the SDS; treat as an SDS addendum for what we model. |
| **MAIL** | "Relay Simulation Cadence" email thread, Walmart ↔ GreyOrange; rows cite author and date | 2026-09-14 → 2026-09-21 | Two things in one thread. **Walmart's PoT measurements** (Arvindhan, 09-14 and 09-15): the only Walmart-measured bot figures we hold. **Vinay's 09-21 spec list**: SLACK's successor — the figures are GreyOrange's internal hardware team's, HAI vetting pending (Akshay), "for this week". Directive where it differs from SLACK. |

---

## 1. Field and layout

| Figure | Value | Source |
|---|---|---|
| Field size (L x W) | 68,532 x 92,112 mm | SDS §2.1 |
| Aisles | 20 | SDS §2.2 |
| Single Bay aisles (single-sided) | 1, 7, 20 | SDS §2.2 |
| Double Bay aisles (back-to-back) | the other 17 | SDS §2.2 |
| North highway lanes | 7 | SDS §2.1 |
| South highway lanes | 7 | SDS §2.1 |
| Super highways | pairs of unidirectional N→S lanes | SDS §2.1 |
| Super highway pairs | **5** (was 3; +2 added between existing) | RACK |
| Super highway target flow | 200 in + 200 out totes/hr per pair | SDS §2.1 |
| Relay positions below super-highway lanes | **none** | SDS §2.1 |
| Aisle width (install constraint) | 48 in = 1,219.2 mm | RACK — ⚠ see §9.1 |
| Station 10 source-tote unload point | direct path to under the racking, in the layout reviewed at the workshop | MAIL 09-14 (Arvindhan) — ⚠ see §9.15 |

## 2. Racking

| Figure | Value | Source |
|---|---|---|
| Storage levels | 23 | SDS §2.2 |
| **Rack height** | **480 in = 12,192 mm = 40 ft** | **DXF** (frame-line block names), DoO §6 |
| Single-bay frame depth | **59.5 in = 1,511.3 mm** | DXF |
| Double-bay (back-to-back) frame depth | **116.5 in = 2,959.1 mm** | DXF |
| Back-to-back shared-frame saving | 63.5 mm (2 x 59.5 in = 119 in vs 116.5 in) | DXF, derived |
| Storage frame lines | 704 double-bay, 352 single-bay (2:1) | DXF |
| Tunnel frame line | 116.5 in x 200 in = 2,959.1 x 5,080 mm | DXF |
| Positions per bay, Single Bay Double Deep | **230** (23 x 10) | SDS §2.2 |
| Positions per bay, Double Bay Double Deep | **460** (23 x 10 x 2 sides) | SDS §2.2 |
| Bays per aisle | 19 | SDS §2.2 |
| Locations per SBDD aisle | 4,370 | SDS §2.2 |
| Locations per DBDD aisle | 8,740 | SDS §2.2 |
| **Total storage, excl. tunnel** | **161,690** | SDS §2.2 |
| **Total storage, incl. tunnel** | **170,570** | SDS §2.2 |
| Bay width (plain) | 2,532.8 mm | MAP |
| Bay width (tunnel) | 2,012.1 mm | MAP |
| Slot pitch, x | 506.6 mm (plain) / 402.4 mm (tunnel) | MAP |
| Slot pitch, y | 749.0 / 750.0 / 783.5 / 790.0 mm | MAP — ⚠ see §9.3 |
| Bay depth | 1,498.0 – 1,580.0 mm | MAP |
| First level height | 800 mm | MAP |
| Level step | 390.9 mm | MAP |
| Back-to-back pair offset | 1,500.0 mm (one bay depth) | MAP (13 of 37 run positions) |
| Steel King front-to-back tolerance | ±½ in = 12.7 mm | RACK |

**Units note.** `walmart_dfw6.json` declares `unit_mm: 0.1` on its primary
frame, so raw integers there are **tenths of a millimetre**. All MAP rows above
are converted.

## 3. Tunnel

| Figure | Value | Source |
|---|---|---|
| Tunnel bays | 12 and 13 | SDS §2.2 |
| Levels skipped | 11 (storage starts at level 12) | SDS §2.2 |
| Levels per tunnel bay | 12 | SDS §2.2, MAP |
| Positions per tunnel bay, Single Bay | 120 (12 x 10) | SDS §2.2, MAP |
| Positions per tunnel bay, Double Bay | 240 (12 x 10 x 2) | SDS §2.2 |
| **Total tunnel storage** | **8,880** | SDS §2.2, MAP |
| Tunnel start height | "approx 18.17 m" | SDS §2.2 — ⚠ see §9.2 |

## 4. Relay positions

| Figure | Value | Source |
|---|---|---|
| Bays with NO relay | 1, 4, 11, 12, 19 | SDS §2.3 |
| Relay locations per active bay | 4 | SDS §2.3 |
| Per SBDD aisle | 56 (4 x 14) | SDS §2.3 |
| Per DBDD aisle | 112 (4 x 14 x 2) | SDS §2.3 |
| **Total relay locations** | **2,072** | SDS §2.3 |

## 5. Buffer

| Figure | Value | Source |
|---|---|---|
| Buffer positions (pilot) | **2,560** across **16 levels** | RACK |
| Buffer positions (full scale) | 27,600 (75 bays x 23 lv x 16) | DoO §6.1 |
| Buffer positions (our map) | **3,450** = 15 bays x 23 levels x 10 | MAP — ⚠ see §9.4 |
| Buffer stores | trays AND cartons together | RACK |
| Buffer/Relay storage zones | physically separate, no shared rack | DoO §6 |

## 6. Fleet

| Figure | Value | Source |
|---|---|---|
| HTM target fleet | 230 | SDS §3.1, DoO §4.3 |
| HTM, pick only | 80 | SDS §3.1 |
| HTM, pick + decant/putaway | 144 | SDS §3.1 |
| HTM, + order extraction | 230 | SDS §3.1 |
| VTM target fleet | 72 | SDS §3.1, DoO §4.3 |
| VTM, pick only | 49 | SDS §3.1 |
| **VTM active, baseline V1** | **40** (2/aisle, rest charging) | SDS §3.1 |
| **HTM model** | **K50** — "230 K50 HTMs" | **MOM Day 1 §H** |
| **VTM model** | **suction-cup C56** — "72 suction-cup VTMs" | **MOM Day 1 §H** |

### 6b. Bot specs to be used (MAIL 2026-09-21, superseding SLACK 2026-09-17)

Vinay's directive figures, restated by email on 2026-09-21 with their
provenance: received from GreyOrange's internal hardware team, HAI vetting in
progress (Akshay), to be used "for this week" while the model stabilises. The
restatement changes one figure — the single 4.0 s VTM alignment becomes a pick
and a drop figure — and leaves every other row as SLACK gave it. Both VTM
families (relay storage and order buffer) take the same numbers, now in
writing. Loaded and unloaded are equal throughout.

| Figure | Value | Source |
|---|---|---|
| **VTM speed (loaded = unloaded)** | **1.5 m/s** | SLACK, MAIL 09-21 — ⚠ see §9.11 |
| **VTM accel (loaded = unloaded)** | **0.5 m/s²** | SLACK, MAIL 09-21 — ⚠ see §9.11 |
| **VTM pick alignment time** | **3.56 s** | **MAIL 09-21** (was one 4.0 s figure in SLACK, §10) — ⚠ see §9.11, §9.13 |
| **VTM drop alignment time** | **2.3 s** | **MAIL 09-21** (same) — ⚠ see §9.11, §9.13 |
| VTM pick time | 4.0 s | SLACK, MAIL 09-21 — ⚠ see §9.11, §9.13 |
| VTM drop time | 4.0 s | SLACK, MAIL 09-21 — ⚠ see §9.11, §9.13 |
| **HTM speed (loaded = unloaded)** | **4.0 m/s** | SLACK, MAIL 09-21 — ⚠ see §9.11, §9.13 |
| **HTM accel (loaded = unloaded)** | **1.8 m/s²** | SLACK, MAIL 09-21 — ⚠ see §9.11 |
| HTM pick time | 2.0 s | SLACK, MAIL 09-21 — ⚠ see §9.13 |
| HTM drop time | 2.0 s | SLACK, MAIL 09-21 — ⚠ see §9.13 |
| HTM lift, min to max | 5.28 s | SLACK, MAIL 09-21 — ⚠ see §9.12 |
| HTM lift, min to cantilever height 1 | 2.0 s | SLACK, MAIL 09-21 — ⚠ see §9.12 |
| HTM lower, max to min | 5.28 s | SLACK, MAIL 09-21 — ⚠ see §9.12 |
| HTM lower, to cantilever height 1 | 2.0 s | SLACK, MAIL 09-21 — ⚠ see §9.12 |
| **HTM 90 degree turn (loaded = unloaded)** | **2.16 s** | SLACK, MAIL 09-21 — ⚠ see §9.11 |
| **HTM 180 degree turn (loaded = unloaded)** | **2.88 s** | SLACK, MAIL 09-21 — ⚠ see §9.12 |
| Order bots per waiting block | 6 | SLACK, MAIL 09-21 |


## 7. Stations and throughput

| Figure | Value | Source |
|---|---|---|
| Pick (EasyLoop) stations | 4 | SDS §3.1, DoO §4.3 |
| Decant / induct stations | 6 | SDS §3.1, DoO §4.3 |
| Order extraction stations | 3 | SDS §3.1, DoO §4.3 |
| **EasyLoop mechanical max** | **360 presentations/hr** (10.0 s each) | SDS §4.2 |
| **EasyLoop target rate** | **290 presentations/hr** | SDS §4.2 |
| **Operator dwell** | **10 s per pick** | SDS §4.2, MOM Day 2 §C — ⚠ see §9.6 |
| **Station cycle window** | **12 s** = 10 s operator + 2 s machine switching | **MOM Day 2 §C** |
| Achieved rate, static loading | 320–330 UPH at ~11 s cycle | MOM Day 2 §C |
| Centre-to-centre tote distance | 740 mm (was 720) | MOM Day 2 §C |
| Source-tote tilt | 10° default, 20° lift | MOM Day 2 §C |
| **Operator time, induction station** | **60 s** | **SLACK**, MAIL 09-21 |
| **Operator time, EasyLoop station** | **10 s** | **SLACK**, MAIL 09-21, SDS §4.2, MOM Day 2 §C |
| Operator time, extraction station | **UNKNOWN — Vinay is asking**; not in MAIL 09-21 | SLACK |
| Target UPH per station | 320 | SDS §4.2 |
| Total pick throughput | 1,280 UPH / 1,160 presentations/hr | SDS §4.2 |
| Mechanical max, all stations | 1,440 presentations/hr | SDS §4.2 |
| Waiting block | 6 cells x 2 blocks | SDS §4.2 |
| Packsize machines | 2 | SDS §4.2 |
| Flex (SIOC) rate | 100 UPH, bypasses EasyLoop | SDS §4.2, DoO §3.1 |
| Outbound flow | 25,600 units/day over 20 h | DoO §3.1 |
| Inbound flow | up to 20,000 units/day over 20 h | DoO §3.1 |
| Decant rate | 250 UPH per active station | DoO §3.1 |
| **HTM tote unload to the EasyLoop conveyor** | **~8.0–8.1 s** = drop 2.47 s + fork to fully down 5.52 s, consistent across both FAT days; the HTM blocks the station for this | **MAIL 09-15** (Arvindhan, PoT) — ⚠ see §9.13 |
| **HTM tote load from the EasyLoop conveyor** | **4.61 s** | **MAIL 09-15** (Arvindhan, PoT) — ⚠ see §9.13 |
| HTM unload/load at induct and extract conveyors | 11–12 s per tote/carton, data "not clean"; Walmart proposes the EasyLoop figures since the mechanism is the same, pending Akshay/Kalpesh | MAIL 09-15 (Arvindhan) |
| HTM pick/drop at the relay position | 2–2.5 s is "accurate only" here, not at any conveyor | MAIL 09-15 (Arvindhan) |
| Decant station design | conveyor-based, approved at the 09-15 workshop; cycle times to follow EasyLoop's, GO solutions validation (Kalpesh/Akshay) pending | MAIL 09-16 (Arvindhan), MAIL 09-21 (Vinay) |
| Put (decant) station positions | 4 put positions + 2 queueing positions | MAIL 09-14 (Arvindhan) |

## 7b. Order release (MAIL 2026-09-15, Arvindhan and Nikita; acknowledged MAIL 09-21)

| Figure | Value | Source |
|---|---|---|
| T0 | 00:00 (midnight) | MAIL 09-15 |
| Dropped at T0 | every order whose dataset drop time falls in 21:00–00:00 — a four-hour pool | MAIL 09-15 |
| Thereafter | one hour of orders each hour, by dataset drop time | MAIL 09-15 |
| Prioritisation | order cut-off times | MAIL 09-15 |
| All orders at T0 | model-optimisation mode only ("true mechanical output of the system with no constraints") | MAIL 09-15, MAIL 09-21 |
| Sign-off runs | must use the staggered drops above | MAIL 09-15 |

## 8. Containers and inventory

| Figure | Value | Source |
|---|---|---|
| Tote / order tote / carton | 650 x 450 x 325 mm, 30 kg | SDS, DoO §3.1, RACK |
| Tray footprint | same 650 x 450 as tote | RACK |
| Orders per tote (batch) | 5 | SDS §4.r |
| Orders per box | 1 | SDS §4.r |
| Mock-put fill | 80% (20% left for rearranging) | SDS §5.2 |
| Units per tote, max | 100 | SDS §5.2 |
| SKUs per tote | 1 | SDS §5.2 |
| On-hand catalogue | ~175k+ SKUs (`onhand.csv`) | SDS §5.2 |
| Orders with multiple units ("multis") | 7% of orders, 17% of units | MAIL 09-14 (Arvindhan) |
| Multi-unit / multi-line orders in the order file | 1,647 / 1,334 | MAIL 09-21 (Vinay, counted from the file) |

## 8c. Trays (MOM Day 1 §C, Option 2 — working direction)

| Figure | Value | Source |
|---|---|---|
| Tray outer | 650 x 450 mm (= tote footprint) | MOM Day 1 §C |
| Tray inner | 615 x 420 mm | MOM Day 1 §C |
| Max carton when trayed | 600 x 400 mm | MOM Day 1 §C |
| Tray height, minimum | 100 mm (sufficiency unverified) | MOM Day 1 §C |
| Larger-tray scenario | 700 x 500 mm, open | MOM Day 1 §C |
| Cartons below 400 x 300 mm | 50–56% of carton-only orders | MOM Day 1 §B |
| Carton task cost | **~double cycle vs a direct tote path** | MOM Day 1 §D |
| Order totes on trays | **never** — trays are carton-only | MOM Day 1 §E |
| Fleet model | one K50 agent class, task-type routing selects sub-fleet | MOM Day 2 §B |

## 8b. Our map, reconciled against SDS §2.2 (2026-09-16)

| Bucket | Positions |
|---|---|
| Inventory, excl. tunnel and order-designated | 153,180 |
| Order-designated pre-staging rows | 8,510 |
| **Subtotal = SDS "excl. tunnel"** | **161,690** ✓ exact |
| Tunnel | 8,880 ✓ exact |
| **Subtotal = SDS "incl. tunnel"** | **170,570** ✓ exact |
| Buffer (`holds: order`, relay steel, 23 levels) | 3,450 |
| **Map total** | **174,020** |

The map matches every SDS storage figure exactly. The only excess is the 3,450
buffer, which SDS §2.2 does not include in its totals.

---

## 9. WHERE SOURCES DISAGREE

Open conflicts. Each needs one answer from a named owner. **Do not average, and
do not silently prefer one.**

### 9.1 VTM aisle width — THREE values

| Value | Source |
|---|---|
| 1,219.2 mm (48 in) | RACK install constraint, 2026-09-16 |
| 1,225.0 mm | MAP, derived (27,250 − 15,000 tenths between run positions) |
| 1,227 mm | Vinay's Pilot Grid v2 xlsx decode; `WALMART.md` §8 |

The three sit within 8 mm of one another — RACK's 48 in install constraint and
two decodes of the drawing — so this is one figure measured three ways. The
open question is which one the pathfinder uses for clearance. **Owner: Vinay**
— confirm the design aisle width.

### 9.2 Rack height — RESOLVED at 40 ft; tunnel start height still wrong

**Closed 2026-09-17 by the DXF.** Every storage and buffer frame-line block is
named `… x 480in`, i.e. **12,192 mm = 40 ft**, matching DoO §6's "12 metres (40
feet)". **SDS §2.2's "32 feet high" is wrong** — moved to §10.

| Claim | Source | Verdict |
|---|---|---|
| 480 in = 40 ft | DXF block names, DoO §6 | **correct** |
| 32 ft | SDS §2.2 | superseded |
| Tunnel storage starts at "approx 18.17 m" | SDS §2.2 | **still wrong** — exceeds a 12.192 m rack |

The tunnel figure remains unresolved: 18.17 m cannot sit inside a 12.192 m
frame. 18.17 **ft** = 5.54 m is plausible, and the DXF's tunnel frame line is
**200 in = 5,080 mm**, which is close. **Owner: Vinay** — confirm the unit.

18.17 m exceeds **both** stated heights, so the tunnel figure is wrong or its
unit is not metres (18.17 ft = 5.54 m is plausible; our level 12 sits at
5,099.9 mm). 32 ft vs 40 ft is a separate disagreement. **Owner: Vinay** for the
tunnel unit; **Chad** for the height envelope.

### 9.3 Bay depth — four values, probably fewer nominals

749.0 / 750.0 / 783.5 / 790.0 mm (MAP). The first two differ by **1 mm**, well
inside Steel King's stated ±12.7 mm tolerance, so they are almost certainly one
nominal measured twice. **Owner: Steel King** — is depth adjustable on one
frame, or is each depth a part number?

### 9.4 Buffer capacity — three values

2,560 / 16 levels (RACK, pilot) · 27,600 / 23 levels (DoO §6.1, full scale) ·
**3,450 / 23 levels (MAP, as of 2026-09-17)**.

**Our buffer is the wrong depth.** It currently reuses the relay storage
template at 23 storage levels, giving 15 bays x 23 x 10 = 3,450 against the
stated 2,560. RACK is explicit that the buffer is a **16-level** rack, so the
map overstates buffer capacity by 890 positions and gets there with the wrong
level pitch.

A 16-level buffer template is prepared but not merged. Once it lands the map
gives 15 x 16 x 10 = 2,400, which is the right depth and **160 positions short**
— exactly one bay, since 2,560 is 16 bays.

Adding the 16th bay needs a position from the layout, which nobody has given us.
**Owner: Chad / Vasu** — where does the 16th buffer bay go?

⚠ In the pending 16-level template the level HEIGHTS are INFERRED, not
measured: the V.4 drawing shows buffer frames at the same 480 in as storage, so
the buffer is not a shorter rack and the pitch must be coarser. It uses 800 mm
base + 573.3 mm pitch, spreading 16 levels across storage's envelope. Confirm
against an elevation before that template lands.

### 9.5 Bot kinematics — RESOLVED as to WHICH robot; our numbers are wrong

**Closed 2026-09-16 by MOM Day 1 §H:** *"Bot BOM for the pilot is unchanged: 72
suction-cup VTMs and 230 K50 HTMs."* The HTM is a **K50**, not an M5F, and the
VTM is a **suction-cup C56**. Day 1 is built entirely around the K50; no M5F
appears in either day's minutes.

Our simulator was calibrated to the Quicktron M5F (HTM, 4.5 m/s empty / 4.2 m/s
loaded) and the A5/DA50 (VTM, 3.0 m/s); the pilot BOM is K50 HTMs and
suction-cup C56 VTMs.

**Both fleets are calibrated to the wrong robots.** This is no longer a conflict
between sources — it is a defect with a known correct value. Action, not
question.

**Numbers supplied 2026-09-17 (SLACK), restated 2026-09-21 (MAIL).** Vinay gave
the figures to use; see §6b. The remaining work is entirely ours: see §9.11 for
the code delta.

Remaining sub-question: the BOM says 230 K50s without saying whether that is
one variant or two (an inventory-tote carrier and an order carrier). The Day 2
flywheel model says one agent class with task-type routing, so any split may be
operational rather than hardware. **Owner: Kalpesh** (fleet-logic analysis,
Day 1 action item 6).

**New sub-conflict — the model names themselves (SLACK, 2026-09-17).** Vinay
listed all three with question marks, so these are proposals, not assertions:

| Bot | Vinay asks | MOM Day 1 §H | MAP |
|---|---|---|---|
| VTM, relay storage | "A42T vacuum vtm?" | suction-cup C56 | `VTM` |
| VTM, order buffer | "gripper vtm?" | (not split out) | `VTM-G` |
| HTM | "HAI-K50?" | K50 | `RB-K50H` |

Three different spellings of the HTM across three sources, and a **VTM family
disagreement** (A42T against C56). That confusion already cost us a withdrawn
travel-time alarm on 2026-09-16.

This matters only for provenance, not for the numbers: Vinay's §6b figures are
directive regardless of what the bot is called. **Owner: Vinay** (he raised it).

### 9.6 Operator dwell — 9 vs 10 seconds

SDS §4.2 says **10 s per unit**; MOM Day 2 §C independently says **~10 s/pick**
inside a **12 s** station window (10 operator + 2 machine switching). Our code
uses **9.0 s** (`config/subsystems.rs` `time_per_pick_secs`).

**Three sources now agree on 10 s** (SLACK 2026-09-17 makes it unanimous, and
is from the person who wrote the SDS row; MAIL 09-21 restates it). **Adopt it. Owner: us** — the code
still says 9.0 and no external input is needed.

**And the larger defect it exposes:** our `conveyor.rs` applies `dwell` per seat
across two seats with no shared operator, so the model effectively has **two
operators per station** and produces ~800 presentations/hour against an actual
**300/hour** (12 s window). The operator must be one exclusive station resource.
That is the fix, not a seat changeover. See §9.10.

### 9.10 Station throughput — our model is ~2.7x over

| Claim | Source |
|---|---|
| 300 presentations/hr (12 s window: 10 s operator + 2 s switching) | MOM Day 2 §C |
| 290 presentations/hr target | SDS §4.2, MOM Day 2 §C |
| 360 presentations/hr mechanical max | SDS §4.2 |
| **~800/hr** (2 seats x 9 s dwell, no shared operator) | **our `conveyor.rs`** |

Not a source conflict — a model defect. The operator is a single exclusive
resource serving both seats; we model it as two. **Action: make the operator
exclusive and adopt the 12 s window. Owner: us** — no external input needed.

### 9.11 Our kinematics vs the specs to be used — every figure is wrong

Not a source conflict. §6b gives the values; these are the code constants that
disagree. **Nothing here is a question; it is a defect list.**

| Parameter | Our code | Spec (§6b) | Wrong by | Where |
|---|---|---|---|---|
| VTM speed | 6.0 m/s | 1.5 m/s | **4.0x fast** | `config/robots.rs` `VtmConfig::default` |
| VTM speed (motion model) | 3.0 m/s | 1.5 m/s | **2.0x fast** | `robots/vtm/motion.rs` `H/V_MAX_SPEED` |
| VTM accel | 2.0 m/s² | 0.5 m/s² | **4.0x high** | `robots/vtm/motion.rs` `H/V_ACCEL` |
| VTM pick / drop | 4.0 / 2.0 s | 4 / 4 s | drop 2.0x fast | `VtmConfig::default` `pick_secs` / `drop_secs` (the 2.25 s `load/unload_time_secs` pair only feeds a handoff estimate) |
| VTM alignment | 1.933 s, one scalar, charged once per fork insertion | pick 3.56 s / drop 2.3 s | 1.8x / 1.2x fast, and one figure where the spec has two | `robots/vtm/motion.rs` `ALIGNMENT_SECS` |
| HTM speed, loaded | 4.2 m/s | 4 m/s | 5% fast | `HtmConfig::default` |
| HTM speed, empty | 4.5 m/s | 4 m/s | 13% fast | `QuicktronM5f::EMPTY_MAX_SPEED` |
| HTM accel | 2.0 m/s² | 1.8 m/s² | 11% high | `QuicktronM5f::{EMPTY,LOADED}_ACCEL` |
| **HTM 90° turn** | **1.0 s** | **2.16 s** | **2.16x fast** | `HtmConfig::default` |
| **HTM 180° turn** | **2 x 90° = 2.0 s** (no 180° action) | **2.88 s** | 1.4x fast | `pathfinding/fh_sipp/expand_rotate.rs` (90° successors only; `rotation_duration` scales linearly with angle) |
| HTM pick / drop at the relay | 2.0 s | 2 s | ✓ correct | `HtmConfig::default` |
| HTM handoff at a station conveyor | 2.0 s — the same `pickup_secs` / `deposit_secs` as the relay | ~8 s unload / 4.61 s load (PoT, §7) | 4x / 2.3x fast | `native-modules/src/migration/flows/fulfillment/{supply_conveyor,collect_tote,induct_container}.rs` |
| HTM lift / lower | not modelled | 5.28 s full, 2.0 s to cantilever height 1 | missing | — |

**The turn time deserves separate notice.** `HtmConfig::default` sets
`loaded_rotate: 1.0` with the trailing comment
`// QuicktronM5f::ROTATION_90_LOADED` — but that constant is **2.1**, not 1.0.
The comment names a source the value does not come from. The M5F figure was
within 3% of Vinay's 2.16; an undocumented override to 1.0 is what put us 2.16x
out. Same for `empty_rotate`.

Every error above is in the direction of **an optimistically fast robot**, so
the current model overstates throughput. That compounds with §9.10's 2.7x
station overstatement; they are independent defects in the same direction.

Note the bundled macOS demo runs `motion_model: htm_kinematic`, so the HTM
figures here are live in every build shipped so far. The VTM constant-speed
figure of 6.0 m/s is live regardless of motion model.

**Owner: us.** No external input needed.

### 9.12 Open questions from Vinay's spec (SLACK, 2026-09-17)

Vinay left these blank or asked them outright. **Owner: Vinay.**

| Question | What we currently model |
|---|---|
| Totes on the conveyor presenting to the operator | `EASYLOOP_CAPACITY = 8` (`conveyor.rs`) |
| Operator working time at the extraction station | no distinct figure |
| Bot model names (all three) | see §9.5 |

And two of ours, raised by the shape of his list rather than by him:

- **VTM alignment — narrowed by MAIL 09-21.** The single 4 s is now a pick
  figure (3.56 s) and a drop figure (2.3 s), i.e. per operation. What remains
  open is only whether each is charged once per fork insertion, which is the
  shape our model has (one scalar, twice per cycle). See §9.13 for the PoT's
  1.72 s against both.
- **Is the 180° turn (2.88 s) a primitive, or two 90° turns?** At 2.16 s each a
  pair would be 4.32 s, so 2.88 s is **not** two 90s. If we compose 180s from
  90s we will be 1.5x slow on reversals, the one error in the conservative
  direction.
- **What do the HTM lift rows attach to?** Walmart's PoT decomposes the
  EasyLoop unload as drop 2.47 s + fork fully down 5.52 s ≈ 8 s (§7); Vinay's
  5.28 s lift/lower rows are the hardware team's figure for the same fork
  travel. We charge one 2 s pickup/deposit at relays and station conveyors
  alike (§9.11). Whether unload = drop + full lower and load = pick + lift to
  cantilever height 1 is our reading, not Vinay's statement. **Owner: Vinay**
  to confirm the decomposition; us to model it.
- **Operator time at the extraction station** — still absent from MAIL 09-21.
- **Decant pick and drop times** — Vinay's own question (MAIL 09-15); answered
  by policy, not measurement: the EasyLoop figures, pending Kalpesh/Akshay
  validation (§7).

### 9.13 Walmart's PoT measurements vs the directive (MAIL 09-14/09-15 vs SLACK/MAIL 09-21)

Arvindhan measured these during the PoT and put them against our inputs
(09-14) and the station cycle (09-15). Vinay's 09-21 reply supplied the
hardware team's list without addressing the measurements row by row, so both
values stand. **Owner: Vinay / Akshay** — the HAI vetting is the adjudication.

| Parameter | Walmart PoT (MAIL) | Directive (§6b) | Note |
|---|---|---|---|
| HTM loaded travel | "as per the spec sheet, 2 m/s" (09-14) | 4.0 m/s | the spec sheet Arvindhan read is not identified |
| HTM pickup | 3.46 s (09-14) | 2.0 s | |
| HTM deposit | 2.3 s (09-14) | 2.0 s | |
| HTM fork lowering | 5.52 s (09-14, 09-15) | 5.28 s lower, max to min | agree within 5% |
| HTM unload at EasyLoop | 10–12 s "even with simultaneous lift" (09-14); **~8.0–8.1 s** = 2.47 + 5.52 (09-15) | not a directive row | two Walmart figures; the 09-15 one is the measured decomposition |
| HTM load at EasyLoop | 4.61 s (09-15) | not a directive row | |
| VTM fork pick / drop | 1.6 s / 1.85 s (09-14) | 4 s / 4 s | |
| VTM alignment | 1.72 s (09-14) | 4.0 s (SLACK) → 3.56 s pick / 2.3 s drop (MAIL 09-21) | |

One coincidence to put to Vinay rather than resolve here: the directive's VTM
alignment figures (drop 2.3 s, pick 3.56 s) sit within 0.1 s of the PoT's
**HTM** deposit (2.3 s) and pickup (3.46 s). Whether that is a coincidence or a
transposed row is a question, not a finding.

### 9.14 SKUs per tote and the tote count

| Claim | Source |
|---|---|
| 1 SKU per tote | SDS §5.2 (§8 above) |
| "Inventory totes may contain multiple SKUs and multiple units of each SKU" | MAIL 09-21 (Vinay, to Walmart) |
| "almost every tote holds only one SKU … roughly 130,000 totes being needed, not 13,386" | MAIL 09-14 (Arvindhan, reading our output) |

The SDS and the statement to Walmart disagree on the mixing rule, and the
13,386 Arvindhan read off our output is not a figure this document knows (the
full preload uses 162,060 totes, MAIL 09-21). **Owner: Vinay** — which mixing rule is the
brief, and where 13,386 came from.

### 9.15 Station 10 approach — which drawing is current

| Claim | Source |
|---|---|
| The station 10 queueing point has no straight path to under the racking | MAIL 09-14 (Arvindhan, reading our floor map) |
| The layout reviewed at the workshop shows a direct path from the station 10 source-tote unload point to under the racking | MAIL 09-14 (Arvindhan) |
| The map has since been reworked with direct EasyLoop approaches and relay access cells under the relays | MAIL 09-21 (Vinay) |

Validating the exact route waits on knowing which source drawing is current;
our newest is the V.4 DXF of 2026-09-09. **Owner: Vinay** — identify the
current drawing; us to validate station 10 against it.

### 9.16 Return-tote destination — awaiting Walmart's rule

Two questions we put back to Walmart on 09-21, after Arvindhan observed that
returning source totes travel far and almost never go back where they came
from:

- Must a source tote use the rack directly opposite its EasyLoop, merely
  prefer it, or use the nearest eligible rack?
- Return-to-origin, nearest eligible slot, zoned placement, or another
  deterministic rule? Each is practical and each changes travel and
  congestion materially.

Today the destination is chosen dynamically (HTM travel, relay readiness, VTM
availability). **Owner: Arvindhan.**

### 9.7 Tunnel geometry — DoO contradicts SDS

| Claim | Source |
|---|---|
| 12 levels, 120/bay single, 240 double | SDS §2.2 **and MAP** |
| Full tunnel 10 levels @ 16/level; half tunnel 10 @ 6/level | DoO §6.1 |

Our map follows the SDS, which is correct for a simulation brief. Separately,
the DoO's own half-tunnel figure is not half its full-tunnel figure (6 vs 8),
unlike the storage racks which split exactly 20:10. **Owner: whoever authored
DoO §6.1.**

### 9.8 EasyLoop station count — a units-of-counting question

DoO §4.3 lists "EasyLoop Station: 4" and SDS §3.1 "Pick Stations: 4", but
`specs/easyloop/DESIGN.md` defines an EasyLoop as **two stations sharing one
conveyor**. So 4 stations may be 2 machines. Our map has 4 `go.easyloop`
components. **Action: confirm before any station-count or belt-count work.
Owner: Vinay** — the SDS and DoO both count 4 without saying 4 of what.

### 9.9 Minor, inside the RACK update itself

82 truckloads vs "14 trucks/week for 6 weeks" (= 84) · "delivery on site will
start 01/18/26" should be 01/18/27 · install stated as 12 weeks but 18 Jan –
9 Apr is ~11.6. **Owner: whoever authored it.**

---

## 10. Superseded — do not reuse

| Figure | Why |
|---|---|
| "Full-workflow HTM fleet is 144" (`spec-review-2026-09-08.md`) | SDS §3.1: 144 is pick+decant/putaway; full workflow is 230 |
| Buffer 27,600 as a pilot figure | DoO full-scale; pilot is 2,560 |
| DoO §6.1 rack counts as pilot figures | DoO §3.1 states they are full scale |
| 21 bays @ 2,697 mm (earlier pilot decode) | superseded by SDS's 19 bays |
| Rack height 32 ft (SDS §2.2) | DXF frame lines are 480 in = 40 ft; DoO §6 agrees |
| HTM = M5F | MOM Day 1 §H: 230 **K50** HTMs |
| "GreyMatter must treat trayed and untrayed K50s as two agent classes" | MOM Day 1 §D, **reversed by** MOM Day 2 §B (one class, task-type routing) |
| Seat changeover as the EasyLoop fix | the defect is the shared operator, not the seat — §9.10 |
| VTM alignment 4.0 s, one figure (SLACK 09-17) | MAIL 09-21 splits it: pick 3.56 s, drop 2.3 s |
| HTM pick/drop time at conveyor "UNKNOWN" | measured in the PoT: ~8 s unload, 4.61 s load — §7 |
