//! `customer_order`: the orders customers have placed, each with the world times from which it is known, at which it
//! is to arrive and by which it is due. `CustomerOrder` is generic over the reader's type for an instant of world time,
//! which must serialise as an integer of milliseconds.

use serde::{Deserialize, Serialize};

use super::SkuQuantity;

/// One order a customer has placed: the units of each SKU the customer wants. A site's flow picks the order as it
/// stands or forms pallets from it, and a task that forms pallets takes the orders in the sequence of their stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct CustomerOrder<WorldTime> {
    /// The order's id.
    pub id: String,
    /// Who the order is for, by id: a customer such as a forward distribution centre. Absent: the order names no
    /// customer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer: Option<String>,
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
    /// The station the order is pinned to, by the id its world document declares, optionally followed by `/` and a
    /// part of the station, which is not modelled. Absent: any station that takes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
    /// What the customer wants, at least one line.
    #[cfg_attr(test, schemars(length(min = 1)))]
    pub lines: Vec<SkuQuantity>,
}
