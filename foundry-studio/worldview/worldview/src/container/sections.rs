//! The sections of one keyframe or delta payload: several independent bodies
//! under one frame, each reachable without decoding the others.
//!
//! A keyframe states everything at an instant and a delta states one slice's
//! changes, and both are made of parts a consumer may not want all of — what
//! the viewer draws, the chart values, the heat grid, the journal rows. The
//! table at the front of the payload says where each part sits, so a consumer
//! takes the ids it knows and steps over the rest. The container never learns
//! what an id means: the scenario block declares that, and this module deals
//! in `u8` ids and byte slices.

use std::ops::Range;

use crate::container::error::{Error, Result};

/// Bytes per table entry: the section id, then its offset and length.
const ENTRY_LEN: usize = 9;

/// The table and the bodies it names, over a payload already decompressed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sections {
    payload: Vec<u8>,
    entries: Vec<(u8, Range<usize>)>,
}

impl Sections {
    /// Lay out bodies behind a table, in the order the caller gave them.
    ///
    /// The ids must be sorted and distinct, and a caller that sorts its own
    /// sections wrongly has a bug in the producer rather than a damaged file,
    /// so this panics where [`Sections::decode`] would raise.
    pub fn encode(sections: &[(u8, &[u8])]) -> Vec<u8> {
        let count = u8::try_from(sections.len()).expect("a payload carries at most 255 sections");
        assert!(
            sections.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "sections are laid out in strictly ascending id: {:?}",
            sections.iter().map(|(id, _)| *id).collect::<Vec<_>>()
        );
        let table_len = 1 + sections.len() * ENTRY_LEN;
        let body_len: usize = sections.iter().map(|(_, body)| body.len()).sum();
        let mut bytes = Vec::with_capacity(table_len + body_len);
        bytes.push(count);
        let mut at = 0u32;
        for (id, body) in sections {
            let len = u32::try_from(body.len()).expect("a section body fits in the payload ceiling");
            bytes.push(*id);
            bytes.extend_from_slice(&at.to_le_bytes());
            bytes.extend_from_slice(&len.to_le_bytes());
            at += len;
        }
        for (_, body) in sections {
            bytes.extend_from_slice(body);
        }
        bytes
    }

    /// Read the table at the front of a decompressed payload, checking that
    /// the bodies it names are one contiguous run ending exactly where the
    /// payload does. `offset` is where the frame's header sits, so a
    /// complaint says which frame is wrong.
    ///
    /// Everything this refuses would otherwise hand a consumer somebody
    /// else's bytes under its own id: a gap hides bytes nothing accounts for,
    /// an overlap gives two sections the same bytes, and a repeated or
    /// descending id makes which body wins depend on how the consumer looks.
    pub fn decode(payload: Vec<u8>, offset: u64) -> Result<Self> {
        let damage = |detail: String| Error::BadSectionTable { offset, detail };
        let count = *payload.first().ok_or_else(|| damage("the payload is empty".to_owned()))? as usize;
        let table_len = 1 + count * ENTRY_LEN;
        if payload.len() < table_len {
            return Err(damage(format!("{count} entries need {table_len} bytes and the payload is {}", payload.len())));
        }
        let bodies_len = payload.len() - table_len;
        let mut entries: Vec<(u8, Range<usize>)> = Vec::with_capacity(count);
        let mut expected = 0usize;
        for position in 0..count {
            let entry = &payload[1 + position * ENTRY_LEN..1 + (position + 1) * ENTRY_LEN];
            let id = entry[0];
            let at = u32::from_le_bytes(entry[1..5].try_into().expect("four bytes")) as usize;
            let len = u32::from_le_bytes(entry[5..9].try_into().expect("four bytes")) as usize;
            if let Some((previous, _)) = entries.last()
                && *previous >= id
            {
                return Err(damage(format!("section {id} follows section {previous}, which is not ascending")));
            }
            if at != expected {
                return Err(damage(format!(
                    "section {id} starts at {at}, and the section before it ends at {expected}"
                )));
            }
            let end = at.checked_add(len).ok_or_else(|| damage(format!("section {id} is {len} bytes from {at}")))?;
            if end > bodies_len {
                return Err(damage(format!("section {id} ends at {end} and the bodies are {bodies_len} bytes")));
            }
            entries.push((id, table_len + at..table_len + end));
            expected = end;
        }
        if expected != bodies_len {
            return Err(damage(format!("the bodies are {bodies_len} bytes and the table accounts for {expected}")));
        }
        Ok(Self { payload, entries })
    }

    /// One section's bytes, or `None` when the frame carries no such id — a
    /// consumer asking for something this run does not record.
    pub fn get(&self, id: u8) -> Option<&[u8]> {
        self.entries.iter().find(|(entry, _)| *entry == id).map(|(_, range)| &self.payload[range.clone()])
    }

    /// Every section, in the ascending id order the payload lays them out in.
    pub fn iter(&self) -> impl Iterator<Item = (u8, &[u8])> {
        self.entries.iter().map(|(id, range)| (*id, &self.payload[range.clone()]))
    }

    pub fn ids(&self) -> Vec<u8> {
        self.entries.iter().map(|(id, _)| *id).collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded(bytes: Vec<u8>) -> Result<Sections> {
        Sections::decode(bytes, 4_096)
    }

    #[test]
    fn the_table_is_a_count_then_an_entry_of_id_offset_and_length() {
        let bytes = Sections::encode(&[(1, b"draw".as_slice()), (3, b"heat".as_slice())]);
        assert_eq!(bytes[0], 2, "count");
        assert_eq!(&bytes[1..10], &[1, 0, 0, 0, 0, 4, 0, 0, 0], "section 1 at offset 0, four bytes");
        assert_eq!(&bytes[10..19], &[3, 4, 0, 0, 0, 4, 0, 0, 0], "section 3 straight after it");
        assert_eq!(&bytes[19..], b"drawheat", "and the bodies follow the table, in order");
    }

    #[test]
    fn a_section_reads_back_by_its_id_and_an_id_the_frame_lacks_is_none() {
        let sections = decoded(Sections::encode(&[(2, b"values".as_slice()), (7, b"".as_slice())])).expect("decodes");
        assert_eq!(sections.get(2), Some(b"values".as_slice()));
        assert_eq!(sections.get(7), Some(b"".as_slice()), "an empty section is present and zero bytes long");
        assert_eq!(sections.get(4), None);
        assert_eq!(sections.ids(), vec![2, 7]);
    }

    #[test]
    fn a_payload_with_no_sections_at_all_is_one_zero_byte() {
        let bytes = Sections::encode(&[]);
        assert_eq!(bytes, vec![0]);
        assert!(decoded(bytes).expect("decodes").is_empty());
    }

    #[test]
    #[should_panic(expected = "ascending id")]
    fn laying_sections_out_in_the_wrong_order_is_a_producer_bug_and_panics() {
        Sections::encode(&[(5, b"later".as_slice()), (2, b"earlier".as_slice())]);
    }

    #[test]
    fn a_gap_between_two_bodies_is_damage_naming_the_frame() {
        let mut bytes = Sections::encode(&[(1, b"draw".as_slice()), (3, b"heat".as_slice())]);
        bytes[11] = 5;
        let error = decoded(bytes).expect_err("nothing accounts for the byte in the gap");
        assert!(matches!(error, Error::BadSectionTable { offset: 4_096, .. }), "got {error}");
    }

    #[test]
    fn two_entries_under_one_id_are_damage() {
        let mut bytes = Sections::encode(&[(1, b"draw".as_slice()), (3, b"heat".as_slice())]);
        bytes[10] = 1;
        assert!(matches!(decoded(bytes).unwrap_err(), Error::BadSectionTable { .. }));
    }

    #[test]
    fn ids_running_the_wrong_way_are_damage() {
        let mut bytes = Sections::encode(&[(1, b"draw".as_slice()), (3, b"heat".as_slice())]);
        bytes[1] = 3;
        bytes[10] = 1;
        let error = decoded(bytes).expect_err("which body a consumer gets must not depend on how it looks");
        assert!(matches!(error, Error::BadSectionTable { offset: 4_096, .. }), "got {error}");
    }

    #[test]
    fn two_bodies_over_the_same_bytes_are_damage() {
        let mut bytes = Sections::encode(&[(1, b"draw".as_slice()), (3, b"heat".as_slice())]);
        bytes[11..15].copy_from_slice(&2u32.to_le_bytes());
        let error = decoded(bytes).expect_err("section 3 would start inside section 1");
        assert!(matches!(error, Error::BadSectionTable { offset: 4_096, .. }), "got {error}");
    }

    #[test]
    fn an_entry_reaching_past_the_payload_is_damage() {
        let mut bytes = Sections::encode(&[(1, b"draw".as_slice())]);
        bytes[6..10].copy_from_slice(&999u32.to_le_bytes());
        assert!(matches!(decoded(bytes).unwrap_err(), Error::BadSectionTable { .. }));
    }

    #[test]
    fn a_count_the_payload_cannot_hold_is_damage_rather_than_a_read_past_the_end() {
        assert!(matches!(decoded(vec![9]).unwrap_err(), Error::BadSectionTable { .. }));
        assert!(matches!(decoded(Vec::new()).unwrap_err(), Error::BadSectionTable { .. }));
    }

    #[test]
    fn bytes_past_the_last_body_are_damage_and_not_padding() {
        let mut bytes = Sections::encode(&[(1, b"draw".as_slice())]);
        bytes.push(0);
        assert!(matches!(decoded(bytes).unwrap_err(), Error::BadSectionTable { .. }));
    }
}
