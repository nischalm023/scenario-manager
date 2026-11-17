//! Within `task`, `stand_ins` registers, in dessim's own unit tests only, a stand-in for each root the named
//! compositions (`compositions/*.json`) list. Those tests link no task, yet scenario generation resolves a run's
//! composition, and a composition names only registered roots. A stand-in takes any configuration and does no work.

use std::fmt;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::PlanError;
use crate::scheduler::task::proposal::{EmptyProposal, ProposalId};
use crate::scheduler::task::{SchedCtx, Task};
use crate::task::{TaskContext, invalid};

macro_rules! stand_ins {
    ($($name:ident),* $(,)?) => {$(
        #[crate::task]
        struct $name;

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(stringify!($name))
            }
        }

        #[async_trait(?Send)]
        impl Task for $name {
            type Proposal = EmptyProposal;
            type Schedule = ();

            fn create(_: &TaskContext) -> Result<Self, PlanError> {
                Ok($name)
            }

            fn validate_config(_: &Value) -> Result<(), PlanError> {
                Ok(())
            }

            async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
                Err(invalid(concat!(stringify!($name), " is a stand-in and does no work")))
            }

            async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<(), PlanError> {
                Err(invalid(concat!(stringify!($name), " is a stand-in and does no work")))
            }
        }
    )*};
}

stand_ins!(
    InitializeFleet,
    FillInventory,
    LoadOrders,
    FleetConservation,
    PutawaySupplyDemand,
    PutawayInduction,
    PickDemand,
    FulfillOrders,
);
