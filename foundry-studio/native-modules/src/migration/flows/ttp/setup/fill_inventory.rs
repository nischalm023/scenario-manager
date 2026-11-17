//! Seed initial inventory and explicitly configured empty order trays. Physical
//! types are assigned before registration so every index observes the same type.

use rustc_hash::FxHashSet;
use serde_json::Value;

use crate::composition::InventoryTaskConfig;
use crate::composition::resources::InventorySeed;
use crate::events::{EventJournal, SimEvent, ToteCreatedEvent, ToteInitialStatesEvent};
use crate::model::ToteLocation;
use crate::model::ToteType;
use crate::model::factories::world_inputs;
use crate::model::inventory::InventoryService;
use crate::observer::time_slice::ToteVisualState;
use crate::prelude::*;
use crate::task;
use crate::task::{TaskContext, invalid, parse_config};

#[task]
#[derive(Display)]
#[display("FillInventory")]
pub struct FillInventory {
    inventory: InventoryService,
    seed: Arc<InventorySeed>,
    events: EventJournal,
    initial_visuals: ToteInitialStatesEvent,
    tray_ids: Vec<ToteId>,
}

#[async_trait(?Send)]
impl Task for FillInventory {
    fn create(ctx: &TaskContext) -> Result<Self, PlanError> {
        let input = world_inputs(ctx.states())?;
        let seed: Arc<InventorySeed> = ctx.states().configuration()?;
        let order_trays = ctx.config::<InventoryTaskConfig>()?.order_trays;
        let nonempty: FxHashSet<_> =
            seed.contents.iter().filter(|(_, _, quantity)| *quantity > 0).map(|(_, tote_id, _)| *tote_id).collect();
        let mut tray_ids: Vec<_> = seed
            .placements
            .iter()
            .filter(|placement| {
                placement.designation == ToteDesignation::Order && !nonempty.contains(&placement.tote_id)
            })
            .map(|placement| placement.tote_id)
            .collect();
        tray_ids.sort_unstable();
        tray_ids.dedup();
        if order_trays > tray_ids.len() as u64 {
            return Err(invalid(format!(
                "FillInventory.order_trays={order_trays} exceeds the {} seeded empty order containers",
                tray_ids.len()
            )));
        }
        tray_ids.truncate(order_trays as usize);
        let totes = seed
            .placements
            .iter()
            .map(|placement| {
                let state = match placement.location {
                    ToteLocation::InRack { rack_id, z_index } => ToteVisualState::InRack {
                        cell_id: input.data.rack_cells.get(&rack_id).map(|cell| cell.0).unwrap_or(0),
                        level: z_index,
                    },
                    _ => ToteVisualState::InRack { cell_id: 0, level: 0 },
                };
                (placement.tote_id.0, state)
            })
            .collect();
        let order_totes = seed
            .placements
            .iter()
            .filter(|placement| placement.designation == ToteDesignation::Order)
            .map(|placement| placement.tote_id.0)
            .collect();
        Ok(Self {
            inventory: ctx.state::<InventoryService>()?.as_ref().clone(),
            seed,
            events: ctx.state::<EventJournal>()?.as_ref().clone(),
            initial_visuals: ToteInitialStatesEvent { totes, order_totes },
            tray_ids,
        })
    }

    fn validate_config(config: &Value) -> Result<(), PlanError> {
        let params: InventoryTaskConfig = parse_config(config)?;
        if params.preload_fraction.is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value)) {
            return Err(invalid("FillInventory.preload_fraction must be between 0 and 1"));
        }
        Ok(())
    }

    type Proposal = EmptyProposal;
    type Schedule = ();
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<(), PlanError> {
        for placement in &self.seed.placements {
            self.inventory.register_tote(
                placement.tote_id,
                if self.tray_ids.binary_search(&placement.tote_id).is_ok() { ToteType::Tray } else { ToteType::Tote },
                placement.designation,
                placement.location,
            )?;
        }
        for &(sku, tote_id, qty) in &self.seed.contents {
            if let Ok(tote) = self.inventory.tote(tote_id) {
                tote.add_item(sku, qty, 0)?;
            }
        }
        self.inventory.seed_inventory_from_totes();
        self.events.emit(SimEvent::ToteInitialStates(self.initial_visuals.clone()));
        for &(id, ref state) in &self.initial_visuals.totes {
            if self.tray_ids.binary_search(&ToteId(id)).is_ok() {
                self.events.emit(ToteCreatedEvent {
                    tote_id: ToteId(id),
                    tote_type: ToteType::Tray,
                    designation: ToteDesignation::Order,
                    initial_state: state.clone(),
                    time: SimTime::ZERO,
                });
            }
        }
        Ok(())
    }
}
