//! Coordinate the first inventory retrieval with its order-container plan.
//! A future readiness timestamp bounds how early retrieval may start; an
//! empty-HTM margin lets that retrieval complete even with a two-robot fleet.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::error::PlanError;
use crate::model::htm::registry::PickupHeadroom;
use crate::model::{SimDuration, SimTime};
use crate::signals::Version;

#[derive(Default)]
pub(super) struct PairPreparation {
    order_ready: Mutex<Option<SimTime>>,
    inventory_owned: AtomicBool,
    changed: Version,
}

impl PairPreparation {
    pub fn own_order(&self, ready: SimTime) -> Result<(), PlanError> {
        let mut at = self.order_ready.lock().unwrap();
        if at.is_some() {
            return Err(super::invalid("order-container readiness was already published"));
        }
        *at = Some(ready);
        drop(at);
        self.changed.bump();
        Ok(())
    }

    pub fn own_inventory(&self) {
        self.inventory_owned.store(true, Ordering::Release);
        self.changed.bump();
    }

    pub fn order_headroom(self: &Arc<Self>) -> PickupHeadroom {
        let pair = self.clone();
        PickupHeadroom::new(1, Arc::new(move || !pair.inventory_owned.load(Ordering::Acquire)), self.changed.clone())
    }

    pub async fn inventory_release(&self, fetch_lead: SimDuration) -> SimTime {
        loop {
            let seen = self.changed.seen();
            if let Some(ready) = *self.order_ready.lock().unwrap() {
                return ready.saturating_sub(fetch_lead);
            }
            self.changed.past(seen).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::task::{Context, Waker};
    use tokio::runtime::Builder;

    use super::*;

    #[test]
    fn physical_release_waits_for_a_container_plan_and_uses_its_simulation_time() {
        let pair = PairPreparation::default();
        let lead = SimDuration::seconds(133);
        assert!(
            Box::pin(pair.inventory_release(lead)).as_mut().poll(&mut Context::from_waker(Waker::noop())).is_pending()
        );
        let runtime = Builder::new_current_thread().build().unwrap();
        let (released, ()) = runtime.block_on(async {
            tokio::join!(pair.inventory_release(lead), async {
                tokio::task::yield_now().await;
                pair.own_order(SimTime::seconds(600)).unwrap();
            })
        });
        assert_eq!(released, SimTime::seconds(467));
        assert!(released < SimTime::seconds(600), "inventory fetch may overlap future container readiness");
        assert!(pair.own_order(SimTime::seconds(900)).is_err(), "an accepted readiness bound is not overwritten");
    }
}
