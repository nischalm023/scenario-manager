# Robot models

One world-proto document per robot make, model and configuration the simulator runs, each holding one robot
entity. An entity describes a robot by one component per aspect of the machine, and the set of components it
carries is its type: there is no role field. Whether a robot works as an HTM or a VTM is the simulator's choice,
made from its cohort's assignment. These files show what each robot's entity will look like in the world graph's
catalog; a site's world file carries the entities its cohorts use.

| file | robot | runs at |
|---|---|---|
| `quicktron_m5f.world.json` | Quicktron M5F (Dematic D30), HTM | Cencora |
| `quicktron_a5.world.json` | Quicktron A5 (Dematic DA50), VTM that rides the racking | Cencora |
| `hai_k50.world.json` | HAI Robotics K50H (RB-K50H in the map), HTM | Walmart DFW6 |
| `quicktron_c56_suction.world.json` | C56 with suction cup, relay storage VTM (model disputed) | Walmart DFW6 |
| `vtm_g_gripper.world.json` | VTM-G with gripper, order buffer VTM (maker unconfirmed) | Walmart DFW6 |

## Components

- `lineage`: who made the robot and who resold it (world-proto's own, SPEC section 10.1).
- `body`: `footprint_mm`, `height_mm`.
- `floor_drive`: driving on the floor. It has `loaded` and `empty` blocks, each with `max_speed_mm_s`, `accel_mm_s2`, `quarter_turn_secs` and, when a source states one, `reversal_secs` (a 180° turn in place as one action).
- `rack_ladder`: riding the racking along one aisle: `max_speed_mm_s`, `accel_mm_s2`.
- `lift`: raising and lowering what holds the carrier. It gives a speed `profile` (`max_speed_mm_s`, `accel_mm_s2`), or timed `strokes` between named heights (`from`, `to`, `secs`).
- `end_effector`: the part that takes and places a carrier. It gives a `kind`, `pick_secs`, `drop_secs` and, when a source states them, `align_before_pick_secs`, `align_before_drop_secs` and `reach_slots_deep`.

## Conventions

- **Units are in each figure's name:** mm/s, mm/s², mm and seconds. A robot entity has no site frame to take a step from, so world-proto's rule of stating lengths and speeds in frame steps (SPEC section 5) does not reach these figures.
- **A figure no source states is left out.** When the simulator reads these files it supplies one rule of its own: deceleration equals acceleration.
- **Each `$comment` names its source.** Where sources disagree, the comment says which one governs and where the other differs.
- **Composed handoffs.** The simulator builds a DFW6 HTM's belt handoffs from the K50H's `end_effector` and `lift` rows:
  - a belt drop is the drop plus the lower from max to min
  - a belt pick is the pick plus the lift from min to cantilever height 1

  That is our reading of the directive, not the directive's own statement (docs/walmart-facts.md section 9.12).

## What these files do not carry

- **Unread sheet figures.** Figures from the manufacturers' sheets that nothing reads:
  - the M5F's arc and docking speeds, operator and tilt times, and its congestion and charging tables
  - the A5 tool's per-depth pick times and its cycle breakdown

  Each file cites its sheet.
- **An unsourced response time.** The 0.5 s per movement step that the simulator's old desvis estimate added, labelled "from A5 spec". No source for it is in hand.
- **Carrier capacity.** The matrix gives back-tray counts: 3 for HAI's A42 and 4 to 6 for the C56 family. The simulator carries one carrier at a time whatever the robot holds.
- **How a robot finds its way.** The simulator moves every HTM cell to cell on its floor graph, and no source in hand says how these robots navigate.
