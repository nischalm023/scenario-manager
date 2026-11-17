//! `Unparkable` passes over a stay whose parking found no bay while the
//! oracle's version is the one it was recorded at, and forgets it once a path
//! commits, though the new parking would depart at the same simtime.

use desgraph::CellId;

use super::*;
use crate::model::test_model::{TestModelConfig, build_model, squatter_ready_at};

/// Robot 95 stands on cell 10 from simtime zero, idle from 60 s, and its
/// parking departing at 60 s found no bay at oracle version 0. With the
/// barrier at 5 s a new parking would depart at 60 s too: it is passed over
/// while the version stays 0, and tried once a path commits (version 1).
#[test]
fn a_stay_whose_parking_found_no_bay_is_tried_again_after_a_path_commits() {
    let (model, oracle) = build_model(TestModelConfig::default());
    squatter_ready_at(&model, &oracle, RobotId(95), CellId(10), SimTime::seconds(60));
    let squatter = model.available_htm.get(RobotId(95)).expect("robot 95's stay");
    let barrier = SimTime::seconds(5);
    let mut unparkable = Unparkable::default();
    unparkable.as_of(Some(0));
    unparkable.insert(squatter.availability_id(), SimTime::seconds(60));
    unparkable.as_of(Some(0));
    assert_eq!(unparkable.untried_departure(&squatter, barrier), None, "passed over with no path committed");
    unparkable.as_of(Some(1));
    let departure = unparkable.untried_departure(&squatter, barrier);
    assert_eq!(departure, Some(SimTime::seconds(60)), "tried again, departing at 60 s, once a path commits");
}
