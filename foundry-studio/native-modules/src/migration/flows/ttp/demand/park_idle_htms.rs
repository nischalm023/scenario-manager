//! Pressure-relief parking moves a named idle HTM off an access cell needed
//! by another flow. Relay deposit selection uses the callback adapter; belt
//! and buffer tasks request a best-effort move directly.

use crate::flows::transport::windows::OneWindowJourney;
use crate::{
    config::IdleRelaySelection,
    error::{PlanError, ReservationError, planner::ErrContext},
    events::EventJournal,
    flows::ttp::task::{commit_plan_requests, move_distance},
    model::{
        ParkingRegistry, RobotId, SimDuration, SimTime,
        bot::BotService,
        htm::registry::{AvailableHtm, AvailableHtmRegistry},
        inventory::InventoryService,
        navigation::Navigation,
        relay::{ParkingRelief, ParkingReliefTarget},
    },
    pathfinding::{CollisionConfig, PathCacheKey, PathfinderError, estimate::cell_point_mm},
    utils::travel::select_min_prescreened,
};
use desgraph::{CellId, types::RobotType};

#[cfg(test)]
#[path = "park_idle_htms_tests.rs"]
mod tests;

/// Build the adapter `reserve_relay_for_deposit` calls when it is blocked.
pub fn parking_relief<'a>(
    navigation: &'a Navigation,
    available_htm: &'a AvailableHtmRegistry,
    bots: &'a BotService,
    inventory: &'a InventoryService,
    parking: Option<&'a ParkingRegistry>,
    events: &'a EventJournal,
) -> ParkingRelief<'a> {
    ParkingRelief::new(move |target| {
        Box::pin(async move {
            park_htm(navigation, available_htm, bots, inventory, parking, events, target)
                .await
                .map_err(|e| ReservationError::PressureReliefFailed { reason: e.to_string() })
        })
    })
}

/// Move the named no-tote HTM off `cell` when it is squatting on a cell the
/// flows need clear — a belt's dropoff, a buffer relay's access — with no
/// relay blocking to report. Best effort, like the relief it wraps.
pub async fn vacate_cell(
    navigation: &Navigation,
    available_htm: &AvailableHtmRegistry,
    bots: &BotService,
    inventory: &InventoryService,
    parking: Option<&ParkingRegistry>,
    events: &EventJournal,
    robot: RobotId,
    cell: CellId,
    not_before: SimTime,
) -> Result<bool, PlanError> {
    park_htm(
        navigation,
        available_htm,
        bots,
        inventory,
        parking,
        events,
        ParkingReliefTarget { relay_id: None, zone_id: None, cell, robot, not_before },
    )
    .await
}

/// Move the named no-tote HTM off `target.cell`; false if it cannot be moved.
pub async fn park_htm(
    navigation: &Navigation,
    available_htm: &AvailableHtmRegistry,
    bots: &BotService,
    inventory: &InventoryService,
    parking: Option<&ParkingRegistry>,
    events: &EventJournal,
    target: ParkingReliefTarget,
) -> Result<bool, PlanError> {
    let Some(parking) = parking else {
        return Ok(false);
    };

    let Some(candidate) = available_htm.squatter(target.cell, target.robot) else {
        return Ok(false);
    };
    let Some(entry) = available_htm.take(candidate.robot()) else {
        return Ok(false);
    };

    park_accepted_htm(navigation, available_htm, bots, inventory, Some(parking), events, entry, target.not_before).await
}

/// Spend an already accepted HTM claim; the parking task must not take it a
/// second time. Failure restores this exact availability and its floor hold.
pub(crate) async fn park_accepted_htm(
    navigation: &Navigation,
    available_htm: &AvailableHtmRegistry,
    bots: &BotService,
    inventory: &InventoryService,
    parking: Option<&ParkingRegistry>,
    events: &EventJournal,
    entry: AvailableHtm,
    not_before: SimTime,
) -> Result<bool, PlanError> {
    let oracle = &navigation.oracle;
    let Some(parking) = parking else {
        available_htm.put_back(entry)?;
        return Ok(false);
    };
    let htm_id = entry.robot();
    let from_cell = entry.cell();
    let drive = match bots.drive(htm_id, false) {
        Ok(drive) => drive,
        Err(error) => {
            available_htm.put_back(entry)?;
            return Err(error);
        }
    };
    let depart = entry.action_ready_at.max(not_before);
    let max_wait = SimDuration::seconds(120);

    let mut candidates: Vec<(CellId, SimTime)> = parking
        .find_candidates(from_cell, cell_point_mm(&navigation.map, from_cell), depart, htm_id)
        .into_iter()
        .filter(|&c| c != from_cell)
        // Parking needs an open-ended tail. The idle registry does not know
        // about every working HTM's floor hold; reject impossible destinations
        // before spending a full route search on them.
        .filter(|&cell| {
            oracle
                .entry_intervals(cell, depart, depart + max_wait, None)
                .iter()
                .any(|&(start, end)| start <= depart + max_wait && end.is_none())
        })
        .map(|c| (c, SimTime::ZERO))
        .collect();
    let lower_bound = |cell: CellId| -> u64 {
        match (cell_point_mm(&navigation.map, from_cell), cell_point_mm(&navigation.map, cell)) {
            (Some(from), Some(to)) => (from.x_mm - to.x_mm).hypot(from.y_mm - to.y_mm) as u64,
            _ => u64::MAX,
        }
    };
    // Rank statically, then try the remaining relays when a nearer relay cannot
    // be reached within the parking horizon. A failed parking retains the HTM.
    let (park_cell, reserved) = loop {
        let selected = if parking.selection() == IdleRelaySelection::Random {
            Ok(candidates.first().copied())
        } else {
            select_min_prescreened(
                candidates.clone(),
                5,
                |(cell, _available_at)| (lower_bound(*cell), *cell),
                |(cell, available_at)| match navigation.cache(htm_id)?.travel_time(PathCacheKey {
                    from: from_cell,
                    from_orientation: entry.orientation,
                    to: cell,
                    carrying_cargo: false,
                }) {
                    Ok(dur) => Ok(Some((cell, (depart + dur).max(available_at)))),
                    Err(PathfinderError::Unreachable { .. } | PathfinderError::NoStaticRoute { .. }) => Ok(None),
                    Err(e) => Err(PlanError::Pathfinder(e)),
                },
                |(_cell, score)| *score,
            )
        };
        let park_cell = match selected {
            Ok(Some((cell, _))) => cell,
            Ok(None) => {
                available_htm.put_back(entry)?;
                return Ok(false);
            }
            Err(error) => {
                available_htm.put_back(entry)?;
                return Err(error);
            }
        };
        candidates.retain(|&(cell, _)| cell != park_cell);
        let config = CollisionConfig {
            self_robot: htm_id,
            predecessor: Some(entry.cell_reservation.clone()),
            destination: None,
            sim_start: depart,
            dest_dwell: Some(SimDuration::MAX),
            collision_depth: 8,
            window_size: 0,
            replan_interval: 0,
            max_wait,
            max_iterations: 5_000_000,
            motion_model: navigation.model(htm_id)?.kind,
        };
        let reserved = navigation
            .pathfinder
            .reserve_path(
                from_cell,
                entry.orientation,
                park_cell,
                drive.as_ref(),
                oracle.as_ref(),
                &config,
                &mut OneWindowJourney { task: "ParkingRelief", events },
            )
            .await;
        match reserved {
            Ok(reserved) => break (park_cell, reserved),
            Err(
                PathfinderError::DestinationExceedsMaxWait { .. }
                | PathfinderError::UnreachableWithinHorizon { .. }
                | PathfinderError::Unreachable { .. }
                | PathfinderError::NoStaticRoute { .. },
            ) => {}
            Err(error) => {
                available_htm.put_back(entry)?;
                return Err(PlanError::Pathfinder(error)).err_context("ParkingRelief");
            }
        }
    };

    let tail = reserved.committed_reservations.last().cloned().ok_or_else(|| PlanError::TaskGraphInvariant {
        context: "ParkingRelief",
        detail: "reserve_path returned no reservations".into(),
    })?;
    let path = reserved.path;

    use crate::model::bot::{
        LegSpec,
        plan::{BotExpectation, Leg, TaskKind, WaypointKind},
    };
    let path_requests = commit_plan_requests(&path.waypoints, Some(SimDuration::MAX));
    let start = path.waypoints.first().map_or(depart, |waypoint| waypoint.start_time());
    let arrival = start + path.travel_time;
    bots.commit_task(
        htm_id,
        TaskKind::Park,
        &[LegSpec {
            leg: Leg::Parking { cell: park_cell },
            requests: &path_requests,
            action_ready_arrival: arrival,
            wp_kind: WaypointKind::Travel,
            expect: BotExpectation::empty(),
            carrying_after: None,
        }],
        inventory,
        Some(parking),
    )?;
    parking.reserve(park_cell, htm_id, arrival);
    available_htm.register(
        tail,
        arrival,
        path.waypoints.last().map(|waypoint| waypoint.orientation).unwrap_or(entry.orientation),
        None,
    )?;

    // TODO(nav-emission): mirrors HtmPickup/Deliver/Store — any direct
    // reserve_path caller that forgets Nav corrupts observer position tracking.
    {
        use crate::events::{NavEventData, TripEndpoint, TripPurpose, TripStartEvent};
        events.emit(TripStartEvent {
            robot_id: htm_id,
            robot_type: RobotType::Htm,
            purpose: TripPurpose::ToParking,
            // Parking belongs to no presentation cycle, and the bot idles at its
            // prior-leg destination — let observer continuity supply the origin.
            presentation_id: None,
            from: None,
            to: TripEndpoint::Parking,
            sim_time_ms: start.as_ms(),
        });
        events.emit(NavEventData {
            robot_id: htm_id,
            robot_type: RobotType::Htm,
            waypoints: path.waypoints.clone(),
            time: start,
            distance_mm: move_distance(&path, &navigation.map),
            carrying_cargo: false,
        });
    }

    Ok(true)
}
