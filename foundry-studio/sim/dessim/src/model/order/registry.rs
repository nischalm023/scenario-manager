//! The run's orders by id, whether a scenario stated them or the demand pool
//! built them mid-run.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::model::ids::OrderId;

use super::OrderSpec;

#[derive(Clone, Default)]
pub struct OrderService(Arc<RwLock<BTreeMap<OrderId, OrderSpec>>>);

impl OrderService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, order: OrderSpec) {
        self.0.write().unwrap().insert(order.id, order);
    }

    pub fn extend(&self, orders: impl IntoIterator<Item = OrderSpec>) {
        let mut guard = self.0.write().unwrap();
        for order in orders {
            guard.insert(order.id, order);
        }
    }

    /// Scoped because the orders sit behind a lock.
    pub fn with<R>(&self, f: impl FnOnce(&BTreeMap<OrderId, OrderSpec>) -> R) -> R {
        f(&self.0.read().unwrap())
    }

    pub fn snapshot(&self) -> Vec<OrderSpec> {
        self.0.read().unwrap().values().cloned().collect()
    }

    pub fn get(&self, id: OrderId) -> Option<OrderSpec> {
        self.0.read().unwrap().get(&id).cloned()
    }

    pub fn len(&self) -> usize {
        self.0.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.read().unwrap().is_empty()
    }
}
