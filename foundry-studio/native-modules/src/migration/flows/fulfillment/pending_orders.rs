//! Pending orders are shared across available waiting berths. Selecting one is
//! revocable; the scheduler awards it at the berth's physical availability time.
//! Released orders retain OrderId priority; future work waits for its arrival.

use std::fmt;
use std::sync::{Arc, Mutex};

use crate::prelude::*;
use crate::scheduler::task::proposal::{AcceptError, Proposal, ProposedReservation};

#[derive(Clone)]
pub(super) struct PendingOrders {
    orders: Arc<Mutex<Vec<OrderSpec>>>,
    release_all_at_start: bool,
}

impl PendingOrders {
    pub fn new(mut orders: Vec<OrderSpec>) -> Result<Self, PlanError> {
        orders.sort_by_key(|order| order.id);
        if orders.windows(2).any(|pair| pair[0].id == pair[1].id) {
            return Err(super::invalid("pending orders contain a duplicate order ID"));
        }
        Ok(Self { orders: Arc::new(Mutex::new(orders)), release_all_at_start: false })
    }

    pub fn with_release_all_at_start(mut self, enabled: bool) -> Self {
        self.release_all_at_start = enabled;
        self
    }

    pub fn release_all_at_start(&self) -> bool {
        self.release_all_at_start
    }
}

#[derive(Display)]
#[display("ClaimNextOrder")]
pub(super) struct ClaimNextOrder {
    pending: PendingOrders,
    available_at: SimTime,
    clock: Mut<SimTime>,
}

impl ClaimNextOrder {
    pub fn new(pending: PendingOrders, available_at: SimTime, clock: Mut<SimTime>) -> Self {
        Self { pending, available_at, clock }
    }
}

pub(super) struct ProposedOrder {
    pending: PendingOrders,
    selected: (OrderId, SimTime),
    clock: Mut<SimTime>,
    accepted: Option<(OrderSpec, SimTime)>,
}

impl fmt::Display for ProposedOrder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "order claim {:?}", self.selected)
    }
}

impl ProposedReservation for ProposedOrder {
    fn start_time(&self) -> Option<SimTime> {
        Some(self.selected.1.max(self.clock.value()))
    }

    fn poisoned(&self) -> bool {
        self.pending.orders.lock().unwrap().binary_search_by_key(&self.selected.0, |order| order.id).is_err()
    }

    fn accept(&mut self) -> Result<(), AcceptError> {
        let (id, _) = self.selected;
        let mut orders = self.pending.orders.lock().unwrap();
        let index = orders.binary_search_by_key(&id, |order| order.id).map_err(|_| AcceptError::ActualConflict {
            detail: format!("order {id:?} was assigned without poisoning its other candidates"),
        })?;
        self.accepted = Some((orders.remove(index), self.selected.1.max(self.clock.value())));
        Ok(())
    }
}

pub(super) enum OrderProposal {
    Pending(ProposedOrder),
    Exhausted,
}

impl fmt::Display for OrderProposal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending(claim) => claim.fmt(f),
            Self::Exhausted => write!(f, "no pending orders"),
        }
    }
}

impl Proposal for OrderProposal {
    fn reservations(&mut self) -> Vec<&mut dyn ProposedReservation> {
        match self {
            Self::Pending(claim) => vec![claim],
            Self::Exhausted => Vec::new(),
        }
    }

    fn start_time(&self) -> Option<SimTime> {
        match self {
            Self::Pending(claim) => ProposedReservation::start_time(claim),
            Self::Exhausted => None,
        }
    }

    fn poisoned(&self) -> bool {
        matches!(self, Self::Pending(claim) if ProposedReservation::poisoned(claim))
    }
}

#[async_trait(?Send)]
impl Task for ClaimNextOrder {
    type Proposal = OrderProposal;
    type Schedule = Option<(OrderSpec, SimTime)>;

    async fn propose(&self, _: ProposalId) -> Result<OrderProposal, PlanError> {
        let floor = self.available_at.max(self.clock.value());
        let selected = self
            .pending
            .orders
            .lock()
            .unwrap()
            .iter()
            .map(|order| {
                let release = if self.pending.release_all_at_start {
                    SimTime::ZERO
                } else {
                    order.arrival.unwrap_or(SimTime::ZERO)
                };
                (order.id, floor.max(release))
            })
            .min_by_key(|&(id, at)| (at, id));
        Ok(match selected {
            Some(selected) => OrderProposal::Pending(ProposedOrder {
                pending: self.pending.clone(),
                selected,
                clock: self.clock.clone(),
                accepted: None,
            }),
            None => OrderProposal::Exhausted,
        })
    }

    async fn schedule(&mut self, proposal: OrderProposal, _: SchedCtx) -> Result<Self::Schedule, PlanError> {
        match proposal {
            OrderProposal::Pending(claim) => {
                claim.accepted.map(Some).ok_or_else(|| super::invalid("order claim was scheduled before acceptance"))
            }
            OrderProposal::Exhausted => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::Scheduler;
    use tokio::runtime::Builder;

    fn orders(count: u32) -> Vec<OrderSpec> {
        (0..count).map(|id| OrderSpec::unstated(OrderId(id), Vec::new())).collect()
    }

    fn claim_at(orders: Vec<OrderSpec>, times: &[u64]) -> Vec<Option<OrderId>> {
        let pending = PendingOrders::new(orders).unwrap();
        let mut scheduler = Scheduler::new();
        let claims: Vec<_> = times
            .iter()
            .map(|&seconds| {
                scheduler.add_task(ClaimNextOrder::new(
                    pending.clone(),
                    SimTime::seconds(seconds),
                    scheduler.barrier().clock(),
                ))
            })
            .collect();
        scheduler.run().unwrap();
        Builder::new_current_thread().build().unwrap().block_on(async {
            let mut assigned = Vec::new();
            for claim in claims {
                assigned.push(claim.await.map(|(order, _)| order.id));
            }
            assigned
        })
    }

    #[test]
    fn physical_availability_wins_over_task_creation_order() {
        assert_eq!(claim_at(orders(2), &[100, 10]), [Some(OrderId(1)), Some(OrderId(0))]);
    }

    #[test]
    fn competing_candidates_assign_each_order_once_and_replay_ties() {
        let expected: Vec<_> = (0..10).map(|id| Some(OrderId(id))).chain(std::iter::repeat_n(None, 14)).collect();
        for _ in 0..3 {
            assert_eq!(claim_at(orders(10), &[0; 24]), expected);
        }
    }

    #[test]
    fn released_work_precedes_a_future_order_with_a_lower_id() {
        let mut work = orders(3);
        work[0].arrival = Some(SimTime::seconds(100));
        work[2].arrival = Some(SimTime::seconds(50));
        assert_eq!(claim_at(work, &[0; 3]), [Some(OrderId(1)), Some(OrderId(2)), Some(OrderId(0))]);
    }

    #[test]
    fn already_released_orders_keep_id_priority() {
        let mut work = orders(2);
        work[0].arrival = Some(SimTime::seconds(100));
        work[1].arrival = Some(SimTime::seconds(10));
        assert_eq!(claim_at(work, &[200; 2]), [Some(OrderId(0)), Some(OrderId(1))]);
    }

    #[test]
    fn experiment_release_ignores_but_preserves_source_arrivals() {
        let mut work = orders(2);
        work[0].arrival = Some(SimTime::seconds(100));
        work[1].arrival = Some(SimTime::seconds(50));
        let pending = PendingOrders::new(work).unwrap().with_release_all_at_start(true);
        let source_arrivals: Vec<_> = pending.orders.lock().unwrap().iter().map(|order| order.arrival).collect();
        assert_eq!(source_arrivals, [Some(SimTime::seconds(100)), Some(SimTime::seconds(50))]);

        let scheduler = Scheduler::new();
        let task = ClaimNextOrder::new(pending, SimTime::ZERO, scheduler.barrier().clock());
        let proposal = Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(Task::propose(&task, ProposalId { task: TaskId(0), attempt: 0 }))
            .unwrap();
        assert_eq!(proposal.start_time(), Some(SimTime::ZERO));
    }

    #[derive(Display)]
    #[display("Berth({duration})")]
    struct Berth {
        pending: PendingOrders,
        duration: SimDuration,
    }

    #[async_trait(?Send)]
    impl Task for Berth {
        type Proposal = EmptyProposal;
        type Schedule = Vec<OrderId>;

        async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
            Ok(EmptyProposal)
        }

        async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<Vec<OrderId>, PlanError> {
            let mut ready = SimTime::ZERO;
            let mut assigned = Vec::new();
            while let Some((order, _)) =
                ctx.add_task(ClaimNextOrder::new(self.pending.clone(), ready, ctx.barrier().clock())).await
            {
                assigned.push(order.id);
                ready = ctx.barrier().floor() + self.duration;
            }
            Ok(assigned)
        }
    }

    #[test]
    fn a_berth_cannot_take_its_next_order_before_the_previous_work_finishes() {
        let pending = PendingOrders::new(orders(10)).unwrap();
        let mut scheduler = Scheduler::new();
        let slow = scheduler.add_task(Berth { pending: pending.clone(), duration: SimDuration::seconds(100) });
        let fast = scheduler.add_task(Berth { pending, duration: SimDuration::seconds(10) });
        scheduler.run().unwrap();
        let (slow, fast) = Builder::new_current_thread().build().unwrap().block_on(async { (slow.await, fast.await) });
        assert_eq!(slow, [OrderId(0)]);
        assert_eq!(fast, (1..10).map(OrderId).collect::<Vec<_>>());
    }

    #[test]
    fn dropping_a_candidate_leaves_the_order_pending() {
        let pending = PendingOrders::new(orders(1)).unwrap();
        let scheduler = Scheduler::new();
        let task = ClaimNextOrder::new(pending.clone(), SimTime::ZERO, scheduler.barrier().clock());
        let proposal = Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(Task::propose(&task, ProposalId { task: TaskId(0), attempt: 0 }))
            .unwrap();
        assert!(!Proposal::poisoned(&proposal));
        drop(proposal);
        assert_eq!(pending.orders.lock().unwrap().len(), 1);
    }
}
