//! Workflow-specific flow modules.
//!
//! Each flow defines the concrete task types, handoff patterns, and
//! domain-specific logic for a particular warehouse workflow. The core
//! Scheduler is agnostic to these — it sees only `dyn Task`.

pub mod transport;
pub mod ttp;

pub mod fulfillment;
