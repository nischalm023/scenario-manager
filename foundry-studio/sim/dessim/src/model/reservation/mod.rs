//! Reservation system — `pathfinder`'s cell occupancy tracking at the paths
//! the simulator has always used, plus dessim's own `TableOracle`.

// Named for what it holds; `oracle` stays the module path the simulator has always used.
#[path = "table_oracle.rs"]
pub mod oracle;

pub use pathfinder::reservation::{CellReservation, CellStatus, ReservationId, ReservationSnapshot, ReservationTable};
pub use pathfinder::reservation::{accumulator, cell_reservation, diagnostics, table};
