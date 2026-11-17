//! Build orders by bringing inventory and order totes together at a shared
//! conveyance, retaining each robot through the work it physically owns.

pub(super) mod collect_tote;
pub mod fetch_bound;
pub mod induct_container;
mod orders;
mod outbound;
mod pending_orders;
mod pending_returns;
mod preparation;
mod present_tote;
mod progress;
mod retrieve_tote;
mod store_tote;
mod supply_conveyor;
mod tote_handoff;
pub mod transfer_items;

pub use orders::FulfillOrders;

use crate::error::PlanError;

fn invalid(detail: impl Into<String>) -> PlanError {
    PlanError::TaskGraphInvariant { context: "fulfillment", detail: detail.into() }
}
