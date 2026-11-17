//! # desunits
//!
//! The quantities and identities every crate that speaks about a simulation
//! run shares: simulated time ([`SimTime`], [`SimDuration`], [`SimOffset`],
//! [`TimeRange`]), distance along one axis ([`Position`], [`Distance`],
//! [`DistanceDelta`]) and the ids of the things a run names ([`RobotId`],
//! [`ToteId`], and the rest of [`ids`]).
//!
//! It exists so the output codec, the browser viewer, and the broker can name
//! a robot at an instant without depending on the simulator, which pulls in
//! rayon, dashmap, and parking_lot, none of which build for wasm32.
//! The simulator re-exports everything here at its old paths, so nothing
//! inside it changed its spelling.

pub mod distance;
pub mod ids;
pub mod simtime;

pub use distance::{Distance, DistanceDelta, Position};
pub use ids::{OrderId, PresentationId, RackId, RackSlot, RobotId, SkuId, StorageSpaceId, TaskId, ToteId, VtmCycleId};
pub use simtime::{SimDuration, SimOffset, SimTime, TimeRange, TimeRangeError};
