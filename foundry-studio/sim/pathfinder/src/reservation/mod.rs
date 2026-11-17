//! Reservation system — cell occupancy tracking for reservation-aware pathfinding.

pub mod accumulator;
pub mod cell_reservation;
mod commit_wake;
pub mod diagnostics;
pub mod oracle;
pub mod table;

#[cfg(test)]
pub mod test;

pub use cell_reservation::{CellReservation, ReservationId};
pub use oracle::CellStatus;
pub use table::{ReservationSnapshot, ReservationTable};
