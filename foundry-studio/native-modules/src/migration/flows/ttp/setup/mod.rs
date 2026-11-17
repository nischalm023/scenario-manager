//! Finite roots that populate initial warehouse state. They commit setup only
//! in schedule; consumers declare completion dependencies in their composition.

mod fill_inventory;
mod induction_schedule;
mod initialize_fleet;
mod load_orders;
