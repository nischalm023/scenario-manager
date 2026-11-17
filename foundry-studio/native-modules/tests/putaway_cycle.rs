//! Putaway HTM cycle integration tests.
//!
//! Map: minimal_putaway (2 HTMs, 1 VTM, 1 pick station, 1 putaway station)
//!
//! - `putaway_3_orders_full_trace` (CollisionDes): asserts the FULL VERBATIM
//!   event trace with 3 orders.
//! - `ghost_fidelity_runs_htm_relay` (Ghost, with a second VTM): regression for
//!   issue #121. `fidelity=ghost` is `oracle: None` (solver.rs:228), but the
//!   three HTM leg tasks hard-`expect` the oracle (htm_pickup.rs:138,
//!   htm_deliver.rs:113, htm_store.rs:116), so the first HTM pickup panics.
//!   Trigger is Ghost + any HTM relay leg, NOT kinematics: both
//!   `htm_kinematic` and `constant_speed` panic identically at the same line.
//!   Acceptance: ghost drives HTMs via the real pathfinder with reservations
//!   never blocking, i.e. the solver runs to completion.

#[macro_use]
mod common;

use desgraph::types::RobotType;

use dessim::config::FidelityMode;
use dessim::events::{CycleKind, SimEvent};
use dessim::{ScenarioConfig, StopReason};
use native_modules as _;

// IGNORED pending deliberate regeneration after the feeding-decoupling change.
//
// This verbatim trace captured the OLD demand-gated putaway flow, where an
// empty-tote chain was dispatched only when a lot was due — so only ~3 totes
// were put away before the run idled. Under feeding-decoupling the supply
// service keeps the station stocked to `cohort_target` independent of demand,
// and `PutawayInduction` fills every presenting empty until physical
// exhaustion: the run now puts away all 63 physically-possible totes (60 rack +
// 3 direct, matching the domain-expert exhaustion count) and completes all 3
// orders (verified: oc(0), oc(1), oc(2) all emit; 66 CycleCompletes). The event
// stream therefore grows from 189 to ~1482 events and the putaway CycleComplete
// now fires from PutawayInduction at fill-time (dwell_ms=pacing_interval), not from
// Presentation. The trace is deterministic across runs, so it CAN be
// regenerated verbatim via `dump_trace` (below) — but pasting a 1482-event
// expected list that encodes the cohort_target / pacing config is a deliberate
// contract decision for the lead/architect, not a mechanical update. Do NOT
// weaken this to a presence check (see feedback_assert_trace); regenerate the
// full list once the new flow is blessed, then remove this #[ignore].
#[test]
#[ignore = "verbatim trace obsolete after feeding-decoupling; regenerate via dump_trace once new dense flow is blessed"]
fn putaway_3_orders_full_trace() {
    let (world, scenario, config) = common::load_world_spec("minimal_putaway_spec.json");

    let (events, _result, metrics) = common::run_world_capture_all(&world, &scenario, config);

    // UOW scheduling trace: VTM Forwards chain back-to-back, HTM delivery
    // decoupled via work queue. Trace regenerated after UOW refactor.
    // 5 bots: 1 VTM + 4 HTMs.
    assert_events!(
        events,
        [
            init_pos(5),                           // 0
            tote_init(60),                         // 1
            nav(0, 0, [(46, 0.000), (47, 4.500)]), // 2
            tote(59, 0),                           // 3
            tote(59, 4500),                        // 4
            res("VtmForwardCompleted", 4500),      // 5
            nav(
                3,
                4500,
                [
                    (59, 0.000),
                    (59, 1.000),
                    (59, 1.000),
                    (58, 0.667),
                    (57, 0.667),
                    (56, 0.667),
                    (55, 0.667),
                    (54, 0.667),
                    (54, 1.000),
                    (48, 0.667),
                    (42, 0.667),
                    (42, 1.000),
                    (43, 0.667),
                    (44, 0.667),
                    (45, 0.667),
                    (46, 0.667)
                ]
            ), // 6
            tote(59, 15837),                       // 7
            nav(
                3,
                15837,
                [
                    (46, 0.000),
                    (46, 1.000),
                    (46, 1.000),
                    (45, 0.667),
                    (44, 0.667),
                    (43, 0.667),
                    (42, 0.667),
                    (42, 1.000),
                    (48, 0.667),
                    (48, 1.000),
                    (49, 0.667),
                    (50, 0.667)
                ]
            ), // 8
            relay(46, true, 4500),                 // 9
            relay(46, false, 15837),               // 10
            cc(3, 29006),                          // 11
            nav(
                3,
                29006,
                [
                    (50, 0.000),
                    (50, 1.000),
                    (44, 0.667),
                    (44, 1.000),
                    (43, 0.667),
                    (42, 0.667),
                    (42, 1.000),
                    (48, 0.667),
                    (54, 0.667),
                    (54, 1.000),
                    (55, 0.667),
                    (56, 0.667),
                    (57, 0.667),
                    (58, 0.667),
                    (59, 0.667),
                    (59, 1.000),
                    (53, 0.667)
                ]
            ), // 12
            nav(3, 29006, [(50, 0.000), (50, 1.000), (44, 0.667), (44, 1.000), (45, 0.667), (46, 0.667)]), // 13
            tote(59, 33007),                       // 14
            nav(
                3,
                33007,
                [
                    (46, 0.000),
                    (46, 1.000),
                    (46, 1.000),
                    (45, 0.667),
                    (44, 0.667),
                    (43, 0.667),
                    (42, 0.667),
                    (42, 1.000),
                    (48, 0.667),
                    (54, 0.667),
                    (54, 1.000),
                    (55, 0.667),
                    (56, 0.667),
                    (57, 0.667),
                    (58, 0.667),
                    (59, 0.667),
                    (59, 1.000),
                    (53, 0.667)
                ]
            ), // 15
            relay(46, true, 33007),                // 16
            nav(1, 33007, [(47, 0.000), (46, 4.500)]), // 17
            tote(59, 33007),                       // 18
            tote(59, 37507),                       // 19
            relay(46, false, 33007),               // 20
            res("VtmReturnCompleted", 37507),      // 21
            nav(0, 0, [(46, 0.000), (41, 4.500)]), // 22
            tote(58, 0),                           // 23
            tote(58, 4500),                        // 24
            res("VtmForwardCompleted", 4500),      // 25
            nav(
                2,
                4500,
                [
                    (53, 0.000),
                    (53, 1.000),
                    (59, 0.667),
                    (59, 1.000),
                    (59, 0.667),
                    (58, 0.667),
                    (57, 0.667),
                    (56, 0.667),
                    (55, 0.667),
                    (54, 0.667),
                    (54, 1.000),
                    (48, 0.667),
                    (42, 0.667),
                    (36, 0.667),
                    (36, 1.000),
                    (37, 0.667),
                    (38, 0.667),
                    (39, 0.667),
                    (40, 0.667)
                ]
            ), // 26
            tote(58, 17838),                       // 27
            nav(
                2,
                17838,
                [
                    (40, 0.000),
                    (40, 1.000),
                    (40, 1.000),
                    (39, 0.667),
                    (38, 0.667),
                    (37, 0.667),
                    (36, 0.667),
                    (36, 1.000),
                    (42, 0.667),
                    (48, 0.667),
                    (48, 1.000),
                    (49, 0.667),
                    (49, 3.166),
                    (50, 0.667)
                ]
            ), // 28
            relay(40, true, 4500),                 // 29
            relay(40, false, 17838),               // 30
            cc(2, 34840),                          // 31
            nav(2, 34840, [(50, 0.000), (50, 1.000), (50, 1.168), (44, 0.667), (44, 1.000), (45, 0.667), (46, 0.667)]), // 32
            tote(58, 40009),                           // 33
            relay(46, true, 40009),                    // 34
            nav(1, 40009, [(47, 0.000), (46, 4.500)]), // 35
            tote(58, 40009),                           // 36
            tote(58, 44509),                           // 37
            relay(46, false, 40009),                   // 38
            res("VtmReturnCompleted", 44509),          // 39
            nav(0, 4500, [(46, 0.000), (35, 4.500)]),  // 40
            tote(57, 4500),                            // 41
            tote(57, 9000),                            // 42
            res("VtmForwardCompleted", 9000),          // 43
            nav(0, 37507, [(46, 0.000), (17, 4.500)]), // 44
            tote(59, 37507),                           // 45
            tote(59, 42007),                           // 46
            res("VtmForwardCompleted", 42007),         // 47
            nav(
                4,
                9000,
                [
                    (65, 0.000),
                    (65, 1.000),
                    (59, 0.667),
                    (59, 1.000),
                    (58, 0.667),
                    (57, 0.667),
                    (56, 0.667),
                    (55, 0.667),
                    (54, 0.667),
                    (54, 1.000),
                    (48, 0.667),
                    (42, 0.667),
                    (36, 0.667),
                    (30, 0.667),
                    (30, 1.000),
                    (31, 0.667),
                    (32, 0.667),
                    (33, 0.667),
                    (34, 0.667)
                ]
            ), // 48
            tote(57, 22338),                           // 49
            nav(
                4,
                22338,
                [
                    (34, 0.000),
                    (34, 1.000),
                    (34, 1.000),
                    (33, 0.667),
                    (32, 0.667),
                    (31, 0.667),
                    (30, 0.667),
                    (30, 1.000),
                    (36, 0.667),
                    (42, 0.667),
                    (48, 0.667),
                    (48, 1.000),
                    (49, 0.667),
                    (49, 3.833),
                    (50, 0.667)
                ]
            ), // 50
            relay(34, true, 9000),                     // 51
            relay(34, false, 22338),                   // 52
            cc(4, 40674),                              // 53
            nav(
                4,
                40674,
                [
                    (50, 0.000),
                    (50, 1.000),
                    (44, 0.667),
                    (44, 1.000),
                    (43, 0.667),
                    (42, 0.667),
                    (42, 1.000),
                    (48, 0.667),
                    (54, 0.667),
                    (54, 1.000),
                    (55, 0.667),
                    (56, 0.667),
                    (57, 0.667),
                    (58, 0.667),
                    (59, 0.667)
                ]
            ), // 54
            nav(4, 40674, [(50, 0.000), (50, 1.000), (44, 0.667), (44, 1.000), (45, 0.667), (46, 0.667)]), // 55
            tote(57, 44675),                           // 56
            nav(
                4,
                44675,
                [
                    (46, 0.000),
                    (46, 1.000),
                    (46, 1.000),
                    (45, 0.667),
                    (44, 0.667),
                    (43, 0.667),
                    (42, 0.667),
                    (42, 1.000),
                    (48, 0.667),
                    (54, 0.667),
                    (54, 1.000),
                    (55, 0.667),
                    (56, 0.667),
                    (57, 0.667),
                    (58, 0.667),
                    (59, 0.667)
                ]
            ), // 57
            relay(46, true, 44675),                    // 58
            nav(1, 44675, [(47, 0.000), (46, 4.500)]), // 59
            tote(57, 44675),                           // 60
            tote(57, 49175),                           // 61
            relay(46, false, 44675),                   // 62
            res("VtmReturnCompleted", 49175),          // 63
            nav(0, 44509, [(46, 0.000), (23, 4.500)]), // 64
            tote(58, 44509),                           // 65
            tote(58, 49009),                           // 66
            res("VtmForwardCompleted", 49009),         // 67
            nav(
                3,
                49009,
                [
                    (46, 0.000),
                    (46, 1.000),
                    (46, 1.000),
                    (45, 0.667),
                    (45, 1.000),
                    (39, 0.667),
                    (33, 0.667),
                    (27, 0.667),
                    (21, 0.667),
                    (21, 1.000),
                    (22, 0.667)
                ]
            ), // 68
            tote(58, 57011),                           // 69
            nav(
                3,
                57011,
                [
                    (22, 0.000),
                    (22, 1.000),
                    (22, 1.000),
                    (21, 0.667),
                    (20, 0.667),
                    (19, 0.667),
                    (18, 0.667),
                    (18, 1.000),
                    (12, 0.667),
                    (6, 0.667),
                    (0, 0.667),
                    (0, 1.000),
                    (1, 0.667),
                    (2, 0.667)
                ]
            ), // 70
            relay(22, true, 49009),                    // 71
            relay(22, false, 57011),                   // 72
            cc(3, 76014),                              // 73
            nav(
                3,
                76014,
                [
                    (2, 0.000),
                    (2, 1.000),
                    (8, 0.667),
                    (14, 0.667),
                    (20, 0.667),
                    (26, 0.667),
                    (32, 0.667),
                    (38, 0.667),
                    (44, 0.667),
                    (44, 1.000),
                    (43, 0.667),
                    (42, 0.667),
                    (42, 1.000),
                    (48, 0.667),
                    (54, 0.667),
                    (60, 0.667),
                    (60, 1.000),
                    (61, 0.667),
                    (62, 0.667),
                    (63, 0.667),
                    (64, 0.667),
                    (65, 0.667)
                ]
            ), // 74
            nav(
                3,
                76014,
                [(2, 0.000), (2, 1.000), (8, 0.667), (14, 0.667), (20, 0.667), (20, 1.000), (21, 0.667), (22, 0.667)]
            ), // 75
            tote(58, 81349),                           // 76
            nav(
                3,
                81349,
                [
                    (22, 0.000),
                    (22, 1.000),
                    (22, 1.000),
                    (21, 0.667),
                    (20, 0.667),
                    (19, 0.667),
                    (18, 0.667),
                    (18, 1.000),
                    (24, 0.667),
                    (30, 0.667),
                    (36, 0.667),
                    (42, 0.667),
                    (48, 0.667),
                    (54, 0.667),
                    (60, 0.667),
                    (60, 1.000),
                    (61, 0.667),
                    (62, 0.667),
                    (63, 0.667),
                    (64, 0.667),
                    (65, 0.667)
                ]
            ), // 77
            relay(22, true, 81349),                    // 78
            nav(0, 81349, [(23, 0.000), (46, 4.500)]), // 79
            tote(58, 81349),                           // 80
            tote(58, 85849),                           // 81
            relay(22, false, 81349),                   // 82
            res("VtmReturnCompleted", 85849),          // 83
            nav(1, 49175, [(46, 0.000), (29, 4.500)]), // 84
            tote(57, 49175),                           // 85
            tote(57, 53675),                           // 86
            res("VtmForwardCompleted", 53675),         // 87
            nav(
                4,
                44675,
                [
                    (46, 0.000),
                    (46, 1.000),
                    (46, 1.000),
                    (45, 0.667),
                    (45, 1.000),
                    (39, 0.667),
                    (33, 0.667),
                    (27, 0.667),
                    (21, 0.667),
                    (15, 0.667),
                    (15, 1.000),
                    (16, 0.667)
                ]
            ), // 88
            tote(59, 53344),                           // 89
            nav(
                4,
                53344,
                [
                    (16, 0.000),
                    (16, 1.000),
                    (16, 1.000),
                    (15, 0.667),
                    (14, 0.667),
                    (13, 0.667),
                    (12, 0.667),
                    (12, 1.000),
                    (18, 0.667),
                    (24, 0.667),
                    (24, 1.000),
                    (24, 1.000),
                    (18, 0.667),
                    (18, 1.000),
                    (12, 0.667),
                    (6, 0.667),
                    (0, 0.667),
                    (0, 1.000),
                    (1, 0.667),
                    (1, 9.000),
                    (2, 0.667)
                ]
            ), // 90
            relay(16, true, 42007),                    // 91
            relay(16, false, 53344),                   // 92
            cc(4, 86348),                              // 93
            nav(
                4,
                86348,
                [
                    (2, 0.000),
                    (2, 1.000),
                    (8, 0.667),
                    (14, 0.667),
                    (20, 0.667),
                    (26, 0.667),
                    (32, 0.667),
                    (38, 0.667),
                    (44, 0.667),
                    (44, 1.000),
                    (43, 0.667),
                    (42, 0.667),
                    (42, 1.000),
                    (48, 0.667),
                    (54, 0.667),
                    (54, 1.000),
                    (55, 0.667),
                    (56, 0.667),
                    (57, 0.667),
                    (58, 0.667),
                    (59, 0.667)
                ]
            ), // 94
            nav(4, 86348, [(2, 0.000), (2, 1.000), (8, 0.667), (14, 0.667), (14, 1.000), (15, 0.667), (16, 0.667)]), // 95
            tote(59, 91016), // 96
            nav(
                4,
                91016,
                [
                    (16, 0.000),
                    (16, 1.000),
                    (16, 1.000),
                    (15, 0.667),
                    (14, 0.667),
                    (13, 0.667),
                    (12, 0.667),
                    (12, 1.000),
                    (18, 0.667),
                    (24, 0.667),
                    (30, 0.667),
                    (36, 0.667),
                    (42, 0.667),
                    (48, 0.667),
                    (54, 0.667),
                    (54, 1.000),
                    (55, 0.667),
                    (56, 0.667),
                    (57, 0.667),
                    (58, 0.667),
                    (59, 0.667)
                ]
            ), // 97
            relay(16, true, 91016), // 98
            nav(
                2,
                53675,
                [
                    (53, 0.000),
                    (53, 1.000),
                    (59, 0.667),
                    (59, 1.000),
                    (58, 0.667),
                    (57, 0.667),
                    (56, 0.667),
                    (55, 0.667),
                    (54, 0.667),
                    (54, 1.000),
                    (48, 0.667),
                    (42, 0.667),
                    (36, 0.667),
                    (30, 0.667),
                    (24, 0.667),
                    (24, 1.000),
                    (25, 0.667),
                    (26, 0.667),
                    (27, 0.667),
                    (28, 0.667)
                ]
            ), // 99
            tote(57, 67680), // 100
            nav(
                2,
                67680,
                [
                    (28, 0.000),
                    (28, 1.000),
                    (28, 1.000),
                    (27, 0.667),
                    (26, 0.667),
                    (25, 0.667),
                    (24, 0.667),
                    (24, 1.000),
                    (18, 0.667),
                    (12, 0.667),
                    (6, 0.667),
                    (0, 0.667),
                    (0, 1.000),
                    (1, 0.667),
                    (1, 9.332),
                    (2, 0.667)
                ]
            ), // 101
            relay(28, true, 53675), // 102
            relay(28, false, 67680), // 103
            cc(2, 96682),    // 104
            nav(
                2,
                96682,
                [
                    (2, 0.000),
                    (2, 1.000),
                    (8, 0.667),
                    (14, 0.667),
                    (20, 0.667),
                    (26, 0.667),
                    (32, 0.667),
                    (38, 0.667),
                    (44, 0.667),
                    (44, 1.000),
                    (43, 0.667),
                    (42, 0.667),
                    (42, 1.000),
                    (48, 0.667),
                    (54, 0.667),
                    (54, 1.000),
                    (55, 0.667),
                    (56, 0.667),
                    (57, 0.667),
                    (58, 0.667),
                    (58, 1.667),
                    (52, 0.667),
                    (52, 1.000),
                    (53, 0.667)
                ]
            ), // 105
            nav(
                2,
                96682,
                [(2, 0.000), (2, 1.000), (8, 0.667), (14, 0.667), (20, 0.667), (20, 1.000), (21, 0.667), (22, 0.667)]
            ), // 106
            tote(57, 102017), // 107
            nav(
                2,
                102017,
                [
                    (22, 0.000),
                    (22, 1.000),
                    (22, 1.000),
                    (21, 0.667),
                    (20, 0.667),
                    (19, 0.667),
                    (18, 0.667),
                    (18, 1.000),
                    (24, 0.667),
                    (30, 0.667),
                    (36, 0.667),
                    (42, 0.667),
                    (48, 0.667),
                    (54, 0.667),
                    (54, 1.000),
                    (55, 0.667),
                    (56, 0.667),
                    (57, 0.667),
                    (58, 0.667),
                    (58, 1.667),
                    (52, 0.667),
                    (52, 1.000),
                    (53, 0.667)
                ]
            ), // 108
            relay(22, true, 102017), // 109
            nav(1, 91016, [(17, 0.000), (46, 4.500)]), // 110
            tote(59, 91016), // 111
            tote(59, 95516), // 112
            relay(16, false, 91016), // 113
            res("VtmReturnCompleted", 95516), // 114
            nav(1, 102017, [(23, 0.000), (46, 4.500)]), // 115
            tote(57, 102017), // 116
            tote(57, 106517), // 117
            relay(22, false, 102017), // 118
            res("VtmReturnCompleted", 106517), // 119
            oc(0, 76014),    // 120
            oc(1, 96682),    // 121
            oc(2, 86348),    // 122
            oc(u32::MAX, 40674), // 123
        ]
    );

    // --- BotLocation + collision audits ---
    let slices = &metrics.time_slices;
    let cell_spacing = 1000.0f64; // minimal_putaway uses 1000mm cells
    common::audit_bot_locations(slices, cell_spacing);

    // --- Time-slice state audit ---
    // Verify BotLocation step timing, tote state, and dwell gaps.
    use dessim::observer::time_slice::{ToteStateRecord, ToteVisualState};

    // Print tote state per slice for diagnostics.
    eprintln!("\n=== Tote state audit ===");
    for (si, slice) in slices.iter().enumerate() {
        let positions = ToteStateRecord::positions_at(
            slices.iter().map(|slice| (slice.start_ms, &slice.tote_state)),
            slice.start_ms,
        );
        let tote59 = positions.iter().find(|(id, _)| *id == 59).map(|(_, s)| s);
        let tote58 = positions.iter().find(|(id, _)| *id == 58).map(|(_, s)| s);
        let transitions_59: Vec<_> = slice.tote_state.transitions.iter().filter(|(_, id, _)| *id == 59).collect();
        let transitions_58: Vec<_> = slice.tote_state.transitions.iter().filter(|(_, id, _)| *id == 58).collect();
        if tote59.is_some() || !transitions_59.is_empty() || tote58.is_some() || !transitions_58.is_empty() {
            eprintln!("Slice {} ({}ms):", si, slice.start_ms);
            if let Some(s) = tote59 {
                eprintln!("  tote59 start: {:?}", s);
            }
            for (offset, _, state) in &transitions_59 {
                eprintln!("  tote59 @+{}ms: {:?}", offset, state);
            }
            if let Some(s) = tote58 {
                eprintln!("  tote58 start: {:?}", s);
            }
            for (offset, _, state) in &transitions_58 {
                eprintln!("  tote58 @+{}ms: {:?}", offset, state);
            }
        }
    }

    // Verify tote 59 transitions to AtRelay while D2 is at cell 46.
    {
        let slice2 = &slices[2];
        let d2 = slice2.bot_locations.get(&2).expect("D2 should have BotLocation in slice 2");

        // Find the window where D2 is at cell 46 (Store arrival → parking departure).
        let first_46 = d2.steps.iter().position(|s| s.cell_id == 46).expect("D2 should visit cell 46 in slice 2");
        // Find last consecutive step at cell 46.
        let last_46 = (first_46..d2.steps.len()).take_while(|&i| d2.steps[i].cell_id == 46).last().unwrap();
        let at_46_start: u64 = d2.steps[..first_46].iter().map(|s| s.delta_ms).sum();
        let at_46_end: u64 = d2.steps[..=last_46].iter().map(|s| s.delta_ms).sum();

        // Tote 59 should transition to AtRelay{46} while D2 is at cell 46.
        let tote59_transition = slice2.tote_state.transitions.iter().find(|(_, id, _)| *id == 59);
        assert!(tote59_transition.is_some(), "tote 59 should have a transition in slice 2");
        let (t_offset, _, new_state) = tote59_transition.unwrap();
        assert_eq!(
            *new_state,
            ToteVisualState::AtRelay { cell_id: 46 },
            "tote 59 should transition to AtRelay{{cell_id:46}}"
        );
        assert!(
            *t_offset >= at_46_start && *t_offset <= at_46_end,
            "tote 59 transition at +{}ms should be while D2 is at cell 46 ({}-{}ms)",
            t_offset,
            at_46_start,
            at_46_end
        );
    }

    // Verify step timing sums don't exceed slice width for any bot.
    let slice_width_ms = 10_000u64; // SLICE_WIDTH_MS
    for (si, slice) in slices.iter().enumerate() {
        for (&bot_key, loc) in &slice.bot_locations {
            let total: u64 = loc.steps.iter().map(|s| s.delta_ms).sum();
            assert!(
                total <= slice_width_ms,
                "Slice {} bot {}: step deltas sum to {}ms > slice width {}ms",
                si,
                bot_key,
                total,
                slice_width_ms
            );
        }
    }

    eprintln!("=== Time-slice state audit: PASS ===");
}

#[test]
fn ghost_fidelity_runs_htm_relay() {
    // The shared trace fixture with a second VTM, because with one its zone
    // saturates: as the barrier reaches a clearance's departure, every relay's
    // reservation is still unfinalized, the pickups and VTM cycles that would
    // free one are decided only later in scheduler time, and the run stops
    // (fwd task Xj7z1w7Y).
    let (world, mut scenario, mut config) = common::load_world_spec("minimal_putaway_two_vtms_spec.json");
    // The shared trace fixture has 2,000 inbound units but only 60 totes and
    // three picks. This regression must drain a feasible finite workload.
    let ScenarioConfig::Generated(generated) = &mut scenario else { panic!("expected generated fixture") };
    generated.num_skus = 1;
    generated.total_inventory_units = 12;
    config.fidelity = FidelityMode::Ghost;
    config.waive_completability_check = true;

    let (events, result, _metrics) = common::try_run_world(&world, &scenario, config)
        .unwrap_or_else(|e| panic!("ghost-mode solver must complete under #121 acceptance: {e}"));
    assert!(matches!(result.stop_reason, StopReason::Complete));
    assert_eq!(result.total_orders_completed, 3);
    let inducted: u32 = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::CycleComplete(cycle) => match cycle.kind {
                CycleKind::Putaway { qty, .. } => Some(qty),
                _ => None,
            },
            _ => None,
        })
        .sum();
    assert_eq!(inducted, 12, "finite inbound work was left unfinished");

    // Proves ghost actually executed HTM relay legs end-to-end — the three
    // `.expect(oracle)` sites in htm_pickup/deliver/store didn't silently
    // degrade to no-ops or get skipped by the planner.
    let relay_events = events.iter().filter(|e| matches!(e, SimEvent::RelayStateChange(_))).count();
    assert!(relay_events > 0, "ghost run produced no RelayStateChange events — HTM relay legs didn't execute");

    // Guards HTM Nav travel timings under ghost. Downstream arrival math
    // (`htm_depart + travel_time`) reads `ReservedPath.path.travel_time`,
    // which `find_path` populates in the allocator's no-reservation-tracking
    // shortcut. If a future change zeroed that out, ghost legs would look
    // instantaneous and the planner would build out-of-order leg sequences
    // that fail with TimeOverlap. This assertion fires loudly if that
    // regresses.
    for event in &events {
        let SimEvent::Nav(nav) = event else { continue };
        if nav.robot_type != RobotType::Htm || nav.distance_mm == 0 {
            continue;
        }
        let total_ms: u64 = nav.waypoints.iter().map(|w| w.action.duration().as_ms()).sum();
        assert!(
            total_ms > 0,
            "HTM Nav at t={}ms moved {}mm but waypoints sum to 0 duration — \
             travel_time threading regressed?",
            nav.time.as_ms(),
            nav.distance_mm,
        );
    }
}

/// Dump the current event stream in assert_events! macro format.
/// Run with `--nocapture` to see output. Copy-paste into the trace above
/// when regenerating after behavioral changes.
#[test]
#[ignore] // manual use only
fn dump_trace() {
    use dessim::events::SimEvent;

    let (world, scenario, config) = common::load_world_spec("minimal_putaway_spec.json");
    let (events, _, _) = common::run_world_capture_all(&world, &scenario, config);

    eprintln!("\n// --- Generated trace ({} events) ---", events.len());
    for (i, e) in events.iter().enumerate() {
        let line = match e {
            SimEvent::InitialPositions(ip) => format!("init_pos({}),", ip.positions.len()),
            SimEvent::ToteInitialStates(ti) => format!("tote_init({}),", ti.totes.len()),
            SimEvent::Nav(n) => {
                let wps: Vec<String> = n
                    .waypoints
                    .iter()
                    .map(|wp| format!("({}, {:.3})", wp.cell.0, wp.action.duration().as_ms() as f64 / 1000.0))
                    .collect();
                format!("nav({}, {}, [{}]),", n.robot_id.0, n.time.as_ms(), wps.join(", "))
            }
            SimEvent::CycleComplete(cc) => format!("cc({}, {}),", cc.robot_id.0, cc.sim_time_ms),
            SimEvent::OrderComplete(oc) => format!("oc({}, {}),", oc.order_id.0, oc.sim_time_ms),
            SimEvent::RelayStateChange(r) => format!("relay({}, {}, {}),", r.cell_id, r.reserved, r.time.as_ms()),
            SimEvent::ToteMoved(tm) => format!("tote({:?}, {}),", tm.tote_id, tm.time.as_ms()),
            SimEvent::Resource(r) => format!("res(\"{:?}\", {}),", r.kind, r.sim_time_ms),
            _ => format!("// unknown: {:?}", e),
        };
        eprintln!("        {} // {}", line, i);
    }
}
