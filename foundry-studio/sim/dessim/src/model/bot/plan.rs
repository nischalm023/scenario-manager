//! BotPlan: unified waypoint timeline for a single bot.
//!
//! Replaces `VecDeque<HtmTask>` with a flat, structurally uniform list of
//! waypoints. Each waypoint has a kind, cell, and [enter, exit) time window
//! that matches the reservation table exactly (identity, not equivalence).
//!
//! Appending a leg updates the plan and reservations through the same oracle.
//! Committed legs cannot be cancelled or replaced.
//!
//! Query verbs (`position_at`, `availability`, etc.) are read-only and don't
//! need the context.

use std::ops::Range;

use crate::model::conveyor::ConveyorId;
use desgraph::{CellId, StationId};

use crate::model::inventory::InventoryService;
use crate::model::parking::ParkingRegistry;
use crate::model::{RobotId, SimTime};
use crate::model::{SkuId, ToteId};
use crate::pathfinding::ReservationRequest;

// ---------------------------------------------------------------------------
// IDs
// ---------------------------------------------------------------------------

/// Opaque task identifier for grouping waypoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlanTaskId(pub u32);

/// Opaque leg identifier within a task.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlanLegId(pub u32);

// ---------------------------------------------------------------------------
// Waypoint kinds and metadata
// ---------------------------------------------------------------------------

/// What happens at this waypoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaypointKind {
    /// Bot is traversing to this cell.
    Travel,
    /// Bot picks up a tote at this cell.
    Pickup,
    /// Bot deposits a tote at this cell.
    Deposit,
    /// Bot presents tote to operator at station.
    Present,
    /// Bot is parked/idling at this cell.
    Park,
    /// Bot waits in place (blocked by reservation conflict or queue).
    Wait,
    /// Bot rotates in place on the current cell.
    Rotate,
}

/// A single waypoint in a bot's plan. The `enter`/`exit` are the EXACT
/// values committed to the reservation table — no reconstruction.
#[derive(Clone, Debug)]
pub struct PlanWaypoint {
    pub cell: CellId,
    /// Time the bot's reservation begins on this cell.
    pub enter: SimTime,
    /// Time the bot's reservation ends on this cell. None = open-ended.
    pub exit: Option<SimTime>,
    pub kind: WaypointKind,
    pub task_id: PlanTaskId,
    pub leg_id: PlanLegId,
}

// ---------------------------------------------------------------------------
// Task and Leg metadata
// ---------------------------------------------------------------------------

/// What kind of task this is (semantic label, not structural differentiation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskKind {
    Navigate,
    Deliver,
    Store,
    Park,
    Putaway,
}

/// What kind of leg within a task.
///
/// Variants that manage resources carry the resource handle directly.
/// BotPlan calls `added` when committing the leg to acquire its resources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leg {
    // --- HTM legs ---
    /// HTM travels between arbitrary floor cells without exchanging its tote.
    Travel,
    /// HTM travels to relay for tote pickup.
    RelayApproach,
    /// HTM travels from relay (or current pos) to station.
    StationApproach,
    /// Station work advances readiness while the existing cell hold continues.
    StationWork { cell: CellId },
    /// HTM travels to relay to deposit tote (Store).
    RelayDeposit,
    /// Committed HTM travel to an idle relay cell. Its resting reservation
    /// is closed when the next task takes the HTM from that destination.
    Parking { cell: CellId },
    /// HTM sets its tote onto a station's fixed conveyance and keeps standing
    /// where it is. A NO-MOVE leg: the bot is already on the station's
    /// `dropoff_cell` because it just presented there, so the leg exists only
    /// to record that it is no longer carrying anything. The tote's onward
    /// journey is the belt's, and no robot is scheduled for it.
    ConveyorDeposit { station: StationId },
    /// HTM travels to a machine's `collection_cell` and lifts a carrier back off the
    /// belt. Unlike the deposit this one moves: the bot is wherever it was
    /// idle, and the carrier has been riding with no robot attached. Names the
    /// CONVEYOR, since the collection cell belongs to the machine, not to either of
    /// the stations sharing it.
    ConveyorCollect { conveyor: ConveyorId },

    // --- VTM legs ---
    /// VTM travels between shelf and relay (diagonal X+Z movement).
    /// Used for both Forward (rack→relay) and Return (relay→rack).
    /// Direction determined by waypoint cells.
    VtmTravel,
    /// VTM fork operation: withdraw or insert tote at shelf or relay.
    /// Extend forks, lift/lower tote, retract forks.
    VtmForkOp,
}

impl Leg {
    /// Called when this leg is about to be appended to a plan.
    /// `arrival_ms` = last waypoint's enter time (when the bot arrives).
    /// Acquires resources. Returns Err if unavailable — plan unchanged.
    pub fn added(&self, parking: Option<&ParkingRegistry>, arrival_ms: u64, robot: RobotId) -> Result<(), PlanError> {
        match self {
            Leg::Parking { cell } => {
                if let Some(parking) = parking {
                    parking.reserve(*cell, robot, SimTime::ms(arrival_ms));
                }
                Ok(())
            }
            // TODO: RelayDeposit → relay try_reserve
            // TODO: Tote fill / inventory mutation — waypoint lifecycle action.
            _ => Ok(()),
        }
    }
}

/// Metadata for a task (group of legs).
#[derive(Clone, Debug)]
pub struct TaskMeta {
    pub id: PlanTaskId,
    pub kind: TaskKind,
    /// Waypoint index range [start, end) in BotPlan.waypoints.
    pub wp_range: Range<usize>,
}

/// Metadata for a leg within a task.
#[derive(Clone, Debug)]
pub struct LegMeta {
    pub id: PlanLegId,
    pub task_id: PlanTaskId,
    pub kind: Leg,
    /// Waypoint index range [start, end) in BotPlan.waypoints.
    pub wp_range: Range<usize>,
    /// What tote the bot was carrying BEFORE this leg started.
    pub carrying_before: Option<ToteId>,
    /// When the bot is action-ready at the leg's destination. Under Phase 1,
    /// reservation windows record leading-edge enter (= one traverse before
    /// action-ready for path-derived Move legs); availability checks must
    /// read this field instead of `requests.last().start`.
    pub action_ready_arrival: SimTime,
}

// ---------------------------------------------------------------------------
// BotPlan
// ---------------------------------------------------------------------------

/// Unified waypoint timeline for a single bot.
///
/// Structurally agnostic to task type — all waypoints are the same struct.
/// Task/leg identity is metadata, not structure.
///
/// Mutations receive inventory validation and parking services explicitly.
/// Query methods need no model.
pub struct BotPlan {
    pub robot_id: RobotId,
    waypoints: Vec<PlanWaypoint>,
    tasks: Vec<TaskMeta>,
    legs: Vec<LegMeta>,
    next_task_id: u32,
    next_leg_id: u32,
    /// Tote the bot is carrying at the plan frontier.
    /// Updated by append_leg when the leg's waypoint kind implies
    /// a pickup (Some) or deposit (None).
    carrying_tote: Option<ToteId>,
}

/// Expected bot state at the start of a leg. Validated by `append_leg`.
#[derive(Clone, Debug, Default)]
pub struct BotExpectation {
    /// Tote the bot should be carrying. None = empty.
    pub tote: Option<ToteId>,
    /// If set, the tote should contain this SKU at the leg's start time.
    /// Cross-validated against the model's InventoryService when present.
    pub contents: Option<SkuId>,
}

impl BotExpectation {
    pub fn empty() -> Self {
        Self { tote: None, contents: None }
    }

    pub fn with_tote(tote_id: ToteId) -> Self {
        Self { tote: Some(tote_id), contents: None }
    }

    pub fn with_tote_containing(tote_id: ToteId, sku: SkuId) -> Self {
        Self { tote: Some(tote_id), contents: Some(sku) }
    }
}

pub use crate::error::PlanError;

impl BotPlan {
    /// Create an empty plan for a bot.
    pub fn new(robot_id: RobotId) -> Self {
        Self {
            robot_id,
            waypoints: Vec::new(),
            tasks: Vec::new(),
            legs: Vec::new(),
            next_task_id: 0,
            next_leg_id: 0,
            carrying_tote: None,
        }
    }

    /// Create a plan seeded with an initial position at T=0.
    /// Used by bot registration — no model handle needed.
    pub fn new_at(robot_id: RobotId, cell: CellId) -> Self {
        let mut plan = Self::new(robot_id);
        let task_id = plan.register_task(TaskKind::Park);
        let leg_id = PlanLegId(plan.next_leg_id);
        plan.next_leg_id += 1;
        plan.waypoints.push(PlanWaypoint {
            cell,
            enter: SimTime::ms(0),
            exit: None,
            kind: WaypointKind::Park,
            task_id,
            leg_id,
        });
        plan.legs.push(LegMeta {
            id: leg_id,
            task_id,
            kind: Leg::Parking { cell },
            wp_range: 0..1,
            carrying_before: None,
            action_ready_arrival: SimTime::ms(0),
        });
        if let Some(task) = plan.tasks.iter_mut().find(|t| t.id == task_id) {
            task.wp_range.end = 1;
        }
        plan
    }

    // -----------------------------------------------------------------------
    // Build
    // -----------------------------------------------------------------------

    /// Begin a new task. Returns the TaskId. Legs are appended to this task
    /// until `close_task` is called.
    pub fn register_task(&mut self, kind: TaskKind) -> PlanTaskId {
        let id = PlanTaskId(self.next_task_id);
        self.next_task_id += 1;
        let start = self.waypoints.len();
        self.tasks.push(TaskMeta {
            id,
            kind,
            wp_range: start..start, // will be extended by append_leg / close_task
        });
        id
    }

    /// Append a leg from committed reservation windows.
    ///
    /// `windows` are the EXACT `Waypoint` entries committed by `reserve_path`.
    /// Each becomes a `PlanWaypoint` with `enter`/`exit` matching the reservation
    /// table. No reconstruction, no derivation. (A window here is a hold's time
    /// range, not a window of a foreshortened path, which appends its own
    /// `LegSpec`.)
    ///
    /// Append a leg, validating the bot's expected state.
    ///
    /// `expect`: what the bot should be carrying at the start of this leg,
    /// and optionally what SKU the tote should contain. Validated against
    /// the plan's tracked state and (if inventory is in ctx) the
    /// InventoryService.
    ///
    /// `carrying_after`: tote the bot carries after this leg.
    /// None = empty, Some(id) = carrying.
    pub fn append_leg(
        &mut self,
        task_id: PlanTaskId,
        leg_kind: Leg,
        requests: &[ReservationRequest],
        action_ready_arrival: SimTime,
        wp_kind: WaypointKind,
        expect: BotExpectation,
        carrying_after: Option<ToteId>,
        inventory: &InventoryService,
        parking: Option<&ParkingRegistry>,
    ) -> Result<PlanLegId, PlanError> {
        // Empty `requests` is the no-move leg (e.g., HTM already at relay,
        // predecessor's open hold continues — see Phase 1 spec §2). The
        // leg records `action_ready_arrival` and updates carrying state
        // without committing duplicate same-cell waypoints.

        // Invariant: only the last request in a leg may be open-ended.
        for (i, req) in requests.iter().enumerate() {
            if req.window.end().is_none() && i + 1 < requests.len() {
                panic!("open-ended request at non-terminal position {} in leg {:?}, task={:?}", i, leg_kind, task_id,);
            }
        }

        // Validate carrying state against plan's tracked state.
        if self.carrying_tote != expect.tote {
            let leg_summary: Vec<String> =
                self.legs.iter().map(|l| format!("{:?}(carrying_before={:?})", l.kind, l.carrying_before)).collect();
            eprintln!(
                "[append_leg] CarryingMismatch {:?} leg={:?} task={:?} expected={:?} actual={:?} \
                legs=[{}]",
                self.robot_id,
                leg_kind,
                task_id,
                expect.tote,
                self.carrying_tote,
                leg_summary.join(", ")
            );
            return Err(PlanError::CarryingMismatch { expected: expect.tote, actual: self.carrying_tote });
        }

        // Cross-validate tote contents against InventoryService. Anchor at
        // the leg's first request when present; for empty (no-move) legs,
        // anchor at `action_ready_arrival` since the bot hasn't moved.
        if let (Some(tote_id), Some(expected_sku)) = (expect.tote, expect.contents) {
            let anchor = requests.first().map(|r| r.window.start()).unwrap_or(action_ready_arrival);
            if inventory.tote(tote_id)?.sku_qty_at(expected_sku, anchor) == 0 {
                return Err(PlanError::TaskGraphInvariant {
                    context: "BotExpectation",
                    detail: format!(
                        "tote {:?} should contain {:?} at T={}ms but has qty=0",
                        tote_id,
                        expected_sku,
                        anchor.as_ms(),
                    ),
                });
            }
        }

        // Validate task exists.
        if !self.tasks.iter().any(|t| t.id == task_id) {
            return Err(PlanError::UnknownTask(task_id));
        }

        // Validate time ordering against existing plan. For non-empty
        // legs, check the proposed first request's start. For empty legs
        // there is no waypoint to time-order; `action_ready_arrival` is
        // tracked on `LegMeta` only.
        if let (Some(first_req), Some(last)) = (requests.first(), self.waypoints.last()) {
            let proposed_enter = first_req.window.start();
            let last_exit = last.exit.unwrap_or(SimTime::EOT);
            if proposed_enter < last_exit && proposed_enter < last.enter {
                return Err(PlanError::TimeOverlap { existing_exit: last_exit, proposed_enter });
            }
        }

        // Validate internal time ordering.
        for i in 1..requests.len() {
            if requests[i].window.start() < requests[i - 1].window.start() {
                return Err(PlanError::InternalTimeOrder { index: i });
            }
        }

        // Acquire resources BEFORE committing waypoints. Use action_ready_arrival
        // (when the bot is action-ready at the leg destination), NOT the raw
        // window start which is leading-edge enter under the new contract.
        leg_kind.added(parking, action_ready_arrival.as_ms(), self.robot_id)?;

        let leg_id = PlanLegId(self.next_leg_id);
        self.next_leg_id += 1;
        let carrying_before = self.carrying_tote;
        let wp_start = self.waypoints.len();

        for req in requests {
            self.waypoints.push(PlanWaypoint {
                cell: req.cell,
                enter: req.window.start(),
                exit: req.window.end(),
                kind: wp_kind,
                task_id,
                leg_id,
            });
        }

        let wp_end = self.waypoints.len();

        self.legs.push(LegMeta {
            id: leg_id,
            task_id,
            kind: leg_kind,
            wp_range: wp_start..wp_end,
            carrying_before,
            action_ready_arrival,
        });

        // Extend the task's wp_range.
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) {
            task.wp_range.end = wp_end;
        }

        // Update tracked carrying state.
        self.carrying_tote = carrying_after;

        Ok(leg_id)
    }

    // -----------------------------------------------------------------------
    // Query (no context needed)
    // -----------------------------------------------------------------------

    /// Where is the bot at sim time T? Returns the waypoint whose
    /// [enter, exit) contains T.
    pub fn position_at(&self, at: SimTime) -> Option<&PlanWaypoint> {
        // Binary search: find the last waypoint with enter <= at.
        let idx = self.waypoints.partition_point(|wp| wp.enter <= at);
        if idx == 0 {
            return None;
        }
        let wp = &self.waypoints[idx - 1];
        // Open-ended (exit=None) means bot is here indefinitely.
        if wp.exit.map(|e| at < e).unwrap_or(true) { Some(wp) } else { None }
    }

    /// When and where is the bot next available?
    /// Parking legs: available at the first waypoint of the parking leg
    /// (the instant real work ends, before travel to parking cell).
    /// Active legs: available when the bot is action-ready at the leg's
    /// destination (`Leg.action_ready_arrival` — the producer's
    /// canonical "bot has arrived" timestamp).
    pub fn availability(&self) -> Option<(SimTime, CellId)> {
        let last_leg = self.legs.last()?;
        if matches!(last_leg.kind, Leg::Parking { .. }) {
            let first_parking_wp = &self.waypoints[last_leg.wp_range.start];
            Some((first_parking_wp.enter, first_parking_wp.cell))
        } else {
            let wp = self.waypoints.last()?;
            Some((last_leg.action_ready_arrival, wp.cell))
        }
    }

    /// Tote the bot is carrying at the plan frontier. None = empty.
    pub fn carrying_tote(&self) -> Option<ToteId> {
        self.carrying_tote
    }

    /// The last task in the plan.
    pub fn last_task(&self) -> Option<&TaskMeta> {
        self.tasks.last()
    }

    /// The last leg in the plan.
    pub fn last_leg(&self) -> Option<&LegMeta> {
        self.legs.last()
    }

    /// All waypoints belonging to a task, in order.
    pub fn waypoints_for_task(&self, id: PlanTaskId) -> &[PlanWaypoint] {
        self.tasks.iter().find(|t| t.id == id).map(|t| &self.waypoints[t.wp_range.clone()]).unwrap_or(&[])
    }

    /// All waypoints belonging to a leg, in order.
    pub fn waypoints_for_leg(&self, id: PlanLegId) -> &[PlanWaypoint] {
        self.legs.iter().find(|l| l.id == id).map(|l| &self.waypoints[l.wp_range.clone()]).unwrap_or(&[])
    }

    /// The last waypoint in the plan, if any.
    pub fn last(&self) -> Option<&PlanWaypoint> {
        self.waypoints.last()
    }

    /// Total number of waypoints in the plan.
    pub fn len(&self) -> usize {
        self.waypoints.len()
    }

    /// Is the plan empty?
    pub fn is_empty(&self) -> bool {
        self.waypoints.is_empty()
    }

    /// Iterate all waypoints in time order.
    pub fn iter(&self) -> impl Iterator<Item = &PlanWaypoint> {
        self.waypoints.iter()
    }

    /// All task metadata.
    pub fn tasks(&self) -> &[TaskMeta] {
        &self.tasks
    }

    /// All leg metadata.
    pub fn legs(&self) -> &[LegMeta] {
        &self.legs
    }
}
