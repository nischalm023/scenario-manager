use super::*;
use crate::model::conveyance::{InventoryPresentation, RetainedHtm};
use crate::model::conveyor::ConveyorId;
use crate::model::{RobotId, ToteId};

fn t(seconds: u64) -> SimTime {
    SimTime::seconds(seconds)
}

fn approach(station: u32, travel: u64) -> PresentationApproach {
    PresentationApproach {
        station: StationId(station),
        travel: SimDuration::seconds(travel),
        available_at: Some(SimTime::ZERO),
    }
}

fn board(service: &Conveyance, id: u32) -> (ConveyedTote, RetainedHtm) {
    service.try_admit(ToteId(id), RobotId(id), SimTime::ZERO).unwrap().unwrap().board(SimTime::ZERO).unwrap()
}

fn finish(source: &InventoryPresentation, retained: &RetainedHtm, end: SimTime) {
    source.clear(end, end).unwrap();
    source.handle.tote.collect(retained, end).unwrap();
}

#[test]
fn later_ready_intent_cannot_claim_a_side_ahead_of_known_earlier_work() {
    let service = Conveyance::new(ConveyorId(0), 2, 2, [StationId(1)]).unwrap();
    let (late, late_htm) = board(&service, 1);
    let (early, early_htm) = board(&service, 2);
    let late_intent = service.request_presentation(&late, t(100), t(0), vec![approach(1, 90)]).unwrap();
    let early_intent = service.request_presentation(&early, t(50), t(0), vec![approach(1, 5)]).unwrap();
    assert_eq!(late_intent.quote(t(0)), None, "a T10 departure must not pin the clock ahead of the T45 winner");
    let quote = early_intent.quote(t(0)).unwrap();
    assert_eq!(quote.depart_at, t(45));
    assert_eq!(service.occupancy_at(t(500)).presentations, 0, "intentions consume no slot capacity");
    let mut claim = early_intent.accept(quote, t(45)).unwrap();
    let source = claim.present(claim.at()).unwrap();
    finish(&source, &early_htm, t(60));
    let quote = late_intent.quote(t(60)).unwrap();
    assert_eq!(quote.depart_at, t(60), "the later request cannot backdate its delayed approach");
    let mut claim = late_intent.accept(quote, t(60)).unwrap();
    let source = claim.present(claim.at()).unwrap();
    let mut pick =
        service.try_pair(&source, ToteId(100), RobotId(100), t(150), t(60), SimDuration::seconds(10)).unwrap();
    assert_eq!(pick.commit().unwrap().start, t(150), "the long approach's real arrival governs picking");
    finish(&source, &late_htm, t(160));
    assert_eq!(service.unfinished(), 0);
    assert!(service.abandoned_custody().is_empty());
}

#[test]
fn blocked_target_side_does_not_hide_an_available_alternative() {
    let service = Conveyance::new(ConveyorId(0), 1, 1, [StationId(1), StationId(2)]).unwrap();
    let (tote, retained) = board(&service, 1);
    let blocked = PresentationApproach { available_at: None, ..approach(1, 5) };
    let intent = service.request_presentation(&tote, t(50), t(0), vec![blocked, approach(2, 10)]).unwrap();
    let quote = intent.quote(t(0)).unwrap();
    assert_eq!(quote.station, StationId(2));
    assert_eq!(quote.depart_at, t(40));
    let mut claim = intent.accept(quote, t(40)).unwrap();
    let source = claim.present(claim.at()).unwrap();
    assert_eq!(service.occupancy_at(t(49)).presentations, 0);
    assert_eq!(service.occupancy_at(t(50)).presentations, 1);
    let mut pick =
        service.try_pair(&source, ToteId(100), RobotId(100), t(65), t(40), SimDuration::seconds(10)).unwrap();
    assert_eq!(pick.commit().unwrap().start, t(65), "traffic-delayed arrival must not pick at its T50 estimate");
    finish(&source, &retained, t(75));
    assert!(service.abandoned_custody().is_empty());
}

#[test]
fn abandoned_intention_and_uncommitted_claim_release_selection_without_releasing_the_tote() {
    let service = Conveyance::new(ConveyorId(0), 2, 2, [StationId(1)]).unwrap();
    let (first, first_htm) = board(&service, 1);
    let (second, second_htm) = board(&service, 2);
    let first_intent = service.request_presentation(&first, t(50), t(0), vec![approach(1, 10)]).unwrap();
    let second_intent = service.request_presentation(&second, t(100), t(0), vec![approach(1, 10)]).unwrap();
    assert!(second_intent.quote(t(0)).is_none());
    drop(first_intent);
    let quote = second_intent.quote(t(0)).unwrap();
    let claim = second_intent.accept(quote, t(90)).unwrap();
    assert_eq!(service.occupancy_at(t(100)).presentations, 1);
    drop(claim);
    assert_eq!(service.occupancy_at(t(100)).presentations, 0);
    assert_eq!(service.occupancy_at(t(100)).conveyor, 2, "dropping intent never invents a physical collection");
    assert_eq!(first.phase(), TotePhase::AwaitingPresentation);
    assert_eq!(second.phase(), TotePhase::AwaitingPresentation);
    for (tote, htm) in [(&first, &first_htm), (&second, &second_htm)] {
        let mut claim = service.try_present(tote, StationId(1), t(100)).unwrap().unwrap();
        let source = claim.present(claim.at()).unwrap();
        finish(&source, htm, t(100));
    }
    assert_eq!(service.unfinished(), 0);
    assert!(service.abandoned_custody().is_empty());
}
