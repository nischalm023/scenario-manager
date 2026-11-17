//! Within `finality`, `recording` is where the observer hands each slice once
//! it is final, so the run's recording is written as the run goes. The
//! observer knows nothing of the format: whatever writes the recording
//! implements [`SliceRecorder`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use desgraph::RobotType;
use thiserror::Error;

use crate::model::{RobotId, SimTime};
use crate::observer::output::{
    AisleFluxRecord, AisleStateRecord, CompletedOrderRecord, HtmLegRecord, PickRecord, PutawayRecord, VtmCycleRecord,
    VtmLegRecord, VtmOperationRecord,
};
use crate::observer::robot_state::RobotSliceSpans;
use crate::observer::time_slice::TimeSliceData;

/// A slice the observer has made final, and what a recording states beside it.
pub struct FinalSlice<'a> {
    pub slice: &'a TimeSliceData,
    /// The slice after it as B leaves it, when a fact has made it. A leg that
    /// crosses the boundary was reported before B passed it, so this slice
    /// already names the cell each robot still moving at the boundary is in.
    pub next: Option<&'a TimeSliceData>,
    /// Where the slice ends: its full width on, or the run's stop for the
    /// run's last slice, which the stop cuts.
    pub ends: SimTime,
    /// Whether this is the run's last slice, which states the facts filed
    /// exactly at the stop at its end.
    pub last: bool,
    /// The journal rows due with this slice: those whose instant of finality
    /// B has passed (see `rows_due`).
    pub rows: SliceRows,
    /// The spans of every robot that enters one in this slice, laid end to
    /// end across it up to `ends` (see `robot_state`).
    pub robot_spans: Vec<RobotSliceSpans>,
    /// The robots the run has read of, by id.
    pub roster: &'a HashMap<RobotId, RobotType>,
}

/// The rows of every journal that one frame states.
#[derive(Default)]
pub struct SliceRows {
    pub htm_legs: Vec<HtmLegRecord>,
    pub vtm_legs: Vec<VtmLegRecord>,
    pub vtm_cycles: Vec<VtmCycleRecord>,
    pub vtm_operations: Vec<VtmOperationRecord>,
    pub picks: Vec<PickRecord>,
    pub putaways: Vec<PutawayRecord>,
    pub completed_orders: Vec<CompletedOrderRecord>,
    pub aisle_states: Vec<AisleStateRecord>,
    pub aisle_flux: Vec<AisleFluxRecord>,
}

/// What writes the slices the observer makes final, each once and in order.
pub trait SliceRecorder: Send {
    /// Write one slice. A slice that cannot be written ends the recording:
    /// the recorder keeps its own account of the failure for whoever closes
    /// it, and the observer hands it no more slices and stops the run.
    fn record(&mut self, slice: FinalSlice<'_>) -> Result<(), RecordingFailed>;

    /// Keep the journal rows the run reported after its last slice went out,
    /// filed by its stop, which only the recording's end can state.
    /// Handed once, as the stream closes; empty when the run's last slice took
    /// every row.
    fn keep_rows_left(&mut self, rows: SliceRows);
}

/// A recorder could not write a slice. The recording is the run's output, so
/// the observer stops the run on it, with the failure's own words as the
/// reason: the observer knows nothing of the format the failure is about.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("the run's recording failed: {0}")]
pub struct RecordingFailed(pub String);

/// A recorder shared between the observer, which hands it the slices, and the
/// run's caller, which closes it once the run is over.
pub type SharedRecorder = Arc<Mutex<dyn SliceRecorder>>;
