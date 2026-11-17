use std::fmt;

use desgraph::{CellId, RelayId, ZoneId};

use crate::model::ids::{RackSlot, TaskId, ToteId};
use crate::model::inventory::InventoryService;
use crate::model::location::ToteDesignation;
use crate::model::location::{ToteLocation, ToteType};
use crate::model::relay::*;
use crate::model::{RackId, SimTime};
use crate::scheduler::task::proposal::{AcceptError, Proposal, ProposalId, ProposedReservation, accept};

use super::helpers::*;

fn owner(task: u32, attempt: u32) -> ProposalId {
    ProposalId { task: TaskId(task), attempt }
}

fn seeded() -> (RelayService, InventoryService) {
    let (svc, inv) = wired_service();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )
    .expect("seed tote");
    (svc, inv)
}

fn craft(svc: &RelayService, who: ProposalId) -> ProposedRetrieve {
    svc.propose_open_retrieve(
        ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
        CellId(15),
        SimTime::ZERO,
        None,
        who,
    )
    .expect("craftable world")
}

#[test]
fn craft_registers_and_drop_deregisters() {
    let (svc, _inv) = seeded();
    let a = owner(1, 0);
    let tentative = craft(&svc, a);
    let relay = tentative.selection.relay_id;

    let fillers = svc.retrieve_fillers(relay);
    assert_eq!(fillers, vec![(a, tentative.selection.start)]);
    assert_eq!(svc.unfinalized_count(), 0, "a tentative consumes no slot capacity");

    drop(tentative);
    assert!(svc.retrieve_fillers(relay).is_empty(), "dropping the tentative deregisters it");
}

#[test]
fn eager_slot_commit_sweeps_relay_rivals() {
    let (svc, _inv) = seeded();
    let (a, b) = (owner(1, 0), owner(2, 0));
    let ta = craft(&svc, a);
    let tb = craft(&svc, b);
    let relay = ta.selection.relay_id;
    assert_eq!(tb.selection.relay_id, relay, "identical crafts rank the same relay");
    assert_eq!(svc.retrieve_fillers(relay).len(), 2, "overlapping tentatives ARE the pool");

    let eager = svc
        .reserve_relay_for_retrieve(
            ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
            CellId(15),
            SimTime::ZERO,
        )
        .expect("eager commit");
    assert!(!ta.is_valid(), "actual landed on the tentative's relay");
    assert!(!tb.is_valid());
    assert!(svc.retrieve_fillers(relay).is_empty(), "poisoned tentatives leave the pool");
    assert!(ProposedReservation::poisoned(&ta), "the scheduler reads the poison through the proposal surface");
    assert!(ProposedReservation::poisoned(&tb));
    eager.finalize(SimTime::ms(100)).expect("cleanup");
}

#[test]
fn last_zone_headroom_invalidates_quotes_on_other_relays() {
    let (svc, _inv) = seeded();
    let tentative = craft(&svc, owner(1, 0));
    let rivals: Vec<_> = (1..=4).map(RelayId).filter(|id| *id != tentative.selection.relay_id).collect();
    let mut selection = tentative.selection.clone();
    selection.relay_id = rivals[0];
    let first = svc.reserve_selected_relay_for_retrieve(&selection).unwrap();
    assert!(!ProposedReservation::poisoned(&tentative), "one inbound place remains in this four-relay zone");
    assert!(ProposedReservation::admissible(&tentative));

    selection.relay_id = rivals[1];
    let second = svc.reserve_selected_relay_for_retrieve(&selection).unwrap();
    assert!(!svc.can_admit_retrieve(ZoneId(1)));
    assert!(ProposedReservation::poisoned(&tentative), "the old quote must leave the scheduler frontier");
    assert!(svc.retrieve_fillers(tentative.selection.relay_id).is_empty());
    assert!(
        svc.propose_open_retrieve(
            ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
            CellId(15),
            SimTime::ZERO,
            None,
            owner(1, 1),
        )
        .is_err(),
        "the replacement waits for headroom instead of retaining its old start"
    );

    first.finalize(SimTime::ms(100)).unwrap();
    let replacement = craft(&svc, owner(1, 2));
    assert!(ProposedReservation::admissible(&replacement));
    second.finalize(SimTime::ms(200)).unwrap();
}

#[test]
fn accept_refuses_a_poisoned_tentative() {
    let (svc, _inv) = seeded();
    let mut ta = craft(&svc, owner(1, 0));
    let eager = svc
        .reserve_relay_for_retrieve(
            ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
            CellId(15),
            SimTime::ZERO,
        )
        .expect("eager rival");
    assert!(!ta.is_valid());

    let err = match ta.accept() {
        Ok(()) => panic!("a poisoned tentative must refuse acceptance"),
        Err(err) => err,
    };
    assert!(matches!(err, AcceptError::Poisoned { .. }));
    eager.finalize(SimTime::ms(100)).expect("cleanup");
}

#[test]
fn accept_writes_the_slot_and_claims_the_tote() {
    let (svc, inv) = seeded();
    let a = owner(1, 0);
    let mut ta = craft(&svc, a);
    let relay = ta.selection.relay_id;
    let start = ta.selection.start;

    let tb = craft(&svc, owner(2, 0));

    ta.accept().expect("valid tentative accepts");
    assert_eq!(svc.unfinalized_count(), 1, "acceptance writes the slot actual");
    assert!(!tb.is_valid(), "the accepted tote claim poisons its rivals");
    assert_eq!(
        inv.tote(ToteId(1)).expect("tote").location_at(start),
        ToteLocation::Reserved { rack_id: RackId(1), z_index: 0 },
        "acceptance claims the tote"
    );

    let (reservation, rack_slot) = ta.into_accepted().expect("accepted state consumes");
    assert_eq!(rack_slot, RackSlot { rack_id: RackId(1), level: 0 });
    assert_eq!(reservation.relay_id, relay);
    assert_eq!(reservation.recommendation, ToteId(1));
    reservation.finalize(SimTime::ms(100)).expect("the accepted reservation is the real thing");
    assert_eq!(svc.unfinalized_count(), 0);
}

#[test]
fn dropping_an_accepted_tentative_releases_the_slot_and_leaves_the_tote_claimed() {
    let (svc, inv) = seeded();
    let mut ta = craft(&svc, owner(1, 0));
    let start = ta.selection.start;

    ta.accept().expect("accepts");
    assert_eq!(svc.unfinalized_count(), 1);
    drop(ta);
    assert_eq!(svc.unfinalized_count(), 0, "drop releases the accepted slot");
    assert_eq!(
        inv.tote(ToteId(1)).expect("tote").location_at(start),
        ToteLocation::Reserved { rack_id: RackId(1), z_index: 0 },
        "the tote stays claimed, exactly as an aborted chain leaves it"
    );
}

#[test]
fn fillable_relays_lists_free_relays_with_pools() {
    let (svc, _inv) = seeded();
    let ta = craft(&svc, owner(1, 0));
    let relay = ta.selection.relay_id;

    let fillable = svc.retrieve_fillable_relays();
    assert_eq!(fillable.len(), 1);
    assert_eq!(fillable[0].0, relay);

    let eager = svc
        .reserve_relay_for_retrieve(
            ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
            CellId(15),
            SimTime::ZERO,
        )
        .expect("eager commit");
    assert!(svc.retrieve_fillable_relays().is_empty());
    eager.finalize(SimTime::ms(100)).expect("cleanup");
    drop(ta);
}

#[test]
fn accept_transaction_converts_a_real_tentative_in_place() {
    struct OneShot {
        tentatives: Vec<ProposedRetrieve>,
    }
    impl fmt::Display for OneShot {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "OneShot")
        }
    }
    impl Proposal for OneShot {
        fn reservations(&mut self) -> Vec<&mut dyn ProposedReservation> {
            self.tentatives.iter_mut().map(|t| t as &mut dyn ProposedReservation).collect()
        }
    }

    let (svc, inv) = seeded();
    let ta = craft(&svc, owner(7, 0));
    let mut proposal = OneShot { tentatives: vec![ta] };

    accept(&mut proposal).expect("valid tentative accepts");
    let (reservation, _rack_slot) =
        proposal.tentatives.pop().expect("one tentative").into_accepted().expect("accepted in place");
    assert_eq!(
        inv.tote(ToteId(1)).expect("tote").location_at(reservation.start),
        ToteLocation::Reserved { rack_id: RackId(1), z_index: 0 },
    );
    reservation.finalize(SimTime::ms(200)).expect("accepted reservation finalizes");
}
