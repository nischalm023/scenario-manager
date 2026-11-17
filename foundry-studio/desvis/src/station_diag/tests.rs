//! Cursor-query tests for [`StationDiagStore`]: what the inspector asks it
//! at a given instant, and what it must answer. A descendant module, so the
//! tests build `StationDiagRecord` slices by hand and read the store's
//! private record vecs — no observer run in the loop.

use desgraph::StationId;
use dessim::events::{
    DispatchRejectionReason, InventoryVisitPhase, OrderStationPhase, PlanLegKind, PresentationStream, VtmIdleReasonKind,
};
use dessim::model::{SimDuration, SimTime, StationArrivalSource};
use dessim::observer::activity::ActivityKind;
use dessim::observer::station_diag::{
    HtmArrivalRecord, HtmAssignmentRecord, HtmRelayPickupRecord, InventoryVisitPhaseRecord, OrderStationPhaseRecord,
    PlanLegRecord, PresentationReadinessRecord, RejectionAggregate, RobotActivityRecord, StationCycleRecord,
    StationDiagRecord, VtmIdleReasonRecord,
};

use super::{InventoryVisitCounts, RECENT_EVENTS_WINDOW, REJECTION_WINDOW, StationDiagStore, StationDiagnosis};

fn assignment(robot_id: u32, station: u32, dispatch_ms: u64, quoted_ms: u64) -> HtmAssignmentRecord {
    HtmAssignmentRecord {
        robot_id,
        station_id: StationId(station),
        tote_id: Some(7),
        source: StationArrivalSource::PickFromRack,
        relay_cell: None,
        dispatch_time: SimTime::ms(dispatch_ms),
        quoted_arrival: SimTime::ms(quoted_ms),
    }
}

fn arrival(robot_id: u32, station: u32, arrival_ms: u64) -> HtmArrivalRecord {
    HtmArrivalRecord { robot_id, station_id: StationId(station), arrival: SimTime::ms(arrival_ms) }
}

fn rejection(station: u32, reason: DispatchRejectionReason, count: u64, last_ms: u64) -> RejectionAggregate {
    RejectionAggregate {
        station_id: StationId(station),
        reason,
        count,
        last_at: SimTime::ms(last_ms),
        last_detail: None,
    }
}

/// One store exercising both query helpers across the cursor timeline:
/// assignment→arrival matching per robot, and the trailing rejection window.
#[test]
fn inbound_and_rejection_window_queries() {
    let slice_a = StationDiagRecord {
        assignments: vec![assignment(1, 10, 1_000, 9_000), assignment(2, 10, 2_000, 12_000)],
        arrivals: vec![arrival(1, 10, 9_500)],
        rejections: vec![rejection(10, DispatchRejectionReason::ConwipCapSaturated, 3, 5_000)],
        ..Default::default()
    };
    let slice_b = StationDiagRecord {
        // Robot 1 re-assigned to the same station after its first arrival.
        assignments: vec![assignment(1, 10, 20_000, 30_000), assignment(3, 11, 21_000, 25_000)],
        rejections: vec![
            rejection(10, DispatchRejectionReason::ConwipCapSaturated, 2, 22_000),
            rejection(10, DispatchRejectionReason::NoProposal, 1, 23_000),
            rejection(11, DispatchRejectionReason::RelayHeadroomExhausted, 4, 23_500),
        ],
        ..Default::default()
    };
    let store = StationDiagStore::build([&slice_a, &slice_b]);
    let station = StationId(10);

    // Before robot 1 arrives: both HTMs inbound, ETA-sorted.
    let inbound = store.inbound_htms_at(station, SimTime::ms(3_000));
    assert_eq!(inbound.iter().map(|h| h.robot_id).collect::<Vec<_>>(), vec![1, 2]);

    // After robot 1's arrival, only robot 2 remains inbound.
    let inbound = store.inbound_htms_at(station, SimTime::ms(10_000));
    assert_eq!(inbound.iter().map(|h| h.robot_id).collect::<Vec<_>>(), vec![2]);

    // Robot 1's second assignment reopens it; robot 3 is at another station.
    let inbound = store.inbound_htms_at(station, SimTime::ms(25_000));
    assert_eq!(inbound.iter().map(|h| h.robot_id).collect::<Vec<_>>(), vec![2, 1]);

    // Window (30s − 60s, 30s] spans the whole run here, so both ConwipCap
    // buckets merge (3 + 2) with the later occurrence kept; station 11's
    // headroom bucket must not leak in.
    let rejections = store.rejections_in_window(station, SimTime::ms(30_000), REJECTION_WINDOW);
    assert_eq!(rejections.len(), 2);
    assert_eq!(rejections[0].reason, DispatchRejectionReason::ConwipCapSaturated);
    assert_eq!(rejections[0].count, 5);
    assert_eq!(rejections[0].last_at, SimTime::ms(22_000));
    assert_eq!(rejections[1].reason, DispatchRejectionReason::NoProposal);

    // A narrow window excludes the t=5s bucket.
    let rejections = store.rejections_in_window(station, SimTime::ms(30_000), SimDuration::ms(10_000));
    assert_eq!(
        rejections.iter().map(|r| (r.reason, r.count)).collect::<Vec<_>>(),
        vec![(DispatchRejectionReason::ConwipCapSaturated, 2), (DispatchRejectionReason::NoProposal, 1),]
    );

    // Bot-side view: at t=25s robot 1 is live toward station 10; its earlier
    // (arrived) assignment does not count.
    let active = store.active_assignment_for_bot(1, SimTime::ms(25_000)).expect("robot 1 has a live assignment");
    assert_eq!(active.station_id, station);
    assert_eq!(active.dispatch_time, SimTime::ms(20_000));
    // At t=10s the first assignment has already arrived — no live assignment.
    assert!(store.active_assignment_for_bot(1, SimTime::ms(10_000)).is_none());
}

fn work(robot_id: u32, kind: ActivityKind, start_ms: u64, end_ms: u64) -> RobotActivityRecord {
    RobotActivityRecord { robot_id, kind, start: SimTime::ms(start_ms), end: SimTime::ms(end_ms) }
}

/// Phase-2 queries: what a bot is doing as of the cursor, VTM idle reason,
/// relay fork-Get window, and HTMs routed through a relay cell.
#[test]
fn bot_activity_relay_and_recent_event_queries() {
    let relay_cell = 500;
    let slice = StationDiagRecord {
        // An HTM routed through relay cell 500 toward station 10.
        assignments: vec![HtmAssignmentRecord { relay_cell: Some(relay_cell), ..assignment(1, 10, 1_000, 9_000) }],
        arrivals: vec![arrival(1, 10, 9_500)],
        activity: vec![work(1, ActivityKind::Travel, 1_000, 9_500), work(1, ActivityKind::Presentation, 9_500, 20_000)],
        vtm_idle_reasons: vec![VtmIdleReasonRecord {
            robot_id: 2,
            reason: VtmIdleReasonKind::AwaitingDemand,
            gap_ms: 3_000,
            relay_cell,
            at: SimTime::ms(4_000),
        }],
        htm_relay_pickups: vec![HtmRelayPickupRecord { robot_id: 1, relay_cell, wait_ms: 250, at: SimTime::ms(8_000) }],
        ..Default::default()
    };
    let store = StationDiagStore::build([&slice]);

    // What the bot is doing as of the cursor: the interval covering it, else
    // the gap it is idle in.
    assert_eq!(store.activity_at(1, SimTime::ms(5_000)), Some((ActivityKind::Travel, SimTime::ms(1_000))));
    assert_eq!(store.activity_at(1, SimTime::ms(9_500)), Some((ActivityKind::Presentation, SimTime::ms(9_500))));
    assert_eq!(
        store.activity_at(1, SimTime::ms(500)),
        Some((ActivityKind::Gap, SimTime::ZERO)),
        "idle before its first work"
    );
    assert_eq!(
        store.activity_at(1, SimTime::ms(25_000)),
        Some((ActivityKind::Gap, SimTime::ms(20_000))),
        "idle since its last"
    );

    // VTM idle reason: none before it, the classification after.
    assert!(store.latest_vtm_idle_reason(2, SimTime::ms(3_000)).is_none());
    let (reason, gap, _) = store.latest_vtm_idle_reason(2, SimTime::ms(5_000)).expect("idle reason recorded");
    assert_eq!(reason, VtmIdleReasonKind::AwaitingDemand);
    assert_eq!(gap, SimDuration::ms(3_000));

    // Relay pickup-wait window: one wait totaling 250ms within the window.
    let (count, total) = store.htm_relay_pickups_in_window(relay_cell, SimTime::ms(10_000), SimDuration::ms(60_000));
    assert_eq!((count, total), (1, SimDuration::ms(250)));

    // HTMs via relay: open before arrival, closed after.
    assert_eq!(store.inbound_htms_via_relay(relay_cell, SimTime::ms(5_000)).len(), 1);
    assert_eq!(store.inbound_htms_via_relay(relay_cell, SimTime::ms(10_000)).len(), 0);

    // Recent events are newest-first and windowed.
    let events = store.recent_bot_events(1, SimTime::ms(10_000));
    assert!(!events.is_empty());
    assert!(events.windows(2).all(|w| w[0].at >= w[1].at), "recent events must be newest-first");
    assert!(events.iter().all(|e| e.at.as_ms() + RECENT_EVENTS_WINDOW.as_ms() >= 10_000));
}

fn cycle(station: u32, start_ms: u64, dwell_ms: u64) -> StationCycleRecord {
    StationCycleRecord {
        station_id: StationId(station),
        start: SimTime::ms(start_ms),
        dwell: SimDuration::ms(dwell_ms),
    }
}

/// Idle-diagnosis precedence: Serving > WaitingInbound > Rejected > Unrecorded,
/// including the overdue-inbound ("HTM not in time") case and the
/// empty-window "cause unrecorded" fallback.
#[test]
fn station_idle_diagnosis_precedence() {
    let station = StationId(10);
    let window = SimDuration::ms(60_000);
    // One station with a cycle [10s,13s), a rejection bucket at 14s, and an
    // inbound HTM (dispatched 20s, quoted 30s).
    let slice = StationDiagRecord {
        assignments: vec![assignment(1, 10, 20_000, 30_000)],
        rejections: vec![rejection(10, DispatchRejectionReason::NoReadyOrders, 4, 14_000)],
        station_cycles: vec![cycle(10, 10_000, 3_000)],
        ..Default::default()
    };
    let store = StationDiagStore::build([&slice]);

    // Serving wins while a cycle covers the cursor.
    assert!(matches!(
        store.station_idle_diagnosis(station, SimTime::ms(11_000), window),
        StationDiagnosis::Serving { .. }
    ));

    // After the cycle and the rejection, before the inbound is dispatched, the
    // rejection dominates — Rejected (demand-starved).
    match store.station_idle_diagnosis(station, SimTime::ms(15_000), window) {
        StationDiagnosis::Rejected { reason, count, .. } => {
            assert_eq!(reason, DispatchRejectionReason::NoReadyOrders);
            assert_eq!(count, 4);
        }
        _ => panic!("expected Rejected at t=15s"),
    }

    // Once the inbound HTM is dispatched, WaitingInbound outranks the
    // rejection; before the quoted arrival it is not overdue.
    match store.station_idle_diagnosis(station, SimTime::ms(25_000), window) {
        StationDiagnosis::WaitingInbound { count, overdue, .. } => {
            assert_eq!(count, 1);
            assert!(!overdue);
        }
        _ => panic!("expected WaitingInbound at t=25s"),
    }

    // Past the quoted arrival with no arrival recorded → overdue ("not in time").
    match store.station_idle_diagnosis(station, SimTime::ms(35_000), window) {
        StationDiagnosis::WaitingInbound { overdue, next_eta, .. } => {
            assert!(overdue);
            assert_eq!(next_eta, SimDuration::ms(5_000));
        }
        _ => panic!("expected overdue WaitingInbound at t=35s"),
    }

    // A station with no signal at all → IdleUnrecorded.
    let empty = StationDiagStore::build(std::iter::empty());
    assert!(matches!(
        empty.station_idle_diagnosis(station, SimTime::ms(5_000), window),
        StationDiagnosis::IdleUnrecorded { since: None }
    ));

    // Rejection window respected: a rejection older than the window is ignored,
    // falling through to IdleUnrecorded (with last completion carried).
    let stale = StationDiagRecord {
        rejections: vec![rejection(10, DispatchRejectionReason::NoReadyOrders, 2, 1_000)],
        station_cycles: vec![cycle(10, 2_000, 500)],
        ..Default::default()
    };
    let store = StationDiagStore::build([&stale]);
    match store.station_idle_diagnosis(station, SimTime::ms(90_000), window) {
        StationDiagnosis::IdleUnrecorded { since } => assert_eq!(since, Some(SimTime::ms(2_500))),
        _ => panic!("stale rejection should not dominate outside the window"),
    }
}

/// Timeline-strip interval extraction: a bot's activity intervals and the gaps
/// between them (with the VTM idle annotation on a gap), station serving
/// spans, and the trailing gap running to `run_end`.
#[test]
fn timeline_interval_extraction() {
    let run_end = SimTime::ms(100_000);
    let slice = StationDiagRecord {
        // Two stretches of work with a gap between them, and a gap after.
        activity: vec![work(2, ActivityKind::Travel, 0, 10_000), work(2, ActivityKind::Travel, 40_000, 60_000)],
        // VTM idle reason recorded during the gap.
        vtm_idle_reasons: vec![VtmIdleReasonRecord {
            robot_id: 2,
            reason: VtmIdleReasonKind::AwaitingDemand,
            gap_ms: 5_000,
            relay_cell: 500,
            at: SimTime::ms(10_000),
        }],
        station_cycles: vec![cycle(10, 20_000, 3_000), cycle(10, 50_000, 3_000)],
        ..Default::default()
    };
    let store = StationDiagStore::build([&slice]);

    // Two stretches of work, the gap between them, and the gap to run_end.
    let spans = store.activity_spans(2, run_end);
    assert_eq!(spans.len(), 4);
    assert_eq!(spans[0].kind, ActivityKind::Travel);
    assert_eq!((spans[0].start, spans[0].end), (SimTime::ms(0), SimTime::ms(10_000)));
    // The gap is idle, annotated with the recorded VTM idle reason.
    assert_eq!(spans[1].kind, ActivityKind::Gap);
    assert_eq!(spans[1].idle_reason, Some(VtmIdleReasonKind::AwaitingDemand));
    assert!(spans[1].covers(SimTime::ms(25_000)));
    // The trailing gap runs to run_end.
    assert_eq!(spans[3].kind, ActivityKind::Gap);
    assert_eq!(spans[3].end, run_end);

    // Serving spans: two cycles, each start..start+dwell; covers() works.
    let serving = store.station_serving_spans(StationId(10));
    assert_eq!(serving.len(), 2);
    assert_eq!((serving[0].start, serving[0].end), (SimTime::ms(20_000), SimTime::ms(23_000)));
    assert!(serving[0].covers(SimTime::ms(21_000)));
    assert!(!serving[0].covers(SimTime::ms(23_000)));
}

/// A recording made before robots' work was recorded as intervals carries
/// none, so its store says nothing of what its bots were doing: not idle
/// since zero, which every bot would otherwise read as for the whole run.
#[test]
fn a_store_without_activity_says_nothing_of_what_bots_were_doing() {
    let slice = StationDiagRecord { arrivals: vec![arrival(1, 10, 9_500)], ..Default::default() };
    let store = StationDiagStore::build([&slice]);

    assert!(!store.records_activity());
    assert_eq!(store.activity_at(1, SimTime::ms(5_000)), None, "not a gap since zero");
    assert!(store.activity_spans(1, SimTime::ms(20_000)).is_empty(), "not one long gap");
}

fn plan_leg(robot_id: u32, kind: PlanLegKind, dest: u32, enter_ms: u64) -> PlanLegRecord {
    PlanLegRecord { robot_id, kind, dest_cell: dest, enter: SimTime::ms(enter_ms) }
}

/// Plan-leg extraction: `plan_legs_from` returns a bot's legs with enter >=
/// cursor (remaining-from-frontier), filtered by robot, capped, in enter order.
#[test]
fn plan_legs_from_cursor() {
    let slice = StationDiagRecord {
        plan_legs: vec![
            plan_leg(5, PlanLegKind::RelayApproach, 100, 1_000),
            plan_leg(5, PlanLegKind::StationApproach, 200, 5_000),
            plan_leg(5, PlanLegKind::RelayDeposit, 300, 9_000),
            plan_leg(6, PlanLegKind::VtmTravel, 400, 2_000),
        ],
        ..Default::default()
    };
    let store = StationDiagStore::build([&slice]);

    // From t=0: all three of bot 5's legs, in enter order; bot 6 excluded.
    let legs = store.plan_legs_from(5, SimTime::ms(0), 8);
    assert_eq!(legs.len(), 3);
    assert_eq!(legs[0].0, PlanLegKind::RelayApproach);
    assert_eq!(legs[2].1, 300);

    // From t=6s: only the leg with enter >= 6s remains (the deposit at 9s).
    let legs = store.plan_legs_from(5, SimTime::ms(6_000), 8);
    assert_eq!(legs.iter().map(|l| l.1).collect::<Vec<_>>(), vec![300]);

    // Cap is respected.
    assert_eq!(store.plan_legs_from(5, SimTime::ms(0), 2).len(), 2);

    // Other bot's leg is isolated.
    assert_eq!(store.plan_legs_from(6, SimTime::ms(0), 8).len(), 1);
}

#[test]
fn order_counts_sort_wall_clock_arrival_and_fold_same_timestamp_handoffs() {
    let phase = |order_id, at, phase| OrderStationPhaseRecord {
        order_id,
        conveyor_id: 4,
        presentation_id: StationId(10),
        waiting_cell: Some(50),
        at: SimTime::ms(at),
        phase,
    };
    let late_slice = StationDiagRecord {
        order_phases: vec![
            phase(1, 2_000, OrderStationPhase::Admitted),
            phase(1, 3_000, OrderStationPhase::Presenting),
            phase(1, 3_000, OrderStationPhase::Waiting),
            phase(1, 5_000, OrderStationPhase::FinalPickCompleted),
            phase(2, 5_000, OrderStationPhase::Assigned),
        ],
        ..Default::default()
    };
    let early_slice =
        StationDiagRecord { order_phases: vec![phase(1, 1_000, OrderStationPhase::Assigned)], ..Default::default() };
    let store = StationDiagStore::build([&late_slice, &early_slice]);
    let counts = store.order_phase_counts(StationId(10), SimTime::ms(5_000));
    assert_eq!(counts.assigned, 1, "same-time release and refill have no sampled gap");
    assert_eq!(counts.physical_residents, 1, "logical completion does not release physical membership");
    let at_presentation = store.order_phase_counts(StationId(10), SimTime::ms(3_000));
    assert_eq!(at_presentation.presenting, 1, "lifecycle precedence wins over equal-time emission order");

    let (assigned, residents) = store.order_count_series(StationId(10), SimTime::ms(5000));
    assert_eq!(assigned.last(), Some(&[5_000.0 / 60_000.0, 1.0]));
    assert_eq!(residents.last(), Some(&[5_000.0 / 60_000.0, 1.0]));
    assert_eq!(assigned.iter().filter(|point| point[0] == 5_000.0 / 60_000.0).count(), 1, "no same-time dip");
    let (early_assigned, early_residents) = store.order_count_series(StationId(10), SimTime::ms(1500));
    assert_eq!(early_assigned.last(), Some(&[1500.0 / 60000.0, 1.0]));
    assert_eq!(early_residents.last(), Some(&[1500.0 / 60000.0, 0.0]));
    assert!(early_assigned.iter().chain(&early_residents).all(|point| point[0] <= 1500.0 / 60000.0));
}

#[test]
fn readiness_pairs_only_the_same_order_visit() {
    let ready = |pick_index, stream, ready_at| PresentationReadinessRecord {
        order_id: 7,
        pick_index,
        conveyor_id: 4,
        presentation_id: StationId(10),
        stream,
        ready_at: SimTime::ms(ready_at),
    };
    let unmatched = StationDiagRecord {
        presentation_readiness: vec![
            ready(1, PresentationStream::Order, 1_000),
            ready(2, PresentationStream::Inventory, 2_000),
        ],
        ..Default::default()
    };
    assert_eq!(StationDiagStore::build([&unmatched]).latest_readiness_pair(StationId(10), SimTime::ms(3_000)), None);

    let matched = StationDiagRecord {
        presentation_readiness: vec![ready(1, PresentationStream::Inventory, 2_500)],
        ..Default::default()
    };
    assert_eq!(
        StationDiagStore::build([&unmatched, &matched]).latest_readiness_pair(StationId(10), SimTime::ms(3_000)),
        Some((SimTime::ms(1_000), SimTime::ms(2_500), 7, 1))
    );
}

#[test]
fn inventory_visit_counts_are_shared_and_same_time_transitions_are_folded() {
    let visit = |order_id, pick_index, at, phase| InventoryVisitPhaseRecord {
        conveyor_id: 4,
        order_id,
        pick_index,
        phase,
        at: SimTime::ms(at),
    };
    let later_slice = StationDiagRecord {
        inventory_visit_phases: vec![
            visit(1, 0, 3_000, InventoryVisitPhase::Boarded),
            visit(1, 0, 5_000, InventoryVisitPhase::PickCompleted),
            visit(2, 0, 5_000, InventoryVisitPhase::Released),
        ],
        ..Default::default()
    };
    let earlier_slice = StationDiagRecord {
        inventory_visit_phases: vec![
            visit(1, 0, 1_000, InventoryVisitPhase::Released),
            visit(1, 0, 2_000, InventoryVisitPhase::Loaded),
        ],
        ..Default::default()
    };
    let store = StationDiagStore::build([&later_slice, &earlier_slice]);
    assert_eq!(
        store.inventory_visit_counts(4, SimTime::ms(2_000)),
        InventoryVisitCounts { conwip: 1, upstream: 1, loaded_transit: 1 }
    );
    assert_eq!(
        store.inventory_visit_counts(4, SimTime::ms(5_000)),
        InventoryVisitCounts { conwip: 1, upstream: 1, loaded_transit: 0 }
    );
    let (conwip, _, _) = store.inventory_visit_series(4, SimTime::ms(5000));
    assert!(
        conwip.iter().filter(|point| point[0] == 5_000.0 / 60_000.0).all(|point| point[1] == 1.0),
        "same-time completion and release must not plot a false dip or peak"
    );
    assert!(!StationDiagStore::default().has_inventory_visit_phases(4));
}
