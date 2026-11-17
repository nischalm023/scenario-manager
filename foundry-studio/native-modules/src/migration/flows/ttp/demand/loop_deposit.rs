//! Setting an inventory tote onto a station's belt and leaving without it.
//!
//! Relative to `demand`, this is the actor that ends an EasyLoop delivery.
//! Every other station hands its tote straight back to the HTM that brought
//! it, and the presentation's clearance walks it home. A loop does not: the
//! HTM sets the tote down on the belt at the station's `dropoff_cell`,
//! departs empty, and the tote rides to the `collection_cell` on its own.
//! That gap is the machine's whole point, because it stops a robot being
//! held for the duration of an operator's pick.
//!
//! The deposit is a NO-MOVE leg. The HTM is already standing on the dropoff
//! cell (it just presented there), so nothing travels; the leg exists to
//! record that the bot is no longer carrying anything, which is the only way
//! plan state learns it. `plan.rs::append_leg` names this case: "Empty
//! `requests` is the no-move leg".

use crate::events::EventJournal;
use crate::model::ParkingRegistry;
use crate::model::bot::BotService;
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::model::inventory::InventoryService;
use crate::model::navigation::Navigation;
use crate::task;
use crate::task::TaskContext;

use desgraph::{CellId, StationId};
use desmap::Station;
use rustc_hash::FxHashMap;
use tokio::select;

use crate::{
    events::ToteMovedEvent,
    model::bot::{
        LegSpec,
        plan::{BotExpectation, Leg, TaskKind, WaypointKind},
    },
    model::conveyor::{BoardRefusal, ConveyorLoops},
    model::{ToteDesignation, ToteLocation},
    observer::time_slice::ToteVisualState,
    prelude::*,
};

use super::park_idle_htms;

#[derive(Display)]
#[display("LoopDeposit")]
#[task]
pub struct LoopDeposit {
    navigation: Arc<Navigation>,
    bots: BotService,
    inventory: InventoryService,
    available: AvailableHtmRegistry,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
    conveyors: ConveyorLoops,
    /// The designation each dropoff cell's belt accepts, from the port's own
    /// `of`. An EasyLoop's conveyance carries INVENTORY totes: a tote rides
    /// out to a seat, is worked, and rides back to be collected for
    /// back-to-store. An ORDER carrier never touches it — it stays on its
    /// robot and moves between the machine's stations. Boarding one here sent
    /// orders out of the collection port, which is not a thing the machine
    /// does.
    accepts: BeltBoarding,
    /// The station each dropoff cell belongs to. Built once: a station's cells
    /// do not move.
    station_of_dropoff: FxHashMap<CellId, StationId>,
    /// The cell a served station itself occupies: its SEAT, where the operator
    /// works a presented tote. Distinct per station, unlike the presentation
    /// cell, which an EasyLoop's two stations share with the dropoff cell. Only
    /// used to place the riding tote for the renderer.
    seat_cell: FxHashMap<StationId, CellId>,
    /// The cell a served station's machine hands the carrier back on. Same
    /// purpose: the last point of the circuit the tote is drawn at.
    handback_cell: FxHashMap<StationId, CellId>,
}

/// The tote designation each station's belt boards at its dropoff cell,
/// built once from the map: the boarding rule `LoopDeposit` applies, which a
/// pick asks before a clearance could send home a tote the belt would take.
#[derive(Clone)]
pub struct BeltBoarding(Arc<FxHashMap<CellId, ToteDesignation>>);

impl BeltBoarding {
    pub fn of(stations: &[Station]) -> Self {
        Self(Arc::new(stations.iter().filter_map(belt_boarding).collect()))
    }

    /// The designation the belt at `cell` boards, when a belt boards there.
    pub fn at(&self, cell: CellId) -> Option<ToteDesignation> {
        self.0.get(&cell).copied()
    }

    /// Whether the belt at `cell` boards a tote of `designation`.
    pub fn boards(&self, cell: CellId, designation: ToteDesignation) -> bool {
        self.at(cell) == Some(designation)
    }
}

/// A station's dropoff cell and the tote designation its belt boards there,
/// from the port's own `of`. A designation the format grows later is skipped,
/// not guessed at: the belt then accepts nothing and the defect is visible as
/// a stalled loop rather than as wrong totes.
fn belt_boarding(station: &Station) -> Option<(CellId, ToteDesignation)> {
    let cell = station.dropoff_cell?;
    let designation = match station.conveyance_accepts.as_deref()? {
        "inventory" => ToteDesignation::Inventory,
        "order" => ToteDesignation::Order,
        _ => return None,
    };
    Some((cell, designation))
}

impl LoopDeposit {
    /// An EMPTY robot loitering on an ingress has nothing left to do there
    /// and blocks every delivery behind it, so moving it on is worth more
    /// than starting another deposit.
    ///
    /// The bot is not sent away the instant it deposits — it is registered
    /// available where it stands, and the flows that hand out real work get
    /// first call on it; a robot with work leaves under its own task and the
    /// cell frees itself, which is how every other station already works.
    /// Only a robot that nobody wanted is still standing here when this step
    /// comes round again, and only that one parks. Vacating unconditionally
    /// instead cost one parking trip per pick: 10,504 against 10,508 picks,
    /// visible in desvis as a constant stream to the parking strip.
    async fn vacate_one_loiterer(&self) -> bool {
        let loitering = self
            .available
            .peek_matching(|htm| htm.tote.is_none() && self.station_of_dropoff.contains_key(&htm.cell()))
            .into_iter()
            .min_by_key(|h| (h.available_at(), h.robot()));
        let Some(idle) = loitering else { return false };
        park_idle_htms::vacate_cell(
            &self.navigation,
            &self.available,
            &self.bots,
            &self.inventory,
            self.parking.as_ref(),
            &self.events,
            idle.robot(),
            idle.cell(),
            idle.available_at(),
        )
        .await
        .unwrap_or(false)
    }

    fn deposit_one(&self) -> Result<bool, PlanError> {
        // Only a carrier the belt accepts. Without this the deposit took
        // whatever stood on the cell, and because an EasyLoop station's
        // presentation cell IS its dropoff cell, close-out's empty ORDER tote
        // was boarded onto the inventory belt and left through the collection
        // port.
        let candidates = self.available.peek_matching(|htm| {
            let Some(tote) = htm.tote.as_ref() else { return false };
            let Some(accepts) = self.accepts.at(htm.cell()) else { return false };
            // A full belt is a LASTING refusal: gate it here so the robot is
            // never taken only to be put back, which would re-wake this actor
            // on its own restore. The belt version signal covers the drain.
            let has_room = self
                .station_of_dropoff
                .get(&htm.cell())
                .and_then(|&sid| self.conveyors.of_station(sid))
                .is_some_and(|c| self.conveyors.has_room(c));
            has_room && self.inventory.tote(tote.tote_id).is_ok_and(|t| t.designation() == accepts)
        });
        let Some(peeked) = candidates.into_iter().min_by_key(|h| (h.available_at(), h.robot())) else {
            return Ok(false);
        };
        let dropoff_cell = peeked.cell();
        let station_id = self.station_of_dropoff[&dropoff_cell];
        let Some(entry) = self.available.take(peeked.robot()) else {
            return Ok(false);
        };
        let tote_id = entry.tote.as_ref().expect("peeked with a tote").tote_id;
        let at = entry.action_ready_at;

        // Board FIRST, and put the HTM back untouched if the belt refuses. A
        // full loop is back-pressure: the tote stays on the bot, the bot stays
        // available holding it, and the deposit is retried when the belt
        // drains. Emptying the bot before knowing the tote had somewhere to
        // go would lose it.
        let no_conveyor = || PlanError::TaskGraphInvariant {
            context: "LoopDeposit",
            detail: format!("station {station_id:?} has a dropoff cell but no conveyor"),
        };
        let conveyor = self.conveyors.of_station(station_id).ok_or_else(no_conveyor)?;
        let journey = match self.conveyors.board(conveyor, tote_id, at) {
            Ok(journey) => journey,
            Err(refusal) => {
                self.available.put_back(entry)?;
                return match refusal {
                    BoardRefusal::Full => Ok(false),
                    // The station was in `station_of_dropoff`, so it had a belt
                    // when this actor was built. Losing one mid-run is a defect
                    // worth a name, not a retry.
                    BoardRefusal::NoConveyor => Err(no_conveyor()),
                };
            }
        };

        self.bots.commit_task(
            entry.robot(),
            TaskKind::Store,
            &[LegSpec {
                leg: Leg::ConveyorDeposit { station: station_id },
                // No move: the bot is already on the dropoff cell.
                requests: &[],
                action_ready_arrival: at,
                wp_kind: WaypointKind::Deposit,
                expect: BotExpectation { tote: Some(tote_id), ..BotExpectation::empty() },
                carrying_after: None,
            }],
            &self.inventory,
            self.parking.as_ref(),
        )?;
        // Named for the SEAT's station, not the one that took the delivery: the
        // seat is where the tote spends the working part of its journey, and
        // the machine's two stations share the belt that carries it there.
        self.inventory.tote(tote_id)?.move_to(ToteLocation::OnConveyor { station_id: journey.seat }, at)?;

        // The render channel is a SEPARATE event from the location log, and
        // nothing else emits it for a belt. Without these three the tote stays
        // `OnBot` at its last emitted state, so the HTM that just set it down
        // keeps drawing it and leaves the dropoff cell looking laden. The
        // journey states all three times, so none of this is invented.
        self.events.emit(ToteMovedEvent {
            tote_id,
            time: at,
            new_state: ToteVisualState::OnConveyor { cell_id: dropoff_cell.0 },
        });
        if let Some(&seat) = self.seat_cell.get(&journey.seat) {
            self.events.emit(ToteMovedEvent {
                tote_id,
                time: journey.seated_from,
                new_state: ToteVisualState::OnConveyor { cell_id: seat.0 },
            });
        }
        if let Some(&handback) = self.handback_cell.get(&station_id) {
            self.events.emit(ToteMovedEvent {
                tote_id,
                time: journey.available_at,
                new_state: ToteVisualState::OnConveyor { cell_id: handback.0 },
            });
        }

        // The bot is free the instant it sets the tote down: it goes back to
        // the pool, where `vacate_one_loiterer` covers it.
        self.available.register(entry.cell_reservation, at, entry.orientation, None)?;
        Ok(true)
    }
}

#[async_trait(?Send)]
impl Task for LoopDeposit {
    fn create(ctx: &TaskContext) -> Result<Self, PlanError> {
        let navigation = ctx.state::<Navigation>()?;
        let parking = ctx.state::<ParkingRegistry>()?;
        let conveyors = ctx.state::<ConveyorLoops>()?.as_ref().clone();
        let accepts = BeltBoarding::of(&navigation.map.stations);
        let station_of_dropoff = navigation
            .map
            .stations
            .iter()
            .filter(|s| conveyors.serves(s.id))
            .filter_map(|s| s.dropoff_cell.map(|cell| (cell, s.id)))
            .collect();
        let seat_cell =
            navigation.map.stations.iter().filter(|s| conveyors.serves(s.id)).map(|s| (s.id, s.position)).collect();
        let handback_cell = navigation
            .map
            .stations
            .iter()
            .filter(|s| conveyors.serves(s.id))
            .filter_map(|s| s.collection_cell.map(|cell| (s.id, cell)))
            .collect();
        Ok(Self {
            navigation,
            bots: ctx.state::<BotService>()?.as_ref().clone(),
            inventory: ctx.state::<InventoryService>()?.as_ref().clone(),
            available: ctx.state::<AvailableHtmRegistry>()?.as_ref().clone(),
            parking: (parking.candidate_count() > 0).then(|| parking.as_ref().clone()),
            events: ctx.state::<EventJournal>()?.as_ref().clone(),
            conveyors,
            accepts,
            station_of_dropoff,
            seat_cell,
            handback_cell,
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
        let _ = &ctx;
        // A map with no belts runs no deposits: every document but DFW6's.
        if self.station_of_dropoff.is_empty() {
            return Ok(());
        }
        loop {
            let fleet_seen = self.available.version().seen();
            let belt_seen = self.conveyors.version().seen();

            let mut progress = self.vacate_one_loiterer().await;
            progress |= self.deposit_one()?;
            if progress {
                continue;
            }

            select! {
                biased;
                _ = self.available.version().past(fleet_seen) => {}
                // A take drains the belt, unblocking a Full refusal.
                _ = self.conveyors.version().past(belt_seen) => {}
            }
        }
    }
}
