//! Inventory custody follows completed VTM fork operations, including relay
//! waiting while the VTM is already free to transport another tote.

use std::sync::{Arc, mpsc};

use desgraph::CellId;

use crate::config::VtmConfig;
use crate::events::{EventJournal, SimEvent, ToteSource, VtmCycleDirection, VtmOperationKind};
use crate::flows::ttp::handoff::ToteAtRelay;
use crate::flows::ttp::task::{VtmForward, VtmReturn};
use crate::model::relay::tests::helpers::{build_test_env, empty_htm_registry, test_navigation};
use crate::model::relay::{RelayService, ToteSearchCriteria};
use crate::model::reservation::oracle::GhostOracle;
use crate::model::{ToteDesignation, ToteLocation, ToteType};
use crate::prelude::*;
use crate::scheduler::Scheduler;

#[derive(Display)]
#[display("DelayHtmReceiver")]
struct DelayHtmReceiver {
    relays: RelayService,
    inventory: InventoryService,
    events: EventJournal,
}

#[async_trait(?Send)]
impl Task for DelayHtmReceiver {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        let mut deliveries = Vec::new();
        for id in [1, 2] {
            let reservation = self.relays.reserve_relay_for_retrieve(
                ToteSearchCriteria::ContainsSku { sku: SkuId(id) },
                CellId(15),
                SimTime::ZERO,
            )?;
            assert_eq!(reservation.recommendation, ToteId(id));
            let delivery = ctx
                .add_task(
                    VtmForward::new(
                        RackSlot { rack_id: RackId(id), level: 0 },
                        reservation,
                        None,
                        self.relays.clone(),
                        self.inventory.clone(),
                        self.events.clone(),
                    )
                    .with_source(ToteSource::Pick),
                )
                .await;
            deliveries.push(delivery);
        }
        assert_ne!(deliveries[0].reservation.relay_id, deliveries[1].reservation.relay_id);
        assert!(deliveries[0].deposit_time < deliveries[1].deposit_time);
        let pickup = deliveries[1].deposit_time + SimDuration::seconds(60);
        // Model the receiving boundary only: these tests exercise real VTM
        // tasks, with both HTMs deliberately unavailable until this instant.
        for delivery in deliveries {
            let tote_id = delivery.reservation.recommendation;
            self.inventory
                .tote(tote_id)?
                .move_to(ToteLocation::WithRobot { robot_id: RobotId(100 + tote_id.0) }, pickup)?;
            delivery.reservation.finalize(pickup)?;
        }

        let deposit = self
            .relays
            .reserve_relay_for_deposit(
                CellId(15),
                Default::default(),
                pickup + SimDuration::seconds(10),
                RobotId(101),
                None,
            )
            .await?;
        let relay_id = deposit.relay_id;
        let deposit_time = deposit.start;
        self.inventory.tote(ToteId(1))?.move_to(ToteLocation::AtRelay { relay_id }, deposit_time)?;
        let handoff = ToteAtRelay {
            tote_id: ToteId(1),
            relay_id,
            rack_slot: deposit.recommendation,
            reservation: deposit,
            deposit_time,
            presentation_id: None,
            tote_source: ToteSource::Pick,
        };
        ctx.add_task(VtmReturn::new(
            handoff,
            self.relays.vtm_service.clone(),
            self.inventory.clone(),
            self.events.clone(),
        ))
        .await;
        Ok(())
    }
}

#[test]
fn vtm_custody_tracks_forks_and_relay_waiting_during_successive_work() {
    let (data, inventory, pathfinder) =
        build_test_env(&[(1, &[(1, 10), (2, 20), (3, 30), (4, 40)])], &[(1, 12, 1), (2, 22, 1)]);
    let available = empty_htm_registry(&data, &inventory);
    let relays = RelayService::new(
        &data,
        inventory.clone(),
        test_navigation(&data, pathfinder, Arc::new(GhostOracle)),
        &available,
        &VtmConfig::default(),
    )
    .unwrap();
    for id in [1, 2] {
        inventory
            .register_tote(
                ToteId(id),
                ToteType::Tote,
                ToteDesignation::Inventory,
                ToteLocation::InRack { rack_id: RackId(id), z_index: 0 },
            )
            .unwrap();
        inventory.tote(ToteId(id)).unwrap().add_item(SkuId(id), 1, 0).unwrap();
    }
    let (tx, rx) = mpsc::channel();
    let mut scheduler = Scheduler::new();
    scheduler.add_task(DelayHtmReceiver {
        relays: relays.clone(),
        inventory: inventory.clone(),
        events: EventJournal::new(Some(Arc::new(tx))),
    });
    scheduler.run().unwrap();
    assert_eq!(relays.unfinalized_count(), 0);
    let events: Vec<_> = rx.try_iter().collect();
    let cycles: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::VtmCycle(cycle) => Some(cycle),
            _ => None,
        })
        .collect();
    assert_eq!(cycles.len(), 3);
    assert!(cycles.iter().all(|cycle| cycle.vtm_id == cycles[0].vtm_id));
    let before = SimDuration::ms(1);
    let mut loaded_intervals = Vec::new();
    for cycle in &cycles {
        let (pickup_kind, drop_kind) = match cycle.direction {
            VtmCycleDirection::Forward => (VtmOperationKind::PickupStorage, VtmOperationKind::DropRelay),
            VtmCycleDirection::Return => (VtmOperationKind::PickupRelay, VtmOperationKind::DropStorage),
        };
        let operation_at = |kind| {
            events
                .iter()
                .find_map(|event| match event {
                    SimEvent::VtmOperation(operation)
                        if operation.vtm_cycle_id == cycle.vtm_cycle_id && operation.kind == kind =>
                    {
                        assert_eq!(operation.tote_id, cycle.tote_id);
                        Some(SimTime::ms(operation.sim_time_ms))
                    }
                    _ => None,
                })
                .unwrap()
        };
        let pickup = operation_at(pickup_kind);
        let drop = operation_at(drop_kind);
        let tote = inventory.tote(cycle.tote_id).unwrap();
        let rack = ToteLocation::InRack { rack_id: cycle.rack_id, z_index: 0 };
        let relay = ToteLocation::AtRelay { relay_id: cycle.relay_id };
        let robot = ToteLocation::WithRobot { robot_id: cycle.vtm_id };
        let (source, destination) = match cycle.direction {
            VtmCycleDirection::Forward => (rack, relay),
            VtmCycleDirection::Return => (relay, rack),
        };
        assert_eq!(tote.location_at(pickup - before), source, "custody must remain at the source through fork travel");
        assert!(tote.location_is_through(pickup, drop - before, robot), "VTM custody must cover its entire loaded leg");
        assert_eq!(tote.location_at(drop), destination, "completed deposit must end VTM custody");
        assert_eq!(tote.contents_at(drop), vec![(SkuId(cycle.tote_id.0), 1)]);
        for at in [pickup - before, pickup, drop - before, drop] {
            let on_vtm = [ToteId(1), ToteId(2)]
                .into_iter()
                .filter(|&id| inventory.tote(id).unwrap().location_at(at) == robot)
                .count();
            assert!(on_vtm <= 1, "the VTM is attributed multiple totes at {at:?}");
        }
        loaded_intervals.push((pickup, drop, cycle.tote_id));
    }
    loaded_intervals.sort_by_key(|(start, _, _)| *start);
    for pair in loaded_intervals.windows(2) {
        assert!(pair[0].1 <= pair[1].0);
    }
    let (second_pickup, second_drop, second_tote) = loaded_intervals[1];
    let first = cycles
        .iter()
        .find(|cycle| cycle.tote_id == ToteId(1) && cycle.direction == VtmCycleDirection::Forward)
        .unwrap();
    let first_tote = inventory.tote(ToteId(1)).unwrap();
    assert!(
        first_tote.location_is_through(
            loaded_intervals[0].1,
            second_drop + SimDuration::seconds(59),
            ToteLocation::AtRelay { relay_id: first.relay_id }
        ),
        "first tote waits at its relay while the VTM transports the second"
    );
    assert_eq!(inventory.tote_on_robot(first.vtm_id, second_pickup).map(|(id, _)| id), Some(second_tote));
    assert!(inventory.tote_on_robot(first.vtm_id, second_drop).is_none());
}
