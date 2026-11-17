//! The journals a run keeps — picks, putaways, legs, cycles, aisle states —
//! as rows a consumer can tabulate without knowing the producer's types.
//!
//! A table declares its columns once, in the scenario block or in a declare
//! frame, and a delta then carries the rows whose instant falls in that
//! slice. The rows travel as one block of bytes: positional MessagePack rows
//! in the table's column order, back to back with no outer container, so two
//! blocks of one table splice by concatenating their bytes and adding their
//! counts. A server can therefore cut a chunk out of a recording without
//! decoding a single row.
//!
//! A table whose rows carry no instant — a run-level summary the producer
//! keeps as a table — rides in the end block instead, where the same three
//! fields say the same thing.

use std::result::Result as StdResult;

use serde::{Deserialize, Serialize, Serializer};

use crate::container::error::{Error, Result};
use crate::container::payload::Payload;
use crate::frames::{byte_enum, in_id_order};

/// A table's identity within one run. Small and stable: the descriptors are
/// append-only, so an id never moves once the run has stated it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TableId(pub u16);

byte_enum! {
    /// The declared type of one column's cells.
    ColumnValueType as "column value type" {
        UInt = 1,
        Int = 2,
        Float = 3,
        Bool = 4,
        Text = 5,
        /// An absolute simulated instant, as milliseconds.
        SimTime = 6,
        /// An elapsed simulated span, as milliseconds.
        SimDuration = 7,
    }
}

byte_enum! {
    /// How a column's cell sits in a row. Declared rather than assumed, so a
    /// later version can change one column's encoding without changing what
    /// every other column means.
    ColumnEncoding as "column encoding" {
        /// The cell is the MessagePack scalar its value type names, at this
        /// column's position in the row's array.
        Positional = 1,
    }
}

/// One declared column of a table's rows.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TableColumn {
    pub name: String,
    pub value_type: ColumnValueType,
    pub encoding: ColumnEncoding,
}

/// A table's identity and shape: what it is called, what its columns are, and
/// which of them holds each row's instant.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TableDescriptor {
    pub id: TableId,
    pub name: String,
    pub columns: Vec<TableColumn>,
    /// The column position holding each row's instant, or nil for a table
    /// whose rows have no instant at all — those ride in the end block.
    pub instant_column: Option<u16>,
}

/// Rows of one table, as one frame states them: those due once B passed the
/// end of the frame's slice, or, in the end block, the rows of a run-level
/// table and those the run reported after its last slice went out.
///
/// `row_count` is authoritative: a decoder reads exactly that many rows out
/// of `rows` and stops.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableRows {
    pub table: TableId,
    pub row_count: u32,
    #[serde(with = "serde_bytes")]
    pub rows: Vec<u8>,
}

impl TableRows {
    pub fn new(table: TableId) -> Self {
        Self { table, row_count: 0, rows: Vec::new() }
    }

    /// Splice a later block of the same table onto this one: appending the
    /// bytes and adding the counts gives exactly what encoding both at once
    /// would have given.
    ///
    /// Both the table and the count come out of a file, so both are checked:
    /// two tables do not splice, and a count that passes what it can hold
    /// says the blocks are damaged rather than that the reader should carry
    /// on with a wrapped number of rows.
    pub fn splice(&mut self, later: &Self) -> Result<()> {
        if self.table != later.table {
            let detail = format!("blocks of table {} do not splice onto it", later.table.0);
            return Err(Error::BadSplice { table: self.table.0, detail });
        }
        let row_count = self.row_count.checked_add(later.row_count).ok_or_else(|| Error::BadSplice {
            table: self.table.0,
            detail: format!("{} rows and {} more pass what a row count holds", self.row_count, later.row_count),
        })?;
        self.rows.extend_from_slice(&later.rows);
        self.row_count = row_count;
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.row_count == 0
    }
}

/// Every table that had rows in one slice. A table with no rows in a slice
/// has no entry at all, and the entries are written in ascending table id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct TablesDelta(pub Vec<TableRows>);

impl Serialize for TablesDelta {
    fn serialize<S: Serializer>(&self, serializer: S) -> StdResult<S::Ok, S::Error> {
        in_id_order(&self.0, |rows| rows.table).serialize(serializer)
    }
}

impl TablesDelta {
    /// One table's rows in this slice, or `None` when the slice had none.
    pub fn get(&self, table: TableId) -> Option<&TableRows> {
        self.0.iter().find(|rows| rows.table == table)
    }
}

impl Payload for TablesDelta {
    const NAME: &'static str = "tables delta";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(table: u16, rows: &[u8], count: u32) -> TableRows {
        TableRows { table: TableId(table), row_count: count, rows: rows.to_vec() }
    }

    #[test]
    fn a_tables_delta_is_a_bare_array_of_table_id_row_count_and_row_bytes() {
        let delta = TablesDelta(vec![block(7, &[0x92, 0x01, 0x02], 1), block(2, &[0x92, 0x03, 0x04], 1)]);
        let bytes = delta.to_bytes().expect("encodes");
        assert_eq!(bytes[0], 0x92, "two entries, with no wrapper around them");
        assert_eq!(&bytes[1..4], &[0x93, 2, 1], "the lower table id first: three positions, id, row count");
        let read = TablesDelta::from_bytes(&bytes, 0).expect("decodes");
        assert_eq!(read.get(TableId(7)).expect("table 7 is there").row_count, 1);
    }

    #[test]
    fn two_blocks_of_one_table_splice_into_what_encoding_both_at_once_would_have_given() {
        let mut whole = block(3, &[0x92, 0x01, 0x02], 1);
        whole.splice(&block(3, &[0x92, 0x03, 0x04], 1)).expect("one table splices");
        assert_eq!(whole, block(3, &[0x92, 0x01, 0x02, 0x92, 0x03, 0x04], 2));

        let error = whole.splice(&block(4, &[], 0)).expect_err("another table does not");
        assert!(matches!(error, Error::BadSplice { table: 3, .. }), "got {error}");
        let mut topped = block(3, &[], u32::MAX);
        let error = topped.splice(&block(3, &[], 1)).expect_err("nor does a count that cannot hold the rows");
        assert!(matches!(error, Error::BadSplice { table: 3, .. }), "got {error}");
    }

    #[test]
    fn a_descriptor_names_the_column_that_holds_the_instant() {
        let descriptor = TableDescriptor {
            id: TableId(1),
            name: "fulfillment.picks".to_owned(),
            columns: vec![
                TableColumn {
                    name: "at".to_owned(),
                    value_type: ColumnValueType::SimTime,
                    encoding: ColumnEncoding::Positional,
                },
                TableColumn {
                    name: "units".to_owned(),
                    value_type: ColumnValueType::UInt,
                    encoding: ColumnEncoding::Positional,
                },
            ],
            instant_column: Some(0),
        };
        let bytes = rmp_serde::to_vec(&descriptor).expect("encodes");
        assert_eq!(bytes[0], 0x94, "four positions");
        assert_eq!(rmp_serde::from_slice::<TableDescriptor>(&bytes).expect("decodes"), descriptor);
        let column = rmp_serde::to_vec(&descriptor.columns[0]).expect("encodes");
        assert_eq!(&column[column.len() - 2..], &[6, 1], "the value type byte, then the encoding byte");
    }
}
