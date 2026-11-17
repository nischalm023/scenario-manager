//! `inbound_lot`: the lots of goods that come to the site during a run, the input of putaway. `InboundLot` is generic
//! over the reader's type for an instant of world time, which must serialise as an integer of milliseconds.

use serde::{Deserialize, Serialize};

/// One lot of goods that comes to the site during the run: putaway's input, put away loose or inducted as built
/// totes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct InboundLot<WorldTime> {
    /// The lot's id, unique in its stream.
    pub id: String,
    /// The instant of world time from which the record is known to the scheduler: in scheduler time, no decision may
    /// use the record before the barrier, the world time below which the scheduler books no new claim, has reached
    /// this instant, and a run without the barrier refuses a record known after 0. Absent: 0.
    #[serde(rename = "known_ms", skip_serializing_if = "Option::is_none")]
    pub known: Option<WorldTime>,
    /// The world time at which the arrival is to happen: the lot's goods come to their station to be put or lifted, or
    /// the order arrives and may be worked. It is a no-earlier-than time: no task books work on the record at an
    /// earlier world time, and the arrival itself comes when the robots' timing allows, close to this time in a run
    /// that flows freely. It is independent of the world time from which the record is known and may be earlier than
    /// it; the record's work then waits until the record is known. Absent: the world time from which the record is
    /// known.
    #[serde(rename = "arrival_ms", skip_serializing_if = "Option::is_none")]
    pub arrival: Option<WorldTime>,
    /// The world time by which the record's work is due: a lenient deadline that gates nothing, and that a task may
    /// weigh as urgency. Absent: no deadline.
    #[serde(rename = "due_ms", skip_serializing_if = "Option::is_none")]
    pub due: Option<WorldTime>,
    /// The station the goods are pinned to, by the id its world document declares, optionally followed by `/` and a
    /// part of the station, which is not modelled. Absent: any station that takes them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
    /// How each built tote the lot comes in is divided; a line names its tote by the tote's index in this list.
    /// Absent: the lot comes in no built totes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(test, schemars(length(min = 1)))]
    pub totes: Vec<TotePartition>,
    /// What the lot brings, at least one line.
    #[cfg_attr(test, schemars(length(min = 1)))]
    pub lines: Vec<InboundLotLine>,
}

/// How a built tote is divided: whole, or into four compartments numbered 0 to 3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum TotePartition {
    Single,
    QuarterPartitioned,
}

/// Units of one SKU that a lot brings, loose or in one of its built totes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[cfg_attr(test, schemars(extend("dependentRequired" = {"compartment": ["tote_index"]})))]
#[serde(deny_unknown_fields)]
pub struct InboundLotLine {
    /// The SKU's id.
    pub sku: String,
    /// How many units, at least 1.
    #[cfg_attr(test, schemars(range(min = 1)))]
    pub quantity: u32,
    /// The built tote that holds the units, by its index in the lot's `totes`. Absent: the units come loose, for a
    /// put station.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tote_index: Option<u32>,
    /// The compartment of a quarter-partitioned tote that holds the units, stated only with that tote's index; not
    /// modelled. Absent: no compartment is named.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, schemars(range(max = 3)))]
    pub compartment: Option<u8>,
}
