//! The record types of the typed streams: each a struct, and beside it the JSON Schema generated from that struct,
//! the contract a stream's writers and readers share. A type's name is also the name of its stream and of its two
//! files, so it says what one record is. A schema is closed, so a new field is a new version; it describes the type
//! and each field; and it holds no `null` and no `number`: a time is an integer of milliseconds of world time, a
//! quantity an unsigned integer of at most 32 bits, an identifier a string, and every integer states its range.
//!
//! A stream's bytes are held to the schema before a record is read. The struct reads what the schema has passed and
//! repeats none of its bounds: on its own it would take a `null`, a zero quantity or an empty list of lines, which
//! the schema refuses.
//!
//! World time is the simulated clock; scheduler time is the sequence in which the scheduler takes its decisions. A
//! type that carries times is generic over the reader's type for an instant of world time, so the contract names no
//! simulator type. A record's times mean the same in every type that carries them and are written out in each struct
//! with one description, as serde refuses to flatten fields into a closed struct; a test holds the copies equal.

mod customer_order;
mod inbound_lot;
#[cfg(test)]
mod schema;
mod sku;
mod starting_tote;

use serde::{Deserialize, Serialize};

pub use customer_order::CustomerOrder;
pub use inbound_lot::{InboundLot, InboundLotLine, TotePartition};
pub use sku::Sku;
pub use starting_tote::StartingTote;

/// Units of one SKU: what an order wants of it, or what a tote holds of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SkuQuantity {
    /// The SKU's id.
    pub sku: String,
    /// How many units, at least 1.
    #[cfg_attr(test, schemars(range(min = 1)))]
    pub quantity: u32,
}
