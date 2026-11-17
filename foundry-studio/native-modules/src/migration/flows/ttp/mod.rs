//! Tote-to-Person (TTP) flow: workflow-specific types for the relay-based
//! VTM/HTM tote delivery system.
//!
//! This module contains types that are specific to the TTP workflow and
//! should NOT be imported by the core Scheduler. The Scheduler is
//! agnostic to flow-specific concepts like relays, stations, and bot types.

pub mod demand;
pub mod handoff;
pub mod inflight;
pub mod scoring;
pub mod task;

pub mod setup;
