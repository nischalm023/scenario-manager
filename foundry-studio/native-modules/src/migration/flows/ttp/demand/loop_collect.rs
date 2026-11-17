//! Lifting an inventory tote back off a station's belt.
//!
//! Relative to `demand`, this is the other end of `loop_deposit`. A tote that
//! boarded at the `dropoff_cell` rides its circuit with no robot attached and
//! nothing scheduled for it; when the circuit is done it is standing at the
//! `collection_cell` waiting for someone to pick it up. This actor is that
//! someone.
//!
//! It ends where the ordinary flow already works: the HTM registers at the
//! collection cell holding the tote, and the presentation clearance it
//! spawns there walks the tote home, or yields the HTM to a productive
//! claim that departs no later than the clearance. The collect plans no
//! return of its own.
//!
//! The collect is ONE leg: the bot travels empty and arrives carrying. There
//! is no separate pickup, because a leg states what the bot holds after it,
//! and "after travelling to the collection cell it holds the tote" is the
//! whole event.

use crate::events::{EventJournal, ToteSource};
use crate::flows::transport::windows::OneWindowJourney;
use crate::flows::ttp::task::clearance::ClearPresentation;
use crate::model::ParkingRegistry;
use crate::model::bot::BotService;
use crate::model::factories::world_inputs;
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::model::inventory::InventoryService;
use crate::model::navigation::Navigation;
use crate::task;
use crate::task::TaskContext;

use desgraph::{CellId, StationId};
use desmap::Station;
use tokio::select;

use crate::{
    error::planner::ErrContext,
    events::ToteMovedEvent,
    flows::ttp::task::commit_plan_requests,
    model::ToteLocation,
    model::bot::{
        LegSpec,
        plan::{BotExpectation, Leg, TaskKind, WaypointKind},
    },
    model::conveyor::{ConveyorId, ConveyorLoops},
    model::htm::registry::AvailableHtm,
    observer::time_slice::ToteVisualState,
    pathfinding::estimate::separation_mm,
    pathfinding::{CollisionConfig, PathCacheKey, PathfinderError},
    prelude::*,
    utils::travel::select_min_prescreened,
};

use rustc_hash::FxHashMap;

#[derive(Display)]
#[display("LoopCollect")]
#[task]
pub struct LoopCollect {
    navigation: Arc<Navigation>,
    bots: BotService,
    inventory: InventoryService,
    available: AvailableHtmRegistry,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
    conveyors: ConveyorLoops,
    /// Where each MACHINE hands its carrier back. Keyed by conveyor because
    /// the collection cell is shared by the machine's two stations.
    collection_cell: FxHashMap<ConveyorId, CellId>,
    /// The station each machine's collected tote is cleared in the name of:
    /// one of the two that share its collection cell.
    clearing_station: FxHashMap<ConveyorId, StationId>,
    /// Every cell a robot on the floor must drive to REACH that collection
    /// cell: the machine's declared lane and the cell that lane is entered
    /// from. A machine authored cell by cell has exactly one way in, so an
    /// open-ended hold anywhere along it admits no arrival at any horizon.
    approach: FxHashMap<ConveyorId, Vec<CellId>>,
    /// Rotates the belt considered first. One collect is committed per
    /// iteration, so without it `occupied()`'s id order is a fixed priority
    /// and the lowest-numbered belt takes everything: measured on DFW6 over
    /// 30 sim-min, collections went 90 / 10 / 3 / 0 across the four while
    /// every belt sat at its capacity of ten, and c3 was never served once.
    iteration_offset: usize,
}

/// Every cell a robot on the floor must cross to reach a machine's collection
/// cell: its declared lane's own steps, and the cell that lane is ENTERED from.
///
/// `None` for a station with no collection cell, or none whose declared lane
/// serves it -- neither is a machine this flow collects from.
///
/// The entry cell is the lane's, not the station's. On every EasyLoop in the
/// tree the two coincide: the lane's head is the first cell under the west
/// belt and the one arc into it comes off the dropoff, which is also the
/// station's `dropoff_cell`, so nothing about DFW6 or the minimal map moves.
/// They are not the same thing, though. A lane binds to what it ENDS at, and
/// nothing requires the arc into its head to come off a handover: a document
/// may run a collection lane in from an ordinary floor cell, and then reading
/// the dropoff would miss the cell a robot actually has to cross (skipping a
/// belt that is truly blocked is the whole point of the reading) and would
/// skip a belt over a dropoff nobody has to drive through. So the cell the
/// LANE resolved to is the one that is checked.
pub fn approach_to_collection(station: &Station) -> Option<Vec<CellId>> {
    let collect = station.collection_cell?;
    let lane = station.handoff_queues.iter().find(|q| q.into == collect)?;
    Some(lane.path.iter().copied().chain(lane.entry_from).collect())
}

impl LoopCollect {
    async fn collect_one(&mut self, ctx: &SchedCtx) -> Result<bool, PlanError> {
        // Only a belt whose head tote has finished its circuit is worth a
        // robot. `occupied` is sorted, so which is served first does not
        // depend on hash order. Walk BELTS, not stations: a machine's two
        // stations share one belt, and the tote comes off at the machine's
        // single collection cell.
        let occupied = self.conveyors.occupied();
        let rotated: Vec<ConveyorId> = if occupied.is_empty() {
            occupied
        } else {
            let offset = self.iteration_offset % occupied.len();
            self.iteration_offset = self.iteration_offset.wrapping_add(1);
            (0..occupied.len()).map(|i| occupied[(i + offset) % occupied.len()]).collect()
        };
        let Some((conveyor, cell, tote_id, available_at)) = rotated.into_iter().find_map(|c| {
            let (tote_id, available_at) = self.conveyors.next_available(c)?;
            let cell = *self.collection_cell.get(&c)?;
            // An open-ended hold by another robot admits no arrival at any
            // horizon — usually the previous collector standing there with
            // its tote, waiting for the return flow to claim it. Searching
            // anyway is a guaranteed-failing A* per fleet signal (measured:
            // ~11,800 failures pinning a core); skip the belt until the
            // holder's next leg bounds the hold, and let the rotation serve
            // the other machines meanwhile.
            let held_open =
                |at: CellId| self.navigation.oracle.reservations_for_cell(at).into_iter().any(|r| r.is_open_ended());
            if held_open(cell) {
                return None;
            }
            // The same reading, applied to the whole WAY IN rather than to its
            // last cell. A machine authored cell by cell reaches its collection
            // through one single-file lane, entered at the one cell with an arc
            // into its head, so a robot standing anywhere along that lane -- a
            // depositor idling on the cell the lane is entered at -- admits no
            // fleet arrival at any horizon either. Measured on DFW6 without
            // this: 614 `UnreachableWithinHorizon` failures inside 480 s of
            // sim, one five-million-iteration search apiece, always the same
            // two cells.
            if self.approach.get(&c).is_some_and(|cells| cells.iter().copied().any(held_open)) {
                return None;
            }
            Some((c, cell, tote_id, available_at))
        }) else {
            return Ok(false);
        };

        // An EMPTY htm: one already carrying something has a trip of its own.
        //
        // Chosen by ARRIVAL, not by who has been idle longest. Ranking on
        // `available_at` alone ignores where the robot is standing, and on a
        // 92m floor that is not a tie-break, it is the whole cost: measured
        // over 30 sim-min, the collector was dispatched from more than 60m
        // away 200 times and from 30-60m another 120, against 16 inside 15m —
        // repeatedly with fifty to ninety-eight free robots to pick from. The
        // trip is the work here, so distance is the term that matters most.
        //
        // Prescreen on straight-line separation, refine on the path cache, the
        // shape `park_idle_htms` already uses and coding-standards §8 requires:
        // a spatial estimate may RANK candidates, only a pathfinder result may
        // commit. `Unreachable`/`NoStaticRoute` skip the candidate; every other
        // pathfinder error is fatal.
        // Empty, and not already standing on the collection cell: a leg
        // needs a move, and taking the squatter only to put it back would
        // re-wake this actor on its own restore.
        let candidates = self.available.peek_matching(|htm| htm.tote.is_none() && htm.cell() != cell);
        let Some(peeked) = select_min_prescreened(
            candidates,
            if self.navigation.models().len() > 1 { usize::MAX } else { 8 },
            |htm: &AvailableHtm| {
                (
                    separation_mm(&self.navigation.map, htm.cell(), cell)
                        .map(|mm| mm.round() as u64)
                        .unwrap_or(u64::MAX),
                    htm.robot(),
                )
            },
            |htm: AvailableHtm| match self.navigation.cache(htm.robot())?.travel_time(PathCacheKey {
                from: htm.cell(),
                from_orientation: htm.orientation,
                to: cell,
                carrying_cargo: false,
            }) {
                // Earliest it could actually be there: free, then travelled.
                Ok(dur) => Ok(Some((htm.available_at() + dur, htm))),
                Err(PathfinderError::Unreachable { .. } | PathfinderError::NoStaticRoute { .. }) => Ok(None),
                Err(e) => Err(PlanError::Pathfinder(e)),
            },
            |(eta, htm): &(SimTime, AvailableHtm)| (*eta, htm.robot()),
        )?
        .map(|(_, htm)| htm) else {
            return Ok(false);
        };
        let htm_id = peeked.robot();
        let from_cell = peeked.cell();
        let drive = self.bots.drive(htm_id, false)?;
        let Some(entry) = self.available.take(htm_id) else {
            return Ok(false);
        };
        let depart = entry.action_ready_at;

        let config = CollisionConfig {
            self_robot: htm_id,
            predecessor: Some(entry.cell_reservation.clone()),
            destination: None,
            sim_start: depart,
            // Open-ended, not MAX: MAX is only legal on a parking cell,
            // and a collection cell is not one. The bot holds the cell
            // from arrival until the pickup, the same "arrive early, wait
            // at the cell" shape `HtmStore` uses for a relay.
            dest_dwell: None,
            collision_depth: 8,
            window_size: 0,
            replan_interval: 0,
            max_wait: SimDuration::seconds(120),
            max_iterations: 5_000_000,
            motion_model: self.navigation.model(htm_id)?.kind,
        };
        let reserved = match self
            .navigation
            .pathfinder
            .reserve_path(
                from_cell,
                entry.orientation,
                cell,
                drive.as_ref(),
                self.navigation.oracle.as_ref(),
                &config,
                &mut OneWindowJourney { task: "LoopCollect", events: &self.events },
            )
            .await
        {
            Ok(reserved) => reserved,
            Err(refused @ PathfinderError::WindowRefused(_)) => return Err(PlanError::Pathfinder(refused)),
            Err(_) => {
                // The collection cell is busy or unreachable right now. Put
                // the bot back untouched and leave the tote on the belt; it
                // does not spoil and the next real signal tries again.
                self.available.put_back(entry)?;
                return Ok(false);
            }
        };
        let tail = reserved.committed_reservations.last().cloned().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "LoopCollect",
            detail: "reserve_path returned no reservations".into(),
        })?;
        // The bot may reach the collection cell before the tote does. It waits
        // there on its own committed hold; the pickup is the later of the two.
        // This is why nothing polls a clock: the belt states WHEN, and the
        // schedule is built around it. Reaching the cell is not having the
        // tote either — lifting it off costs the assigned robot’s pickup time, the
        // robot's action, the mirror of the unload.
        let start = reserved.path.waypoints.first().map_or(depart, |waypoint| waypoint.start_time());
        let in_hand = (start + reserved.path.travel_time).max(available_at);
        let arrival = in_hand + self.bots.model(htm_id)?.figures.pickup_time();

        // Take only now that the robot is committed to arriving. Taking
        // earlier and failing would have reordered the belt.
        if !self.conveyors.take(conveyor, tote_id) {
            return Err(PlanError::TaskGraphInvariant {
                context: "LoopCollect",
                detail: format!("conveyor {conveyor:?} lost {tote_id:?} between plan and take"),
            });
        }

        let path_requests = commit_plan_requests(&reserved.path.waypoints, None);
        self.bots
            .commit_task(
                htm_id,
                TaskKind::Deliver,
                &[LegSpec {
                    leg: Leg::ConveyorCollect { conveyor },
                    requests: &path_requests,
                    action_ready_arrival: arrival,
                    wp_kind: WaypointKind::Pickup,
                    // Departs empty, arrives holding: one leg states both.
                    expect: BotExpectation::empty(),
                    carrying_after: Some(tote_id),
                }],
                &self.inventory,
                self.parking.as_ref(),
            )
            .err_context("LoopCollect")?;
        self.inventory.tote(tote_id)?.move_to(ToteLocation::WithRobot { robot_id: htm_id }, arrival)?;
        // Same instant on the render channel, which the location log does not
        // feed. Without it the tote is still drawn wherever it was last
        // emitted and this HTM drives away from the collection cell
        // empty-handed.
        self.events.emit(ToteMovedEvent {
            tote_id,
            time: arrival,
            new_state: ToteVisualState::OnBot { bot_id: htm_id.0 },
        });

        let stay = self.available.register(
            tail,
            arrival,
            reserved.path.waypoints.last().map(|waypoint| waypoint.orientation).unwrap_or(entry.orientation),
            None,
        )?;
        let station = self.clearing_station.get(&conveyor).copied().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "LoopCollect",
            detail: format!("conveyor {conveyor:?} has a collection cell but no station"),
        })?;
        ctx.add_task(ClearPresentation::new(
            stay,
            station,
            ToteSource::Pick,
            self.navigation.clone(),
            self.available.clone(),
            ctx.barrier().clock(),
        ));
        Ok(true)
    }
}

#[async_trait(?Send)]
impl Task for LoopCollect {
    fn create(ctx: &TaskContext) -> Result<Self, PlanError> {
        let navigation = ctx.state::<Navigation>()?;
        let parking = ctx.state::<ParkingRegistry>()?;
        let conveyors = ctx.state::<ConveyorLoops>()?.as_ref().clone();
        let collection_cell: FxHashMap<ConveyorId, CellId> = navigation
            .map
            .stations
            .iter()
            .filter_map(|s| Some((conveyors.of_station(s.id)?, s.collection_cell?)))
            .collect();
        let clearing_station: FxHashMap<ConveyorId, StationId> = navigation
            .map
            .stations
            .iter()
            .filter(|s| s.collection_cell.is_some())
            .filter_map(|s| Some((conveyors.of_station(s.id)?, s.id)))
            .collect();
        // The lane into each collection cell, plus the cell it is entered
        // from. A machine's two stations name the same lane, so this is
        // written twice with the same value and that is harmless.
        let mut approach: FxHashMap<ConveyorId, Vec<CellId>> = FxHashMap::default();
        for station in &navigation.map.stations {
            let Some(conveyor) = conveyors.of_station(station.id) else { continue };
            let Some(cells) = approach_to_collection(station) else { continue };
            approach.insert(conveyor, cells);
        }
        Ok(Self {
            navigation,
            bots: ctx.state::<BotService>()?.as_ref().clone(),
            inventory: ctx.state::<InventoryService>()?.as_ref().clone(),
            available: ctx.state::<AvailableHtmRegistry>()?.as_ref().clone(),
            parking: (parking.candidate_count() > 0).then(|| parking.as_ref().clone()),
            events: ctx.state::<EventJournal>()?.as_ref().clone(),
            conveyors,
            collection_cell,
            clearing_station,
            approach,
            iteration_offset: 0,
        })
    }

    fn holds_completion() -> bool {
        false
    }

    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _owner: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _proposal: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        if self.collection_cell.is_empty() {
            return Ok(());
        }
        loop {
            let fleet_seen = self.available.version().seen();
            let belt_seen = self.conveyors.version().seen();

            if self.collect_one(&ctx).await? {
                continue;
            }

            select! {
                biased;
                // A board puts a new tote in reach; a freed robot may be the
                // nearer collector.
                _ = self.conveyors.version().past(belt_seen) => {}
                _ = self.available.version().past(fleet_seen) => {}
            }
        }
    }
}
