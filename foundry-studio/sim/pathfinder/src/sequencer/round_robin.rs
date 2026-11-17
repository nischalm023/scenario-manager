//! [`RoundRobin`], the default sequencer: turns in arrival order.

use std::collections::VecDeque;
use std::future::poll_fn;
use std::task::{Poll, Waker};

use async_trait::async_trait;
use desgraph::CellId;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use super::{Progress, Sequencer, Turn};
use crate::sequencer::TurnGuard;
use crate::{PathfinderError, RobotId};

#[derive(Default)]
struct Rotation {
    /// Askers in arrival order.
    queue: VecDeque<RobotId>,
    /// The robot whose turn is running.
    holder: Option<RobotId>,
    /// Blocked askers and what they need: ineligible until a commit frees a needed cell
    /// (any commit, when they could name no needs).
    stale: FxHashMap<RobotId, Vec<CellId>>,
    wakers: Vec<(RobotId, Waker)>,
    granted: u64,
}

impl Rotation {
    /// Whose turn is next: the first asker that is not stale.
    fn next(&self) -> Option<RobotId> {
        self.queue.iter().find(|r| !self.stale.contains_key(r)).copied()
    }
}

/// Turns in arrival order, one at a time. A `Blocked` robot stays ineligible until a commit frees a
/// cell it needs; [`RoundRobin::wedged`] names a stuck set.
#[derive(Default)]
pub struct RoundRobin {
    rotation: Mutex<Rotation>,
}

impl RoundRobin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn turns_granted(&self) -> u64 {
        self.rotation.lock().granted
    }

    /// `robot` stopped asking: out of the queue and blocked set, so it can neither be granted a turn
    /// nobody will take nor stand in front of the askers behind it.
    fn withdraw(&self, robot: RobotId) {
        {
            let rotation = &mut *self.rotation.lock();
            rotation.queue.retain(|r| *r != robot);
            rotation.stale.remove(&robot);
            rotation.wakers.retain(|(r, _)| *r != robot);
        }

        self.wake_all();
    }

    pub fn waiting(&self) -> Vec<RobotId> {
        self.rotation.lock().queue.iter().copied().collect()
    }

    /// Every asker is waiting for a commit that no asker can make: the set of
    /// them, in id order. Empty while anyone can still move.
    pub fn wedged(&self) -> Vec<RobotId> {
        let rotation = self.rotation.lock();
        if rotation.holder.is_some() || rotation.queue.is_empty() || rotation.next().is_some() {
            return Vec::new();
        }

        let mut wedged: Vec<RobotId> = rotation.queue.iter().copied().collect();
        wedged.sort();
        wedged
    }

    fn wake_all(&self) {
        let wakers = std::mem::take(&mut self.rotation.lock().wakers);
        for (_, waker) in wakers {
            waker.wake();
        }
    }
}

/// An ask in progress: dropped before its turn (the asker's task was cancelled), it withdraws the robot
/// from the rotation instead of leaving a ghost at the head of the queue that no one will ever poll.
struct Asking<'a> {
    rotation: &'a RoundRobin,
    robot: RobotId,
    granted: bool,
}

impl Drop for Asking<'_> {
    fn drop(&mut self) {
        if !self.granted {
            self.rotation.withdraw(self.robot);
        }
    }
}

#[async_trait(?Send)]
impl Sequencer for RoundRobin {
    async fn next_turn(&self, robot: RobotId, _at: CellId, _to: CellId) -> (Turn, TurnGuard) {
        {
            let mut rotation = self.rotation.lock();
            assert!(
                rotation.holder != Some(robot) && !rotation.queue.contains(&robot),
                "robot {robot:?} already has an outstanding request or turn"
            );
            rotation.queue.push_back(robot);
        }

        let mut asking = Asking { rotation: self, robot, granted: false };
        poll_fn(|cx| {
            let rotation = &mut *self.rotation.lock();
            if rotation.holder.is_some() || rotation.next() != Some(robot) {
                if let Some((_, waker)) = rotation.wakers.iter_mut().find(|(r, _)| *r == robot) {
                    waker.clone_from(cx.waker());
                } else {
                    rotation.wakers.push((robot, cx.waker().clone()));
                }
                return Poll::Pending;
            }

            rotation.queue.retain(|r| *r != robot);
            rotation.granted += 1;
            rotation.holder = Some(robot);
            asking.granted = true;
            Poll::Ready((Turn::Advance, TurnGuard::new(robot)))
        })
        .await
    }

    fn end_turn(&self, mut guard: TurnGuard, progress: &Result<Progress, PathfinderError>) {
        guard.disarm();

        {
            let rotation = &mut *self.rotation.lock();
            let holder = rotation.holder.take();
            let robot = guard.robot_id();
            debug_assert_eq!(holder, Some(robot), "end_turn from a robot that does not hold the turn");

            match progress {
                Ok(Progress::Arrived { freed }) => {
                    rotation
                        .stale
                        .retain(|_, needs| !needs.is_empty() && !freed.iter().any(|cell| needs.contains(cell)));
                }

                Ok(Progress::Blocked { needs, .. }) => {
                    rotation.stale.insert(robot, needs.clone());
                }

                Err(_) => {
                    rotation.stale.remove(&robot);
                }
            }
        }

        self.wake_all();
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::pin::pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context, Poll, Wake, Waker};

    use desgraph::CellId;

    use super::{Progress, RoundRobin, Sequencer, Turn, TurnGuard};
    use crate::{PathfinderError, RobotId};

    /// Poll an ask once: `Some` when the turn was granted on this poll.
    fn poll_once(ask: &mut (impl Future<Output = (Turn, TurnGuard)> + Unpin)) -> Option<TurnGuard> {
        match pin!(ask).poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready((Turn::Advance, guard)) => Some(guard),
            Poll::Pending => None,
        }
    }

    fn ask(sequencer: &RoundRobin, robot: u32) -> impl Future<Output = (Turn, TurnGuard)> + Unpin + '_ {
        Box::pin(sequencer.next_turn(RobotId(robot), CellId(0), CellId(1)))
    }

    #[test]
    fn turns_go_in_arrival_order_one_at_a_time() {
        let sequencer = RoundRobin::new();
        let (mut first, mut second) = (ask(&sequencer, 1), ask(&sequencer, 2));

        let granted = poll_once(&mut first).expect("the first asker is granted at once");
        assert!(poll_once(&mut second).is_none(), "the second waits while the first holds the turn");
        assert_eq!(sequencer.waiting(), vec![RobotId(2)]);

        sequencer.end_turn(granted, &Ok(Progress::Arrived { freed: vec![CellId(0)] }));
        let granted = poll_once(&mut second).expect("the second is granted once the first ends");
        sequencer.end_turn(granted, &Ok(Progress::Arrived { freed: vec![] }));
        assert_eq!(sequencer.turns_granted(), 2);
    }

    #[test]
    fn a_blocked_robot_waits_for_the_cell_it_needs_and_a_wedged_fleet_is_named() {
        let sequencer = RoundRobin::new();
        let granted = poll_once(&mut ask(&sequencer, 1)).unwrap();
        sequencer.end_turn(granted, &Ok(Progress::Blocked { by: vec![RobotId(2)], needs: vec![CellId(5)] }));

        let (mut blocked, mut other) = (ask(&sequencer, 1), ask(&sequencer, 2));
        assert!(poll_once(&mut blocked).is_none(), "a blocked asker is passed over");
        let granted = poll_once(&mut other).expect("the asker behind it goes first");
        sequencer.end_turn(granted, &Ok(Progress::Arrived { freed: vec![CellId(9)] }));
        assert!(poll_once(&mut blocked).is_none(), "a commit that frees nothing it needs leaves it blocked");
        assert_eq!(sequencer.wedged(), vec![RobotId(1)], "nobody else is asking, so the fleet is stuck");

        let granted = poll_once(&mut ask(&sequencer, 2)).unwrap();
        sequencer.end_turn(granted, &Ok(Progress::Arrived { freed: vec![CellId(5)] }));
        assert!(sequencer.wedged().is_empty());
        let granted = poll_once(&mut blocked).expect("freeing the needed cell makes it eligible again");
        sequencer.end_turn(granted, &Ok(Progress::Arrived { freed: vec![] }));
    }

    #[test]
    fn duplicate_pending_request_is_rejected_without_withdrawing_the_original() {
        let sequencer = RoundRobin::new();
        let held = poll_once(&mut ask(&sequencer, 1)).unwrap();
        let mut original = ask(&sequencer, 2);
        assert!(poll_once(&mut original).is_none());
        let duplicate = catch_unwind(AssertUnwindSafe(|| {
            poll_once(&mut ask(&sequencer, 2));
        }));
        assert!(duplicate.is_err(), "duplicate requests must fail instead of sharing a queue entry");
        assert_eq!(sequencer.waiting(), vec![RobotId(2)]);
        sequencer.end_turn(held, &Ok(Progress::Arrived { freed: vec![] }));
        let granted = poll_once(&mut original).expect("the original request survives the duplicate");
        sequencer.end_turn(granted, &Ok(Progress::Arrived { freed: vec![] }));
    }

    #[test]
    fn a_robot_cannot_ask_again_while_holding_a_turn() {
        let sequencer = RoundRobin::new();
        let held = poll_once(&mut ask(&sequencer, 1)).unwrap();
        let duplicate = catch_unwind(AssertUnwindSafe(|| {
            poll_once(&mut ask(&sequencer, 1));
        }));
        assert!(duplicate.is_err());
        sequencer.end_turn(held, &Ok(Progress::Arrived { freed: vec![] }));
        let granted = poll_once(&mut ask(&sequencer, 1)).expect("a new request is legal after end_turn");
        sequencer.end_turn(granted, &Ok(Progress::Arrived { freed: vec![] }));
    }

    #[test]
    fn cancelling_the_next_asker_lets_its_successor_take_the_turn() {
        let sequencer = RoundRobin::new();
        let held = poll_once(&mut ask(&sequencer, 1)).unwrap();
        let mut cancelled = ask(&sequencer, 2);
        let mut successor = pin!(ask(&sequencer, 3));
        let wakes = Arc::new(WakeCount::default());
        let waker = Waker::from(wakes.clone());
        let mut context = Context::from_waker(&waker);
        assert!(poll_once(&mut cancelled).is_none());
        assert!(successor.as_mut().poll(&mut context).is_pending());
        sequencer.end_turn(held, &Ok(Progress::Arrived { freed: vec![] }));
        assert!(successor.as_mut().poll(&mut context).is_pending(), "the next asker still has priority");
        wakes.0.store(0, Ordering::Relaxed);
        drop(cancelled);
        assert_eq!(wakes.0.load(Ordering::Relaxed), 1, "cancellation must wake the successor");
        let granted = poll_once(&mut successor).expect("the cancelled request leaves no ghost ahead of it");
        sequencer.end_turn(granted, &Ok(Progress::Arrived { freed: vec![] }));
        assert!(sequencer.waiting().is_empty());
    }

    #[derive(Default)]
    struct WakeCount(AtomicUsize);

    impl Wake for WakeCount {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn a_waiter_registers_only_its_latest_waker() {
        let sequencer = RoundRobin::new();
        let held = poll_once(&mut ask(&sequencer, 1)).unwrap();
        let mut waiting = pin!(ask(&sequencer, 2));
        let old = Arc::new(WakeCount::default());
        let latest = Arc::new(WakeCount::default());
        for counter in [&old, &latest, &latest] {
            let waker = Waker::from(counter.clone());
            assert!(waiting.as_mut().poll(&mut Context::from_waker(&waker)).is_pending());
        }
        sequencer.end_turn(held, &Ok(Progress::Arrived { freed: vec![] }));
        assert_eq!(old.0.load(Ordering::Relaxed), 0);
        assert_eq!(latest.0.load(Ordering::Relaxed), 1);
        let granted = poll_once(&mut waiting).unwrap();
        sequencer.end_turn(granted, &Ok(Progress::Arrived { freed: vec![] }));
    }

    #[test]
    fn an_empty_needs_list_waits_for_a_commit_and_errors_release_the_turn() {
        let sequencer = RoundRobin::new();
        let held = poll_once(&mut ask(&sequencer, 1)).unwrap();
        sequencer.end_turn(held, &Ok(Progress::Blocked { by: vec![], needs: vec![] }));
        let mut blocked = ask(&sequencer, 1);
        assert!(poll_once(&mut blocked).is_none());

        let held = poll_once(&mut ask(&sequencer, 2)).unwrap();
        let error = Err(PathfinderError::Unreachable { from: CellId(0), to: CellId(1) });
        sequencer.end_turn(held, &error);
        assert!(poll_once(&mut blocked).is_none(), "a failed attempt is not a commit");

        let held = poll_once(&mut ask(&sequencer, 2)).expect("the failed turn released its holder");
        sequencer.end_turn(held, &Ok(Progress::Arrived { freed: vec![] }));
        let granted = poll_once(&mut blocked).expect("any successful commit permits the blocked retry");
        sequencer.end_turn(granted, &Ok(Progress::Arrived { freed: vec![] }));
    }
}
