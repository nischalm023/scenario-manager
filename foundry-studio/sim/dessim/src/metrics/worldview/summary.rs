//! Within `worldview`, `summary` is THE SUMMARY AS THE END BLOCK CARRIES IT:
//! the run's headline numbers with the journals taken out.
//!
//! A recording already states every journal row as table rows, each in the
//! frame written once B has passed the instant that makes it final. Packing
//! them a second time into the summary would double the file and would leave
//! a run that died with none of them, so they are lifted out for the encoding
//! and put straight back: the caller hands over a [`SummaryStats`] and gets
//! the same one back.

use serde::Serialize;
use serde::de::DeserializeOwned;
use worldview::container::payload::Opaque;

use crate::metrics::profile::SummaryStats;
use crate::metrics::worldview::write::RecordingError;

/// The summary as the end block carries it, with the nine journals taken out
/// and restored before this returns.
pub fn summary_document(summary: &mut SummaryStats) -> Result<Opaque, RecordingError> {
    let held = (
        std::mem::take(&mut summary.pick_log),
        std::mem::take(&mut summary.completed_order_log),
        std::mem::take(&mut summary.putaway_log),
        std::mem::take(&mut summary.htm_leg_log),
        std::mem::take(&mut summary.vtm_leg_log),
        std::mem::take(&mut summary.vtm_cycle_log),
        std::mem::take(&mut summary.vtm_operation_log),
        std::mem::take(&mut summary.aisle_state_log),
        std::mem::take(&mut summary.aisle_flux_log),
    );
    let packed = document(summary, "summary statistics");
    summary.pick_log = held.0;
    summary.completed_order_log = held.1;
    summary.putaway_log = held.2;
    summary.htm_leg_log = held.3;
    summary.vtm_leg_log = held.4;
    summary.vtm_cycle_log = held.5;
    summary.vtm_operation_log = held.6;
    summary.aisle_state_log = held.7;
    summary.aisle_flux_log = held.8;
    packed
}

/// Pack one of the simulator's own documents for the recording to carry.
///
/// Field names go on the wire. The format's own records are positional
/// arrays, but these documents are not the format's: `SimConfig`, `SimResult`
/// and `SummaryStats` each default and skip fields, so a positional array of
/// one build's fields is read wrong by the next, and a recording written
/// before a field existed would come back shifted.
pub fn document<T: Serialize>(value: &T, what: &'static str) -> Result<Opaque, RecordingError> {
    rmp_serde::to_vec_named(value).map(Opaque).map_err(|source| RecordingError::Encoding { what, source })
}

/// Read one of those documents back into the simulator's own type.
pub fn read_document<T: DeserializeOwned>(opaque: &Opaque, what: &'static str) -> Result<T, String> {
    opaque.read(what).map_err(|e| e.to_string())
}
