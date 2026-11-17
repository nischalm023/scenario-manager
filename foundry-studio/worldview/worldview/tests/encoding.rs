//! The bytes the container writes, pinned: the frame header, an index record
//! and the trailer against literal arrays, the section table against itself
//! under arbitrary input, and the rule that every record is a positional
//! MessagePack array.
//!
//! These are the assertions a change to the format has to walk through. The
//! layouts are normative, so they are written out here as the numbers they
//! are rather than derived from the code that produces them.

use proptest::prelude::*;
use serde::{Deserialize, Serialize};
use worldview::container::frame::{MAGIC_LEN, VERSION};
use worldview::container::index::{TRAILER_LEN, Trailer};
use worldview::{Error, Frame, Index, IndexRecord, MAGIC, Ms, Payload, Sections, Tag};

#[test]
fn the_frame_header_is_twenty_bytes_in_the_order_the_format_gives() {
    let frame = Frame::build(Tag::Keyframe, Ms(0x0102_0304), b"ab", false).expect("builds");
    let bytes = frame.to_bytes();
    assert_eq!(bytes.len(), 20 + 2);
    assert_eq!(bytes[0], 0x03, "tag: a keyframe is 3");
    assert_eq!(bytes[1], 0x00, "flags: this payload is not compressed");
    assert_eq!(&bytes[2..4], &[0x00, 0x00], "reserved");
    assert_eq!(&bytes[4..12], &[0x04, 0x03, 0x02, 0x01, 0, 0, 0, 0], "t_ms, little-endian");
    assert_eq!(&bytes[12..16], &[0x02, 0x00, 0x00, 0x00], "payload_len");
    assert_eq!(
        &bytes[16..20],
        &[0x99, 0x01, 0xc4, 0xe1],
        "crc32c (Castagnoli) over the first sixteen header bytes and then the payload, computed by hand"
    );
    assert_eq!(&bytes[20..], b"ab");
}

#[test]
fn the_scenario_frame_opens_with_eleven_uncompressed_bytes_of_magic_and_version() {
    let frame = Frame::build(Tag::Scenario, Ms::ZERO, b"the run", true).expect("builds");
    assert_eq!(&frame.payload[..MAGIC_LEN], b"WORLDVIEW\x00\x01");
    assert_eq!(MAGIC, *b"WORLDVIEW\0");
    assert_eq!(VERSION, 1, "the scenario block of 24 positions, robot positions in millimetres");
    assert_eq!(u64::from(frame.header.payload_len), frame.byte_len() - 20, "the length covers magic and zstd frame");
}

#[test]
fn an_index_record_is_twenty_four_bytes_in_the_order_the_format_gives() {
    let mut index = Index::new();
    index.push(IndexRecord { offset: 0x0b, t: Ms(0x0a), tag: Tag::Delta, len: 0x0c });
    let bytes = index.block_bytes();
    assert_eq!(
        bytes,
        vec![
            0x0b, 0, 0, 0, 0, 0, 0, 0, // offset
            0x0a, 0, 0, 0, 0, 0, 0, 0,    // t_ms
            0x04, // tag: a delta is 4
            0x00, 0x00, 0x00, // padding
            0x0c, 0, 0, 0, // len, the whole frame
        ]
    );
}

#[test]
fn the_trailer_is_twenty_four_bytes_of_magic_and_two_offsets() {
    let bytes = Trailer { index_offset: 0x0102, end_offset: 0x0304 }.to_bytes();
    assert_eq!(bytes.len(), TRAILER_LEN);
    assert_eq!(
        bytes,
        [
            b'W', b'V', b'E', b'N', b'D', 0, 0, 0, // magic
            0x02, 0x01, 0, 0, 0, 0, 0, 0, // where the index block starts
            0x04, 0x03, 0, 0, 0, 0, 0, 0, // where the end frame starts
        ]
    );
}

/// MessagePack marks a short array `0x90 | n` and a short map `0x80 | n`.
fn says_array(bytes: &[u8], what: &str) {
    let marker = bytes[0];
    assert!(
        (0x90..=0x9f).contains(&marker) || marker == 0xdc || marker == 0xdd,
        "{what} encodes as {marker:#04x}, which is not a MessagePack array marker"
    );
    assert!(
        !(0x80..=0x8f).contains(&marker) && marker != 0xde && marker != 0xdf,
        "{what} encodes as a map — a record must never carry its field names"
    );
}

/// A record shaped the way the vocabulary's records will be shaped.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Reading {
    at: Ms,
    metric: u32,
    value: i64,
}

impl Payload for Reading {
    const NAME: &'static str = "reading";
}

#[test]
fn a_payload_record_encodes_as_a_positional_array_and_a_quantity_as_its_integer() {
    let reading = Reading { at: Ms(7), metric: 2, value: -1 };
    let bytes = reading.to_bytes().expect("encodes");
    says_array(&bytes, "a reading");
    assert_eq!(bytes.len(), 4, "three positions and the array marker, nothing spent on names");
    assert_eq!(rmp_serde::to_vec(&Ms(7)).expect("encodes"), vec![0x07], "a newtype is its integer on the wire");
    assert_eq!(Reading::from_bytes(&bytes, 0).expect("decodes"), reading);
}

proptest! {
    /// Whatever bodies a producer lays out, each comes back under its own id.
    #[test]
    fn a_section_table_round_trips(bodies in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..64), 0..12)) {
        let owned: Vec<(u8, Vec<u8>)> =
            bodies.into_iter().enumerate().map(|(position, body)| (position as u8 * 3 + 1, body)).collect();
        let borrowed: Vec<(u8, &[u8])> = owned.iter().map(|(id, body)| (*id, body.as_slice())).collect();

        let read = Sections::decode(Sections::encode(&borrowed), 0).expect("what the encoder lays out, the decoder reads");
        prop_assert_eq!(read.len(), owned.len());
        for (id, body) in &owned {
            prop_assert_eq!(read.get(*id), Some(body.as_slice()));
        }
        prop_assert_eq!(read.iter().map(|(id, _)| id).collect::<Vec<_>>(), read.ids());
    }

    /// Arbitrary bytes are either a table the encoder would have produced, or
    /// damage naming the frame. Nothing in between, and nothing that panics.
    #[test]
    fn arbitrary_bytes_are_a_canonical_table_or_damage(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
        match Sections::decode(bytes.clone(), 512) {
            Ok(read) => {
                let owned: Vec<(u8, Vec<u8>)> = read.iter().map(|(id, body)| (id, body.to_vec())).collect();
                let borrowed: Vec<(u8, &[u8])> = owned.iter().map(|(id, body)| (*id, body.as_slice())).collect();
                prop_assert_eq!(Sections::encode(&borrowed), bytes, "one layout per set of sections");
            }
            Err(error) => prop_assert!(matches!(error, Error::BadSectionTable { offset: 512, .. }), "got {}", error),
        }
    }
}
