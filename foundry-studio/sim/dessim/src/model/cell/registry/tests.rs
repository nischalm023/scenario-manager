use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use super::*;
use crate::model::test_model::{OneWindow, TestModelConfig, block, build_model};
use crate::model::{RobotId, SimDuration, ids::TaskId};
use crate::pathfinding::MotionModel;
use crate::pathfinding::{CollisionConfig, Orientation};
use crate::simtime::TimeRange;

fn owner(attempt: u32) -> ProposalId {
    ProposalId { task: TaskId(1), attempt }
}

fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

/// An entrant keeps a need-by at 30 s on a hold whose `Vacate` is at 100 s.
/// Once that `Vacate` is replaced by one at 200 s, the keeper answers the
/// replacement; once the keeper is dropped, the replacement is due at its
/// own start again.
#[test]
fn a_kept_need_by_answers_each_replacement_until_its_keeper_drops() {
    let (model, oracle) = build_model(TestModelConfig::default());
    let source = oracle.table().reserve_cell(RobotId(1), CellId(10), TimeRange::open_ended(SimTime::ZERO)).unwrap();
    let cells = &model.navigation.cells;
    let first = cells.vacate(&source, owner(0), SimTime::seconds(100));
    let mut keeper = Box::pin(cells.keep_need_by(&source, SimTime::seconds(30)));
    assert!(poll_once(keeper.as_mut()).is_pending());
    assert_eq!(first.resolve_by(SimTime::seconds(100), SimTime::ZERO), SimTime::seconds(30));
    let replacement = cells.vacate(&source, owner(1), SimTime::seconds(200));
    drop(first);
    assert!(poll_once(keeper.as_mut()).is_pending());
    assert_eq!(replacement.resolve_by(SimTime::seconds(200), SimTime::ZERO), SimTime::seconds(30));
    drop(keeper);
    assert_eq!(replacement.resolve_by(SimTime::seconds(200), SimTime::ZERO), SimTime::seconds(200));
}

#[test]
fn need_bys_only_advance_the_vacate_they_answer_and_withdraw_independently() {
    let (model, oracle) = build_model(TestModelConfig::default());
    let source = oracle.table().reserve_cell(RobotId(1), CellId(10), TimeRange::open_ended(SimTime::ZERO)).unwrap();
    let cells = &model.navigation.cells;
    let vacate = cells.vacate(&source, owner(0), SimTime::seconds(100));
    assert_eq!(vacate.resolve_by(SimTime::seconds(100), SimTime::ZERO), SimTime::seconds(100));
    let first = cells.need_by(cells.vacating(&source).unwrap(), SimTime::ZERO).unwrap();
    let second = cells.need_by(cells.vacating(&source).unwrap(), SimTime::seconds(30)).unwrap();
    assert_eq!(vacate.resolve_by(SimTime::seconds(100), SimTime::seconds(20)), SimTime::seconds(20));
    drop(first);
    assert_eq!(vacate.resolve_by(SimTime::seconds(100), SimTime::ZERO), SimTime::seconds(30));
    let replacement = cells.vacate(&source, owner(1), SimTime::seconds(200));
    assert_eq!(replacement.resolve_by(SimTime::seconds(200), SimTime::ZERO), SimTime::seconds(200));
    drop(vacate);
    assert!(!second.is_live());
    let current = cells.vacating(&source).unwrap();
    assert_eq!(current.id.owner, owner(1));
    let current_need_by = cells.need_by(current, SimTime::seconds(40)).unwrap();
    drop(second);
    assert!(current_need_by.is_live());
    assert_eq!(replacement.resolve_by(SimTime::seconds(200), SimTime::ZERO), SimTime::seconds(40));
    drop(current_need_by);
    assert_eq!(replacement.resolve_by(SimTime::seconds(200), SimTime::ZERO), SimTime::seconds(200));
}

#[test]
fn a_closed_or_replaced_hold_cannot_keep_a_need_by_live() {
    let (model, oracle) = build_model(TestModelConfig::default());
    let source = oracle.table().reserve_cell(RobotId(1), CellId(10), TimeRange::open_ended(SimTime::ZERO)).unwrap();
    let cells = &model.navigation.cells;
    let vacate = cells.vacate(&source, owner(0), SimTime::seconds(100));
    let candidate = cells.vacating(&source).unwrap();
    let need_by = cells.need_by(candidate, SimTime::ZERO).unwrap();
    let config = CollisionConfig {
        self_robot: source.robot(),
        predecessor: Some(source.clone()),
        sim_start: SimTime::seconds(50),
        dest_dwell: None,
        motion_model: MotionModel::ConstantSpeed,
        destination: None,
        collision_depth: 8,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(120),
        max_iterations: 5_000_000,
    };
    block(model.navigation.pathfinder.reserve_path(
        source.cell(),
        Orientation::East,
        CellId(11),
        crate::model::test_model::reference_model().estimate[0].as_ref(),
        oracle.as_ref(),
        &config,
        &mut OneWindow(&model.events),
    ))
    .unwrap();
    assert!(source.is_open_ended(), "the external reservation snapshot intentionally remains stale");
    assert!(!need_by.is_live());
    assert!(cells.vacating(&source).is_none());
    assert!(cells.need_by(candidate, SimTime::ZERO).is_none());
    assert_eq!(vacate.resolve_by(SimTime::seconds(100), SimTime::ZERO), SimTime::seconds(100));
    let next_source =
        oracle.table().reserve_cell(RobotId(1), CellId(10), TimeRange::open_ended(SimTime::seconds(150))).unwrap();
    let next_vacate = cells.vacate(&next_source, owner(1), SimTime::seconds(200));
    let next_need_by = cells.need_by(cells.vacating(&next_source).unwrap(), SimTime::seconds(40)).unwrap();
    drop(vacate);
    drop(need_by);
    assert!(next_need_by.is_live());
    assert_eq!(next_vacate.resolve_by(SimTime::seconds(200), SimTime::ZERO), SimTime::seconds(40));
}
