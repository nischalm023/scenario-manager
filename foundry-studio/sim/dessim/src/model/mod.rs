//! The simulation's domain **model**: the persistent state and services the
//! flows act on — robots, stations, totes/inventory, relays, reservations,
//! orders, and the identifier/destination vocabulary they share. Flows
//! (`crate::flows`) mutate this state through the services here; the model
//! itself schedules nothing.

pub mod admission;
pub mod area_admission;
pub mod bot;
pub mod cell;
pub mod conveyance;
pub mod conveyor;
pub mod destination;
pub mod factories;
pub mod htm;
pub mod ids;
pub mod inventory;
pub(crate) mod location;
pub mod navigation;
pub mod order;
pub(crate) mod parking;
pub mod pipeline_admission;
pub use parking::ParkingRegistry;
pub mod relay;
pub mod reservation;
pub mod station;
pub mod station_service;
pub mod station_timing;
#[cfg(any(test, feature = "test-support"))]
pub mod test_model;

pub use crate::simtime::{SimDuration, SimOffset, SimTime};
pub use admission::AdmissionLaw;
pub use bot::plan::PlanError;
pub use bot::{BotKind, BotService};
pub use destination::{Destination, DistributionCenterId};
pub use ids::{OrderId, PresentationId, RackId, RackSlot, RobotId, SkuId, StorageSpaceId, TaskId, ToteId, VtmCycleId};
pub use inventory::{InventoryService, Tote};
pub use location::{ToteDesignation, ToteLocation, ToteType};
pub use order::{ExitPath, OrderContainer, OrderLine, OrderService, OrderSpec};
pub use relay::{RelayReservation, RelayService};
pub use reservation::{ReservationSnapshot, ReservationTable};
#[cfg(test)]
pub use station::test_station_set;
pub use station::{
    Station, StationArrivalDiagnostics, StationArrivalPlan, StationArrivalSource, StationDetails, StationSet,
};
pub use station_timing::{
    STATION_DELIVERY_MAX_WAIT_LOOSE, STATION_DELIVERY_MAX_WAIT_STRICT, StationFetchBounds, station_delivery_max_wait,
};
