use super::*;
use std::time::Duration;

const LEFT: StationId = StationId(1);
const RIGHT: StationId = StationId(2);

fn t(time: u64) -> SimTime {
    SimTime::ms(time)
}
fn d(time: u64) -> SimDuration {
    SimDuration::ms(time)
}
fn service(capacity: usize, retained: usize) -> Conveyance {
    Conveyance::new(ConveyorId(0), capacity, retained, [LEFT, RIGHT]).unwrap()
}
fn board(service: &Conveyance, tote: u32, robot: u32, at: u64) -> (ConveyedTote, RetainedHtm) {
    let mut admission = service.try_admit(ToteId(tote), RobotId(robot), t(at)).unwrap().unwrap();
    assert_eq!(admission.at(), t(at));
    admission.board(t(at)).unwrap()
}
fn present(service: &Conveyance, tote: &ConveyedTote, station: StationId, at: u64) -> InventoryPresentation {
    let mut claim = service.try_present(tote, station, t(at)).unwrap().unwrap();
    assert_eq!(claim.at(), t(at));
    claim.present(t(at)).unwrap()
}

#[test]
fn eight_totes_include_presentations_and_returns_until_actual_collection() {
    let service = service(8, 8);
    let visits: Vec<_> = (0..8).map(|i| board(&service, i, i, 0)).collect();
    assert!(service.try_admit(ToteId(8), RobotId(8), t(0)).unwrap().is_none());
    for (i, (tote, _)) in visits.iter().enumerate() {
        let presented = present(&service, tote, LEFT, 10 + i as u64);
        presented.clear(t(11 + i as u64), t(30 + i as u64)).unwrap();
    }
    assert_eq!(service.returning().len(), 8);
    assert!(service.try_admit(ToteId(8), RobotId(8), t(0)).unwrap().is_none());
    visits[6].0.collect(&visits[6].1, t(100)).unwrap();
    let mut ninth = service.try_admit(ToteId(8), RobotId(8), t(0)).unwrap().unwrap();
    assert_eq!(ninth.at(), t(100), "a planned future collection does not free today's capacity");
    let _ninth_custody = ninth.board(t(100)).unwrap();
    visits[2].0.collect(&visits[2].1, t(50)).unwrap();
    let mut tenth = service.try_admit(ToteId(9), RobotId(9), t(0)).unwrap().unwrap();
    assert_eq!(tenth.at(), t(50), "collection planning order does not impose false FIFO timing");
    let _tenth_custody = tenth.board(t(50)).unwrap();
    let history = service.visits();
    let mut times: Vec<_> = history
        .iter()
        .flat_map(|visit| {
            let end = match visit.phase {
                TotePhase::Collected { at, .. } => Some(at),
                _ => None,
            };
            visit.boarded_at.into_iter().chain(end)
        })
        .collect();
    times.sort();
    times.dedup();
    for time in times {
        let aboard = history
            .iter()
            .filter(|visit| {
                visit.boarded_at.is_some_and(|at| at <= time)
                    && !matches!(visit.phase, TotePhase::Collected { at, .. } if at <= time)
            })
            .count();
        assert!(aboard <= 8, "{aboard} physical totes aboard at {time}");
        assert_eq!(service.occupancy_at(time).conveyor, aboard);
        assert!(service.occupancy_at(time).retained_htms <= 8);
    }
}

#[test]
fn retained_htm_capacity_is_required_before_unloading_and_claims_cancel() {
    let service = service(8, 2);
    let first = service.try_admit(ToteId(1), RobotId(1), t(20)).unwrap().unwrap();
    let second = service.try_admit(ToteId(2), RobotId(2), t(40)).unwrap().unwrap();
    assert!(
        service.try_admit(ToteId(3), RobotId(3), t(0)).unwrap().is_none(),
        "an open claim would exceed the future retained-HTM capacity"
    );
    drop(second);
    let third = service.try_admit(ToteId(3), RobotId(3), t(0)).unwrap().unwrap();
    assert_eq!(third.at(), t(0));
    drop(first);
    drop(third);
    assert_eq!(service.occupancy_at(t(100)).conveyor, 0);
    assert_eq!(service.occupancy_at(t(100)).retained_htms, 0);
    assert!(service.visits().is_empty());
    assert!(service.abandoned_custody().is_empty());
}

#[test]
fn two_presentations_share_one_operator_and_clearance_waits_for_pick_end() {
    let service = service(8, 8);
    let (a, _ha) = board(&service, 1, 1, 0);
    let (b, _hb) = board(&service, 2, 2, 0);
    let (c, _hc) = board(&service, 3, 3, 0);
    let left = present(&service, &a, LEFT, 10);
    let right = present(&service, &b, RIGHT, 10);
    assert_eq!(service.occupancy_at(t(10)).presentations, 2);
    assert!(service.try_present(&c, LEFT, t(0)).unwrap().is_none());
    let mut first = service.try_pair(&left, ToteId(101), RobotId(11), t(15), t(0), d(10)).unwrap();
    let first_pair = first.commit().unwrap();
    let mut second = service.try_pair(&right, ToteId(102), RobotId(12), t(18), t(0), d(10)).unwrap();
    assert_eq!((first_pair.start, first_pair.end), (t(15), t(25)));
    assert_eq!((second.interval().start, second.interval().end), (t(25), t(35)));
    assert_eq!(right.clear(t(35), t(40)), Err(ConveyanceError::PendingTransfer));
    second.commit().unwrap();
    assert!(matches!(left.clear(t(24), t(40)), Err(ConveyanceError::TooEarly { .. })));
    left.clear(t(30), t(40)).unwrap();
    right.clear(t(35), t(45)).unwrap();
    let mut third = service.try_present(&c, LEFT, t(0)).unwrap().unwrap();
    assert_eq!(third.at(), t(30), "clearing a future presentation does not free its past");
    let _third = third.present(t(30)).unwrap();
    for time in 0..50 {
        assert!(service.occupancy_at(t(time)).operator <= 1);
        assert!(service.occupancy_at(t(time)).presentations <= 2);
    }
    let pairs = service.transfers();
    assert_eq!(pairs.len(), 2);
    assert!(pairs[0].end <= pairs[1].start);
    assert_eq!((pairs[0].inventory_tote, pairs[0].order_tote), (ToteId(1), ToteId(101)));
}

#[test]
fn presentation_selection_uses_the_free_side_and_keeps_stable_ties() {
    let service = service(8, 8);
    let (a, _ha) = board(&service, 1, 1, 0);
    let (b, _hb) = board(&service, 2, 2, 0);
    let (c, _hc) = board(&service, 3, 3, 0);
    let first = service.try_present_any(&a, [RIGHT, LEFT], t(10)).unwrap().unwrap();
    assert_eq!((first.station(), first.at()), (RIGHT, t(10)), "equal times respect caller preference");
    let second = service.try_present_any(&b, [RIGHT, LEFT], t(10)).unwrap().unwrap();
    assert_eq!(second.station(), LEFT, "an open preferred-side hold must not block the free side");
    assert!(service.try_present_any(&c, [RIGHT, LEFT], t(10)).unwrap().is_none());
    assert_eq!(c.phase(), TotePhase::AwaitingPresentation);
    drop(first);
    let retry = service.try_present_any(&c, [LEFT, RIGHT], t(10)).unwrap().unwrap();
    assert_eq!(retry.station(), RIGHT, "cancellation frees only the selected slot");
    assert_eq!(service.occupancy_at(t(10)).presentations, 2);
}

#[test]
fn presentation_selection_prefers_earliest_capacity_over_requested_side() {
    let service = service(8, 8);
    let (a, _ha) = board(&service, 1, 1, 0);
    let (b, _hb) = board(&service, 2, 2, 0);
    let (c, _hc) = board(&service, 3, 3, 0);
    present(&service, &a, LEFT, 10).clear(t(40), t(50)).unwrap();
    present(&service, &b, RIGHT, 10).clear(t(20), t(30)).unwrap();
    let mut earliest = service.try_present_any(&c, [LEFT, RIGHT], t(0)).unwrap().unwrap();
    assert_eq!((earliest.station(), earliest.at()), (RIGHT, t(20)));
    let actual = earliest.present(t(25)).unwrap();
    assert_eq!((actual.station(), actual.ready_at()), (RIGHT, t(25)));
    assert_eq!(service.occupancy_at(t(24)).presentations, 1);
    assert_eq!(service.occupancy_at(t(25)).presentations, 2);
}

#[test]
fn invalid_eligible_station_does_not_leave_a_partial_claim() {
    let service = service(8, 8);
    let (tote, _htm) = board(&service, 1, 1, 0);
    assert!(matches!(
        service.try_present_any(&tote, [LEFT, StationId(999)], t(10)),
        Err(ConveyanceError::UnknownPresentation(StationId(999)))
    ));
    assert_eq!(tote.phase(), TotePhase::AwaitingPresentation);
    assert_eq!(service.occupancy_at(t(10)).presentations, 0);
    assert!(service.try_present_any(&tote, [LEFT, RIGHT], t(10)).unwrap().is_some());
}

#[test]
fn provisional_presentations_and_operator_claims_restore_their_resources() {
    let service = service(8, 8);
    let (a, _ha) = board(&service, 1, 1, 0);
    let (b, _hb) = board(&service, 2, 2, 0);
    let pending = service.try_present(&a, LEFT, t(10)).unwrap().unwrap();
    assert!(service.try_present(&b, LEFT, t(0)).unwrap().is_none());
    drop(pending);
    assert_eq!(a.phase(), TotePhase::AwaitingPresentation);
    let a_presented = present(&service, &a, LEFT, 10);
    let b_presented = present(&service, &b, RIGHT, 10);
    let pair = service.try_pair(&a_presented, ToteId(100), RobotId(100), t(10), t(0), d(20)).unwrap();
    drop(pair);
    let mut replacement = service.try_pair(&b_presented, ToteId(101), RobotId(101), t(10), t(0), d(20)).unwrap();
    assert_eq!(replacement.interval().start, t(10));
    replacement.commit().unwrap();
    assert_eq!(service.transfers().len(), 1);
    assert!(service.abandoned_custody().is_empty());
}

#[test]
fn stale_presentations_and_duplicate_commits_are_refused() {
    let service = service(8, 8);
    let mut admission = service.try_admit(ToteId(1), RobotId(1), t(10)).unwrap().unwrap();
    assert!(matches!(admission.board(t(9)), Err(ConveyanceError::TooEarly { .. })));
    let (tote, htm) = admission.board(t(20)).unwrap();
    assert!(matches!(admission.board(t(20)), Err(ConveyanceError::SpentClaim)));
    assert_eq!(service.occupancy_at(t(19)).conveyor, 0);
    let mut proposed = service.try_present(&tote, LEFT, t(30)).unwrap().unwrap();
    assert!(matches!(proposed.present(t(29)), Err(ConveyanceError::TooEarly { .. })));
    let held = proposed.present(t(40)).unwrap();
    let mut pair = service.try_pair(&held, ToteId(100), RobotId(100), t(35), t(0), d(5)).unwrap();
    assert_eq!(pair.interval().start, t(40), "actual presentation readiness is binding");
    pair.commit().unwrap();
    assert_eq!(pair.commit(), Err(ConveyanceError::SpentClaim));
    held.clear(t(45), t(50)).unwrap();
    assert!(matches!(
        service.try_pair(&held, ToteId(100), RobotId(100), t(40), t(0), d(5)),
        Err(ConveyanceError::WrongState)
    ));
    assert!(matches!(tote.collect(&htm, t(49)), Err(ConveyanceError::TooEarly { .. })));
    tote.collect(&htm, t(50)).unwrap();
    assert_eq!(tote.collect(&htm, t(60)), Err(ConveyanceError::WrongState));
    assert_eq!(service.unfinished(), 0);
    assert!(service.abandoned_custody().is_empty());
}

#[test]
fn retained_robot_can_collect_a_different_tote_but_only_from_its_machine() {
    let service = service(8, 8);
    let other = Conveyance::new(ConveyorId(1), 8, 8, [LEFT, RIGHT]).unwrap();
    let (a, ha) = board(&service, 1, 1, 0);
    let (b, hb) = board(&service, 2, 2, 0);
    let (_foreign_tote, foreign_robot) = board(&other, 3, 3, 0);
    present(&service, &a, LEFT, 10).clear(t(20), t(30)).unwrap();
    present(&service, &b, RIGHT, 10).clear(t(20), t(25)).unwrap();
    let returning = service.returning();
    assert_eq!(returning.iter().map(|r| r.tote.tote()).collect::<Vec<_>>(), vec![ToteId(2), ToteId(1)]);
    assert_eq!(a.collect(&foreign_robot, t(30)), Err(ConveyanceError::DifferentConveyance));
    a.collect(&hb, t(30)).unwrap();
    b.collect(&ha, t(35)).unwrap();
    assert_eq!(a.phase(), TotePhase::Collected { at: t(30), robot: RobotId(2) });
    assert_eq!(b.phase(), TotePhase::Collected { at: t(35), robot: RobotId(1) });
    assert_eq!(service.unfinished(), 0);
    assert_eq!(service.occupancy_at(t(35)).conveyor, 0);
}

#[test]
fn dropping_committed_custody_records_a_leak_without_freeing_capacity() {
    let service = service(1, 1);
    let (tote, htm) = board(&service, 1, 1, 0);
    let held = present(&service, &tote, LEFT, 10);
    let copied = tote.clone();
    drop(tote);
    assert!(service.abandoned_custody().is_empty());
    drop(held);
    drop(copied);
    drop(htm);
    assert_eq!(
        service.abandoned_custody(),
        vec![
            CustodyLeak::InventoryTote(ToteId(1)),
            CustodyLeak::RetainedHtm(RobotId(1)),
            CustodyLeak::Presentation { tote: ToteId(1), station: LEFT },
        ]
    );
    assert_eq!(
        service.occupancy_at(t(100)),
        Occupancy { conveyor: 1, retained_htms: 1, presentations: 1, operator: 0 }
    );
    assert!(service.try_admit(ToteId(2), RobotId(2), t(100)).unwrap().is_none());
    assert_eq!(service.unfinished(), 2);
}

#[tokio::test]
async fn cancellation_wakes_version_waiters_even_if_they_subscribe_afterwards() {
    let service = service(1, 1);
    let pending = service.try_admit(ToteId(1), RobotId(1), t(20)).unwrap().unwrap();
    let seen = service.version().seen();
    drop(pending);
    tokio::time::timeout(Duration::from_millis(100), service.version().past(seen)).await.unwrap();
    assert!(service.try_admit(ToteId(2), RobotId(2), t(0)).unwrap().is_some());
}

#[test]
fn the_same_tote_and_robot_cannot_start_another_visit_before_their_release() {
    let service = service(8, 8);
    let (tote, htm) = board(&service, 1, 1, 10);
    assert!(matches!(
        service.try_admit(ToteId(1), RobotId(2), t(0)),
        Err(ConveyanceError::ToteAlreadyAdmitted(ToteId(1)))
    ));
    assert!(matches!(
        service.try_admit(ToteId(2), RobotId(1), t(0)),
        Err(ConveyanceError::RobotAlreadyRetained(RobotId(1)))
    ));
    present(&service, &tote, LEFT, 20).clear(t(30), t(40)).unwrap();
    tote.collect(&htm, t(100)).unwrap();
    let mut again = service.try_admit(ToteId(1), RobotId(1), t(0)).unwrap().unwrap();
    assert_eq!(again.at(), t(100));
    let (_tote_again, _htm_again) = again.board(t(110)).unwrap();
    assert_eq!(service.occupancy_at(t(99)).conveyor, 1);
    assert_eq!(service.occupancy_at(t(100)).conveyor, 0);
    assert_eq!(service.occupancy_at(t(110)).conveyor, 1);
    assert_eq!(service.visits().len(), 2);
}

#[test]
fn cancelling_a_return_to_presentation_restores_return_eligibility() {
    let service = service(8, 8);
    let (tote, _htm) = board(&service, 1, 1, 0);
    let prior = present(&service, &tote, LEFT, 10);
    prior.clear(t(20), t(30)).unwrap();
    let pending = service.try_present(&tote, RIGHT, t(0)).unwrap().unwrap();
    assert_eq!(pending.at(), t(30));
    assert!(service.returning().is_empty(), "a reserved presentation cannot also be collected");
    drop(pending);
    assert_eq!(service.returning().len(), 1);
    let current = present(&service, &tote, RIGHT, 35);
    assert!(matches!(
        service.try_pair(&prior, ToteId(100), RobotId(100), t(40), t(0), d(5)),
        Err(ConveyanceError::WrongState)
    ));
    assert_eq!(current.station(), RIGHT);
}

#[test]
fn independent_machines_can_use_their_operators_at_the_same_time() {
    let a = service(8, 8);
    let b = Conveyance::new(ConveyorId(1), 8, 8, [LEFT, RIGHT]).unwrap();
    let (ta, _ha) = board(&a, 1, 1, 0);
    let (tb, _hb) = board(&b, 2, 2, 0);
    let pa = present(&a, &ta, LEFT, 10);
    let pb = present(&b, &tb, LEFT, 10);
    let mut ca = a.try_pair(&pa, ToteId(100), RobotId(100), t(10), t(0), d(10)).unwrap();
    let mut cb = b.try_pair(&pb, ToteId(101), RobotId(101), t(10), t(0), d(10)).unwrap();
    assert_eq!(ca.commit().unwrap().start, cb.commit().unwrap().start);
}

#[test]
fn collection_claim_excludes_other_collectors_and_releases_selection_on_cancel() {
    let service = service(8, 8);
    let (a, ha) = board(&service, 1, 1, 0);
    let (b, hb) = board(&service, 2, 2, 0);
    present(&service, &a, LEFT, 10).clear(t(20), t(30)).unwrap();
    present(&service, &b, RIGHT, 10).clear(t(20), t(35)).unwrap();

    let first = service.try_collect(&a, &hb, t(0)).unwrap().unwrap();
    assert_eq!(first.at(), t(30));
    assert!(service.try_collect(&a, &ha, t(0)).unwrap().is_none(), "one tote cannot have two collectors");
    assert!(service.try_collect(&b, &hb, t(0)).unwrap().is_none(), "one HTM cannot collect two totes");
    assert_eq!(service.returning().iter().map(|tote| tote.tote.tote()).collect::<Vec<_>>(), [ToteId(2)]);
    assert_eq!(a.collect(&ha, t(40)), Err(ConveyanceError::WrongState), "direct collection cannot steal a claim");
    assert!(matches!(service.try_present(&a, LEFT, t(40)), Err(ConveyanceError::WrongState)));
    assert_eq!(service.occupancy_at(t(100)).conveyor, 2, "a route proposal does not release capacity");
    drop(first);
    assert_eq!(service.returning().len(), 2);
    assert_eq!(a.phase(), TotePhase::Returning { ready_at: t(30) });
    let mut replacement = service.try_collect(&a, &ha, t(40)).unwrap().unwrap();
    replacement.commit(t(50)).unwrap();
    let mut other = service.try_collect(&b, &hb, t(40)).unwrap().unwrap();
    other.commit(t(60)).unwrap();
    assert_eq!(service.unfinished(), 0);
    assert!(service.abandoned_custody().is_empty());
}

#[test]
fn collection_claim_uses_actual_pickup_completion_for_capacity_release() {
    let service = service(1, 1);
    let (tote, htm) = board(&service, 1, 1, 0);
    present(&service, &tote, LEFT, 10).clear(t(20), t(30)).unwrap();
    let mut claim = service.try_collect(&tote, &htm, t(40)).unwrap().unwrap();
    assert_eq!(claim.at(), t(40));
    assert!(matches!(claim.commit(t(39)), Err(ConveyanceError::TooEarly { .. })));
    assert!(service.try_admit(ToteId(2), RobotId(2), t(0)).unwrap().is_none());
    claim.commit(t(100)).unwrap();
    assert_eq!(claim.commit(t(100)), Err(ConveyanceError::SpentClaim));
    let next = service.try_admit(ToteId(2), RobotId(2), t(0)).unwrap().unwrap();
    assert_eq!(next.at(), t(100));
    assert_eq!(service.occupancy_at(t(99)).conveyor, 1);
    assert_eq!(service.occupancy_at(t(99)).retained_htms, 1);
    drop(next);
    drop(claim);
    assert_eq!(service.occupancy_at(t(100)).conveyor, 0);
    assert_eq!(service.unfinished(), 0);
}

#[tokio::test]
async fn collection_cancellation_wakes_waiters_and_preserves_named_custody() {
    let service = service(8, 8);
    let (a, ha) = board(&service, 1, 1, 0);
    let (b, hb) = board(&service, 2, 2, 0);
    present(&service, &a, LEFT, 10).clear(t(20), t(30)).unwrap();
    present(&service, &b, RIGHT, 10).clear(t(20), t(30)).unwrap();
    let claim = service.try_collect(&a, &hb, t(0)).unwrap().unwrap();
    let seen = service.version().seen();
    drop(claim);
    tokio::time::timeout(Duration::from_millis(100), service.version().past(seen)).await.unwrap();
    let mut first = service.try_collect(&a, &hb, t(0)).unwrap().unwrap();
    let mut second = service.try_collect(&b, &ha, t(0)).unwrap().unwrap();
    first.commit(t(40)).unwrap();
    second.commit(t(50)).unwrap();
    assert_eq!(a.phase(), TotePhase::Collected { at: t(40), robot: RobotId(2) });
    assert_eq!(b.phase(), TotePhase::Collected { at: t(50), robot: RobotId(1) });
    assert_eq!(service.unfinished(), 0);
}
