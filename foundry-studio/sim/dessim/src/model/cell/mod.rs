//! A floor cell as a resource, within `model`: one robot at a time, booked
//! through the reservation oracle, and held open-ended by the robot on it
//! until that robot's next task commits its path (scheduler time). This
//! module holds the cell's registry.
//!
//! Proposals that compete for one resource register in a registry beside
//! it, at `<resource>/registry.rs`. A registry has up to two sides. A
//! claim side ranks the proposals that want the resource, and the
//! scheduler's contest reads that ranking through `ranked_competitors`.
//! Where a holder can stand
//! open-ended, a vacating side records, for each hold, who would vacate it
//! at what simtime and who needs that decided before the barrier passes a
//! simtime T, and the vacating proposal's decision deadline reads those
//! need-bys. A cell's registry has the vacating side alone: who enters a
//! cell next is decided by the reservations its entrants commit.
//! `model/order/registry.rs` is a lookup of orders by id, not this pattern.
//!
//! TODO(module): `model/htm/registry.rs` holds the HTM pool's contest, the
//! claim side for an HTM's stay, together with the pool and its pickup
//! candidacy; splitting them is fwd task NdzgcalQ.

pub mod registry;
