//! Declarative event trace assertion framework.
//!
//! Each expected entry specifies: time, event pattern, and optional
//! side assertions on entity state. The runner matches actual events
//! against expected entries in order.

use dessim::events::*;
use dessim::model::ToteId;
use dessim::observer::time_slice::ToteVisualState;

// ---------------------------------------------------------------------------
// Expected event entry
// ---------------------------------------------------------------------------

pub struct ExpectedEvent {
    pub time_ms: u64,
    pub pattern: EventPattern,
    pub checks: Vec<SideCheck>,
}

/// What event we expect.
pub enum EventPattern {
    InitPositions {
        count: usize,
    },
    ToteInitial {
        count: usize,
    },
    Nav {
        robot_id: u32,
        min_waypoints: usize,
    },
    CycleComplete {
        robot_id: u32,
        station_id: u32,
        picks: u32,
    },
    OrderComplete {
        order_id: u32,
    },
    ToteMoved {
        tote_id: u32,
        state: ToteVisualState,
    },
    VtmForwardCompleted,
    VtmReturnCompleted,
    RelayReserved {
        cell_id: u32,
    },
    RelayFreed {
        cell_id: u32,
    },
    /// Match any event at this time (skip).
    Any,
}

/// Side assertion on entity state.
pub enum SideCheck {
    ToteState { tote_id: u32, state: ToteVisualState },
}

// ---------------------------------------------------------------------------
// Builder helpers
// ---------------------------------------------------------------------------

pub fn at(time_ms: u64, pattern: EventPattern) -> ExpectedEvent {
    ExpectedEvent { time_ms, pattern, checks: vec![] }
}

pub fn at_with(time_ms: u64, pattern: EventPattern, checks: Vec<SideCheck>) -> ExpectedEvent {
    ExpectedEvent { time_ms, pattern, checks }
}

// Event pattern constructors.
pub fn init_positions(count: usize) -> EventPattern {
    EventPattern::InitPositions { count }
}
pub fn tote_initial(count: usize) -> EventPattern {
    EventPattern::ToteInitial { count }
}
pub fn nav(robot_id: u32, min_wps: usize) -> EventPattern {
    EventPattern::Nav { robot_id, min_waypoints: min_wps }
}
pub fn cycle_complete(robot_id: u32, station_id: u32, picks: u32) -> EventPattern {
    EventPattern::CycleComplete { robot_id, station_id, picks }
}
pub fn order_complete(order_id: u32) -> EventPattern {
    EventPattern::OrderComplete { order_id }
}
pub fn tote_moved(tote_id: u32, state: ToteVisualState) -> EventPattern {
    EventPattern::ToteMoved { tote_id, state }
}
pub fn vtm_forward_completed() -> EventPattern {
    EventPattern::VtmForwardCompleted
}
pub fn vtm_return_completed() -> EventPattern {
    EventPattern::VtmReturnCompleted
}
pub fn relay_reserved(cell_id: u32) -> EventPattern {
    EventPattern::RelayReserved { cell_id }
}
pub fn relay_freed(cell_id: u32) -> EventPattern {
    EventPattern::RelayFreed { cell_id }
}
pub fn any() -> EventPattern {
    EventPattern::Any
}

// Side check constructors.
pub fn tote_is(tote_id: u32, state: ToteVisualState) -> SideCheck {
    SideCheck::ToteState { tote_id, state }
}

// ToteVisualState helpers.
pub fn on_bot(bot_id: u32) -> ToteVisualState {
    ToteVisualState::OnBot { bot_id }
}
pub fn at_relay(cell_id: u32) -> ToteVisualState {
    ToteVisualState::AtRelay { cell_id }
}
pub fn in_rack(cell_id: u32, level: u16) -> ToteVisualState {
    ToteVisualState::InRack { cell_id, level }
}

// ---------------------------------------------------------------------------
// Matcher
// ---------------------------------------------------------------------------

fn event_time(e: &SimEvent) -> u64 {
    match e {
        SimEvent::InitialPositions(_) | SimEvent::ToteInitialStates(_) | SimEvent::StripePlan(_) => 0,
        SimEvent::Nav(n) => n.time.as_ms(),
        SimEvent::VtmNav(v) => v.kind.start().as_ms(),
        SimEvent::CycleComplete(cc) => cc.sim_time_ms,
        SimEvent::VtmCycle(c) => c.sim_time_ms,
        SimEvent::VtmOperation(op) => op.sim_time_ms,
        SimEvent::OrderComplete(oc) => oc.sim_time_ms,
        SimEvent::Resource(r) => r.sim_time_ms,
        SimEvent::RelayStateChange(r) => r.time.as_ms(),
        SimEvent::ToteMoved(tm) => tm.time.as_ms(),
        SimEvent::ToteCreated(t) => t.time.as_ms(),
        SimEvent::ToteContentsChanged(c) => c.at.as_ms(),
        SimEvent::ToteTransit(t) => t.start.as_ms(),
        SimEvent::HtmDispatch(d) => d.sim_time_ms,
        SimEvent::OrderDispatch(d) => d.sim_time_ms,
        SimEvent::PipelineSnapshot(s) => s.sim_time_ms,
        SimEvent::ArrivalError(a) => a.sim_time_ms,
        SimEvent::WatchdogTimeout(w) => w.sim_time_ms,
        SimEvent::TripStart(t) => t.sim_time_ms,
        SimEvent::Induction(i) => i.sim_time_ms,
        SimEvent::VtmIdleReason(v) => v.at.as_ms(),
        SimEvent::InventoryDelta(d) => d.at.as_ms(),
        // Station dispatch diagnostics: not asserted by any golden trace, but
        // the matcher must place them on the timeline.
        SimEvent::HtmAssignment(a) => a.dispatch_time.as_ms(),
        SimEvent::HtmArrival(a) => a.arrival.as_ms(),
        SimEvent::DispatchRejection(r) => r.at.as_ms(),
        SimEvent::OrderStationPhase(p) => p.at.as_ms(),
        SimEvent::PresentationReadiness(r) => r.ready_at.as_ms(),
        SimEvent::InventoryVisitPhase(v) => v.at.as_ms(),
        SimEvent::BotPlanSnapshot(_) => 0,
        SimEvent::InventoryMove(m) => m.at.as_ms(),
        SimEvent::HtmRelayPickup(rp) => rp.start.as_ms(),
        SimEvent::HtmRelayDeposit(rd) => rd.start.as_ms(),
        SimEvent::DivingBoardSlotWait(db) => db.sim_time_ms,
        SimEvent::BarrierAdvanced(at) => at.as_ms(),
        SimEvent::CellHeld(hold) => hold.from.as_ms(),
        SimEvent::CellReleased(release) => release.at.as_ms(),
    }
}

fn matches_pattern(event: &SimEvent, pattern: &EventPattern) -> bool {
    match (event, pattern) {
        (_, EventPattern::Any) => true,

        (SimEvent::InitialPositions(ip), EventPattern::InitPositions { count }) => ip.positions.len() == *count,

        (SimEvent::ToteInitialStates(ti), EventPattern::ToteInitial { count }) => ti.totes.len() == *count,

        (SimEvent::Nav(n), EventPattern::Nav { robot_id, min_waypoints }) => {
            n.robot_id.0 == *robot_id && n.waypoints.len() >= *min_waypoints
        }

        (SimEvent::CycleComplete(cc), EventPattern::CycleComplete { robot_id, station_id, picks }) => {
            let pick_match = match cc.kind {
                CycleKind::Pick { qty, .. } => qty == *picks,
                CycleKind::Putaway { .. } | CycleKind::OrderLoad { .. } | CycleKind::OrderExtract { .. } => 0 == *picks,
            };
            cc.robot_id.0 == *robot_id && cc.station_id.0 == *station_id && pick_match
        }

        (SimEvent::OrderComplete(oc), EventPattern::OrderComplete { order_id }) => oc.order_id.0 == *order_id,

        (SimEvent::ToteMoved(tm), EventPattern::ToteMoved { tote_id, state }) => {
            tm.tote_id == ToteId(*tote_id) && tm.new_state == *state
        }

        (SimEvent::Resource(r), EventPattern::VtmForwardCompleted) => {
            matches!(r.kind, ResourceEventKind::VtmForwardCompleted { .. })
        }

        (SimEvent::Resource(r), EventPattern::VtmReturnCompleted) => {
            matches!(r.kind, ResourceEventKind::VtmReturnCompleted { .. })
        }

        (SimEvent::RelayStateChange(r), EventPattern::RelayReserved { cell_id }) => r.cell_id == *cell_id && r.reserved,

        (SimEvent::RelayStateChange(r), EventPattern::RelayFreed { cell_id }) => r.cell_id == *cell_id && !r.reserved,

        _ => false,
    }
}

fn format_pattern(p: &EventPattern) -> String {
    match p {
        EventPattern::Any => "Any".into(),
        EventPattern::InitPositions { count } => format!("InitPositions({count} bots)"),
        EventPattern::ToteInitial { count } => format!("ToteInitial({count} totes)"),
        EventPattern::Nav { robot_id, min_waypoints } => format!("Nav(robot {robot_id}, >={min_waypoints} wps)"),
        EventPattern::CycleComplete { robot_id, station_id, picks } => {
            format!("CC(robot {robot_id}, stn {station_id}, {picks} picks)")
        }
        EventPattern::OrderComplete { order_id } => format!("OC(order {order_id})"),
        EventPattern::ToteMoved { tote_id, state } => format!("ToteMoved({tote_id}, {state:?})"),
        EventPattern::VtmForwardCompleted => "VtmForwardCompleted".into(),
        EventPattern::VtmReturnCompleted => "VtmReturnCompleted".into(),
        EventPattern::RelayReserved { cell_id } => format!("RelayReserved({cell_id})"),
        EventPattern::RelayFreed { cell_id } => format!("RelayFreed({cell_id})"),
    }
}

fn format_event_short(e: &SimEvent) -> String {
    match e {
        SimEvent::InitialPositions(ip) => format!("InitPositions({} bots)", ip.positions.len()),
        SimEvent::ToteInitialStates(ti) => format!("ToteInitial({} totes)", ti.totes.len()),
        SimEvent::Nav(n) => format!("Nav({:?}-{}, {} wps)", n.robot_type, n.robot_id.0, n.waypoints.len()),
        SimEvent::CycleComplete(cc) => {
            let picks = match cc.kind {
                CycleKind::Pick { qty, .. } => qty,
                CycleKind::Putaway { .. } | CycleKind::OrderLoad { .. } | CycleKind::OrderExtract { .. } => 0,
            };
            format!("CC({:?}-{}, stn {}, {} picks)", cc.robot_type, cc.robot_id.0, cc.station_id.0, picks)
        }
        SimEvent::OrderComplete(oc) => format!("OC(order {})", oc.order_id.0),
        SimEvent::ToteMoved(tm) => format!("ToteMoved({}, {:?})", tm.tote_id, tm.new_state),
        SimEvent::Resource(r) => format!("Resource({:?})", r.kind),
        SimEvent::RelayStateChange(r) => {
            format!("Relay(cell {}, {})", r.cell_id, if r.reserved { "rsv" } else { "free" })
        }
        _ => format!("{:?}", e),
    }
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

/// Assert that `actual` events match `expected` entries in order.
///
/// Extra actual events between expected entries are allowed (skipped with a note).
/// Missing expected events cause a panic with context.
pub fn assert_trace(actual: &[SimEvent], expected: &[ExpectedEvent]) {
    let mut exp_idx = 0;
    let mut matched = 0;

    for (act_idx, event) in actual.iter().enumerate() {
        if exp_idx >= expected.len() {
            break; // All expected entries matched.
        }

        let exp = &expected[exp_idx];
        let act_time = event_time(event);

        if act_time == exp.time_ms && matches_pattern(event, &exp.pattern) {
            eprintln!("  ✓ [{:>3}] t={:>8}ms  {}", exp_idx, act_time, format_pattern(&exp.pattern));
            matched += 1;
            exp_idx += 1;
        }
        // If time matches but pattern doesn't, it's an extra event at same time — skip.
    }

    if exp_idx < expected.len() {
        eprintln!("\n  TRACE MISMATCH: matched {}/{} expected events", matched, expected.len());
        eprintln!(
            "  Next expected: [{exp_idx}] t={}ms {}",
            expected[exp_idx].time_ms,
            format_pattern(&expected[exp_idx].pattern)
        );
        eprintln!("\n  Remaining actual events:");
        for (i, e) in actual.iter().enumerate() {
            let t = event_time(e);
            if t >= expected[exp_idx].time_ms.saturating_sub(5000) {
                eprintln!("    [{i}] t={t:>8}ms  {}", format_event_short(e));
            }
        }
        panic!(
            "expected event [{exp_idx}] not found: t={}ms {}",
            expected[exp_idx].time_ms,
            format_pattern(&expected[exp_idx].pattern)
        );
    }

    eprintln!("  Trace: {matched}/{} expected events matched", expected.len());
}
