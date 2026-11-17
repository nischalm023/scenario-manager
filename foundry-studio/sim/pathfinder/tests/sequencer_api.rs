//! The public turn protocol can be implemented outside the pathfinder crate.

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use async_trait::async_trait;
use desgraph::CellId;
use pathfinder::{PathfinderError, Progress, RobotId, Sequencer, Turn, TurnGuard};

struct CustomSequencer;

#[async_trait(?Send)]
impl Sequencer for CustomSequencer {
    async fn next_turn(&self, robot: RobotId, _at: CellId, _to: CellId) -> (Turn, TurnGuard) {
        (Turn::Advance, TurnGuard::new(robot))
    }

    fn end_turn(&self, mut guard: TurnGuard, _progress: &Result<Progress, PathfinderError>) {
        guard.disarm();
    }
}

#[test]
fn an_external_implementation_can_issue_and_finish_a_turn() {
    let sequencer: &dyn Sequencer = &CustomSequencer;
    let mut request = pin!(sequencer.next_turn(RobotId(7), CellId(1), CellId(2)));
    let Poll::Ready((Turn::Advance, guard)) = request.as_mut().poll(&mut Context::from_waker(Waker::noop())) else {
        panic!("the test sequencer grants immediately");
    };
    assert_eq!(guard.robot_id(), RobotId(7));
    sequencer.end_turn(guard, &Ok(Progress::Arrived { freed: vec![CellId(1)] }));
}
