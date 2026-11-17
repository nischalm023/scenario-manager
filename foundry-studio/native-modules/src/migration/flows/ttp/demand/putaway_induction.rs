//! `PutawayInduction`: the demand half of putaway — it owns the pending-lot
//! sequence (SKU, quantity, net-time) and the per-station induction pacing, and
//! spends them by filling empty totes that [`super::feed_putaway_stations`] has
//! staged at the stations. One fill per iteration; the task completes once
//! the queue is exhausted.
//! A fill fixes its release (simtime) when it is committed (scheduler time), so
//! it is committed only once a deposit relay is reserved for its tote's return
//! at that release. Until one can be reserved, no fill is committed, and the
//! empty tote stands on the station's cell (simtime) until the fill committed
//! then begins.

use super::feed_putaway_stations::*;
use crate::composition::InductionTaskConfig;
use crate::composition::policy;
use crate::composition::resources::inputs;
use crate::flows::ttp::task::clearance::{Attempt, ClearPresentation, ReturnRelays, Unparkable};
use crate::model::bot::plan::{BotExpectation, Leg, TaskKind, WaypointKind};
use crate::model::bot::{BotService, LegSpec};
use crate::model::inventory::InventoryService;
use crate::model::relay::RelayReservation;
use crate::task;
use crate::task::TaskContext;
use crate::task::parse_config;
use crate::{events::*, model::htm::registry::*, prelude::*, utils::temporal_counter::*};
use desgraph::{CellId, RobotType, StationId};
use rustc_hash::FxHashMap;
use serde_json::Value;
use tokio::select;

#[derive(Display)]
#[display("PutawayInduction")]
#[task]
pub struct PutawayInduction {
    inventory: InventoryService,
    bots: BotService,
    available: AvailableHtmRegistry,
    events: EventJournal,
    supply: Arc<PutawaySupply>,
    arrival_queue: ArrivalQueue,
    station_pace: FxHashMap<StationId, (CellId, TemporalCounter)>,
    pacing_interval: SimDuration,
    return_relays: ReturnRelays,
}

impl PutawayInduction {
    // Emptiness is read from live inventory at the entry's own action-ready
    // time — never from the tote snapshot the registry froze at registration.
    fn presenting_empty(&self, htm: &AvailableHtm, cell: CellId) -> bool {
        htm.cell() == cell
            && htm.tote.as_ref().is_some_and(|carried| {
                self.inventory.tote(carried.tote_id).is_ok_and(|tote| tote.is_empty_at(htm.action_ready_at))
            })
    }

    fn select_fill(&self, net_floor: SimTime) -> Option<(AvailableHtm, StationId, SimTime)> {
        self.station_pace
            .iter()
            .flat_map(|(&station_id, (cell, fills))| {
                let pace_floor = match fills.frontier() {
                    (_, 0) => SimTime::ZERO,
                    (last_fill, _) => last_fill,
                };
                self.available.peek_matching(|htm| self.presenting_empty(htm, *cell)).into_iter().map(move |htm| {
                    (
                        htm.action_ready_at.max(pace_floor).max(net_floor) + self.pacing_interval,
                        station_id,
                        htm.robot(),
                        htm,
                    )
                })
            })
            .min_by_key(|&(fill_at, station_id, robot, _)| (fill_at, station_id, robot))
            .map(|(fill_at, station_id, _, htm)| (htm, station_id, fill_at))
    }

    /// Commit the fill and hand its clearance `return_relay`, or give the
    /// relay back if the fill cannot be committed.
    #[allow(clippy::too_many_arguments)]
    fn fill(
        &mut self,
        entry: AvailableHtm,
        station_id: StationId,
        sku: SkuId,
        qty: u32,
        fill_at: SimTime,
        return_relay: RelayReservation<RackSlot>,
        ctx: &SchedCtx,
    ) -> Result<(), PlanError> {
        match self.commit_fill(entry, station_id, sku, qty, fill_at) {
            Ok(filled) => ClearPresentation::spawn(ctx, filled, station_id, ToteSource::Putaway, return_relay),
            Err(error) => {
                return_relay.cancel();
                Err(error)
            }
        }
    }

    fn commit_fill(
        &mut self,
        entry: AvailableHtm,
        station_id: StationId,
        sku: SkuId,
        qty: u32,
        fill_at: SimTime,
    ) -> Result<AvailableHtm, PlanError> {
        let invariant = |detail: String| PlanError::TaskGraphInvariant { context: "PutawayInduction", detail };
        let htm_id = entry.robot();
        let tote_id = entry
            .tote
            .as_ref()
            .ok_or_else(|| invariant(format!("presenting empty {htm_id:?} carries no tote")))?
            .tote_id;

        self.bots.commit_task(
            htm_id,
            TaskKind::Putaway,
            &[LegSpec {
                leg: Leg::StationWork { cell: entry.cell() },
                requests: &[],
                action_ready_arrival: fill_at,
                wp_kind: WaypointKind::Present,
                expect: BotExpectation::with_tote(tote_id),
                carrying_after: Some(tote_id),
            }],
            &self.inventory,
            None,
        )?;
        self.inventory.tote(tote_id)?.add_item(sku, qty, fill_at.as_ms())?;

        let taken = self
            .available
            .take(htm_id)
            .ok_or_else(|| invariant(format!("presenting empty {htm_id:?} left the registry between peek and fill")))?;
        let presentation_id = taken.tote.as_ref().and_then(|carried| carried.presentation_id);
        // Re-registering at `fill_at` re-snapshots the now-filled tote and moves the
        // entry's action-ready time up to the fill, so the clearance spawned below
        // departs no earlier and `finalize()` narrows the open-ended cell hold
        // instead of widening it.
        let filled = self
            .available
            .register(taken.cell_reservation, fill_at, taken.orientation, presentation_id)
            .map_err(|e| invariant(format!("re-register presenting empty {htm_id:?} after fill: {e}")))?;

        // Putaway's CycleComplete fires at the END of the deposit dwell (pick's fires at its start).
        self.events.emit(SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: htm_id,
            robot_type: RobotType::Htm,
            kind: CycleKind::Putaway { sku, qty },
            station_id,
            sim_time_ms: fill_at.as_ms(),
            dwell_ms: self.pacing_interval.as_ms(),
            arrival_source: Some(StationArrivalSource::PutawayDirect),
            dc_id: None,
            presentation_id,
            tote_presentation: None,
        }));

        self.station_pace
            .get_mut(&station_id)
            .ok_or_else(|| invariant(format!("station {station_id:?} has no pacing clock")))?
            .1
            .add_open(fill_at);

        crate::trace_event!(
            commit,
            putaway_fill,
            station = station_id,
            htm = htm_id,
            tote = tote_id,
            sku = sku,
            fill_at = fill_at,
        );
        Ok(filled)
    }
}

#[async_trait(?Send)]
impl Task for PutawayInduction {
    fn create(ctx: &TaskContext) -> Result<Self, PlanError> {
        let params: InductionTaskConfig = ctx.config()?;
        let input = inputs(ctx.states())?;
        let supply: Arc<PutawaySupply> = ctx.state()?;
        let station_pace = supply.station_cells().map(|(id, cell)| (id, (cell, TemporalCounter::new()))).collect();
        Ok(Self {
            inventory: ctx.state::<InventoryService>()?.as_ref().clone(),
            bots: ctx.state::<BotService>()?.as_ref().clone(),
            available: ctx.state::<AvailableHtmRegistry>()?.as_ref().clone(),
            events: ctx.state::<EventJournal>()?.as_ref().clone(),
            supply,
            arrival_queue: ctx.state::<Arrivals>()?.0.clone(),
            station_pace,
            pacing_interval: policy::induction_period(&input.config, &params)?,
            return_relays: ReturnRelays::new(ctx)?,
        })
    }

    fn validate_config(config: &Value) -> Result<(), PlanError> {
        parse_config::<InductionTaskConfig>(config).map(|_| ())
    }

    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _owner: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _proposal: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        if self.station_pace.is_empty() {
            return Ok(());
        }
        let mut unparkable = Unparkable::default();
        loop {
            let fleet_seen = self.available.version().seen();
            let inventory_seen = self.inventory.version().seen();
            let supply_seen = self.supply.version().seen();

            let Some((sku, qty, net_time)) = self.arrival_queue.with(|queue| queue.front().copied()) else {
                return Ok(());
            };

            // No fill begins (simtime) behind the barrier: one committed after
            // an await can find the barrier past its HTM's action-ready time.
            let Some((entry, station_id, fill_at)) = self.select_fill(net_time.max(ctx.barrier().clock().value()))
            else {
                // Supply starvation is never quiesced: with lots pending and
                // no fill arriving, this task stays pending and the planner's
                // deadlock alarm reports the wedge with its census.
                select! {
                    biased;
                    _ = self.available.version().past(fleet_seen) => {}
                    _ = self.inventory.version().past(inventory_seen) => {}
                    _ = self.supply.version().past(supply_seen) => {}
                }
                continue;
            };
            let declined = match self.return_relays.attempt(&entry, fill_at, &mut unparkable, &ctx).await? {
                Attempt::Reserved(return_relay) => {
                    self.fill(entry, station_id, sku, qty, fill_at, return_relay, &ctx)?;
                    self.arrival_queue.update(|queue| {
                        queue.pop_front();
                    });
                    continue;
                }
                Attempt::SquatterMoved => continue,
                Attempt::Declined(declined) => declined,
            };
            // Beyond what `next_attempt_due` awaits, a change to the supply
            // wakes the wait.
            select! {
                biased;
                due = self.return_relays.next_attempt_due(declined, fill_at, self.pacing_interval, &ctx) => due?,
                _ = self.supply.version().past(supply_seen) => {}
            }
        }
    }
}

#[cfg(test)]
#[path = "putaway_induction_tests.rs"]
mod tests;
