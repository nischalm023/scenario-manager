//! Physical movement shared by warehouse workflows. These tasks receive an
//! owned robot position, move it, and return its next position to their parent.

mod acquire_pipeline;
mod admit_area;
mod navigate;
mod parking;
mod path;
pub(crate) mod windows;

pub use acquire_pipeline::{AcquirePipeline, ProposedPipeline};
pub use admit_area::{AdmitArea, ProposedAdmission};
pub use navigate::{HtmPosition, Navigate, ProposedNavigate};
pub use parking::{ParkClaimedHtm, Parking, ParkingOutcome};
pub(crate) use path::{commit_plan_requests, move_distance};
pub use windows::PathCommitObserver;
