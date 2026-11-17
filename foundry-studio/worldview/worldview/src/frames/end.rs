//! How the run ended, and everything that only became true when it did.
//!
//! The end block closes a file: why the run stopped, the producer's own
//! result and summary documents, the final value of every metric, and the
//! tables whose rows carry no instant — a run-level table cannot be cut into
//! slices, so it is written once, here, in the same three fields a slice's
//! rows use — with any rows the run reported after its last slice went out.
//! It is stamped at the run's stop, the simtime its summary ends it at,
//! which is where the last slice ends: no slice begins at or after it.
//! A stream carries no end block until the run reaches its end.

use serde::{Deserialize, Serialize};

use crate::container::payload::{Opaque, Payload};
use crate::frames::tables::TableRows;

/// The last frame's contents.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct End {
    /// Why the run stopped, in the producer's own words.
    pub stop_reason: String,
    /// What the stop reason points at — a robot, a station, a deadlock — when
    /// it points at something.
    pub stop_detail: Option<String>,
    /// The run's result, as MessagePack this crate carries and does not read.
    pub result: Opaque,
    /// The summary statistics the workbook cannot derive from the tables.
    pub summary: Opaque,
    /// Every metric's final value, positional per the descriptors, nil where
    /// the run never stated one.
    pub cumulative_scalars: Vec<Option<f64>>,
    /// The run-level tables, whose rows carry no instant, and the rows the run
    /// reported after its last slice went out.
    pub tables: Vec<TableRows>,
    pub frames_written: u64,
}

impl Payload for End {
    const NAME: &'static str = "end block";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::tables::TableId;

    #[test]
    fn the_end_block_is_seven_positions_and_carries_the_tables() {
        let end = End {
            stop_reason: "reached the horizon".to_owned(),
            stop_detail: None,
            result: Opaque::of(&vec![1u32], "result").expect("encodes"),
            summary: Opaque::default(),
            cumulative_scalars: vec![Some(3.0), None],
            tables: vec![TableRows { table: TableId(9), row_count: 1, rows: vec![0x91, 0x01] }],
            frames_written: 42,
        };
        let bytes = end.to_bytes().expect("encodes");
        assert_eq!(bytes[0], 0x97, "seven positions");
        let read = End::from_bytes(&bytes, 0).expect("decodes");
        assert_eq!(read, end);
        assert_eq!(read.tables[0].row_count, 1);
    }
}
