//! The framing: one frame's twenty-byte header and the payload it stores.
//!
//! A frame is the unit everything else in the container is built from. It
//! stands alone — the header says how long the payload is and the checksum
//! says whether header and payload arrived whole — so a reader can start at
//! any frame offset, a file cut mid-write is a valid prefix, and the same
//! bytes travel over a socket without a file around them.

use std::fmt;

use crate::container::Ms;
use crate::container::compress::{MAX_DECOMPRESSED_LEN, compress, decompress};
use crate::container::error::{Error, Result};

/// The eleven bytes the scenario frame's stored payload opens with, ahead of
/// the zstd frame it compresses its body into: ten bytes of magic and then the
/// container version. Putting them inside the first frame rather than in a
/// file preamble is what lets one rule open a file and a stream alike.
///
/// The byte names a format that has reached master, and this one has not: it
/// stays where it is while the layout is still being settled, and moves the
/// first time a released format changes. A file whose byte is not this one is
/// refused, naming what it said, because a reader that guessed which reading a
/// block wanted would be deciding what a recording says. There is no shim.
pub const MAGIC: [u8; 10] = *b"WORLDVIEW\0";
pub const VERSION: u8 = 1;
pub const MAGIC_LEN: usize = MAGIC.len() + 1;

pub const HEADER_LEN: usize = 20;

/// Bit 0 of the flags byte: the stored payload is exactly one zstd frame. The
/// writer always sets it; a reader accepts it clear and takes the payload
/// raw. Any other bit set is damage.
pub const FLAG_ZSTD: u8 = 1 << 0;

/// The largest stored payload a frame may declare. A torn or altered length
/// field otherwise asks the reader for an allocation the file cannot fill;
/// the ceiling turns that into an error naming the offset, before a byte is
/// reserved.
pub const MAX_PAYLOAD_LEN: u32 = 64 * 1024 * 1024;

/// What a frame's payload means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tag {
    Scenario = 1,
    Declare = 2,
    Keyframe = 3,
    Delta = 4,
    Index = 5,
    End = 6,
}

impl Tag {
    pub fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            1 => Self::Scenario,
            2 => Self::Declare,
            3 => Self::Keyframe,
            4 => Self::Delta,
            5 => Self::Index,
            6 => Self::End,
            _ => return None,
        })
    }

    pub fn as_byte(self) -> u8 {
        self as u8
    }

    /// Whether this frame's payload opens with a section table. Keyframes and
    /// deltas do; every other frame carries one payload of its own.
    pub fn carries_sections(self) -> bool {
        matches!(self, Self::Keyframe | Self::Delta)
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            Self::Scenario => "scenario",
            Self::Declare => "declare",
            Self::Keyframe => "keyframe",
            Self::Delta => "delta",
            Self::Index => "index",
            Self::End => "end",
        })
    }
}

/// Read the eleven bytes a scenario payload opens with, refusing bytes this
/// crate does not read before anything is decompressed and naming what it
/// found instead. It is the same check off a disk and off a socket.
pub fn check_magic(bytes: &[u8]) -> Result<()> {
    let Some(opening) = bytes.get(..MAGIC_LEN) else {
        return Err(Error::NotAWorldview { found: describe(bytes) });
    };
    if opening[..MAGIC.len()] != MAGIC {
        return Err(Error::NotAWorldview { found: describe(&opening[..MAGIC.len()]) });
    }
    match opening[MAGIC.len()] {
        VERSION => Ok(()),
        found => Err(Error::UnsupportedVersion { found, expected: VERSION }),
    }
}

fn describe(bytes: &[u8]) -> String {
    let shown: String = bytes.iter().take(MAGIC.len()).map(|byte| byte.escape_ascii().to_string()).collect();
    format!("the first frame's payload begins \"{shown}\", not the WORLDVIEW magic")
}

/// The twenty bytes ahead of a payload: what the payload means, how it is
/// stored, the instant its contents begin at, and how to tell it arrived
/// whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameHeader {
    pub tag: Tag,
    pub flags: u8,
    pub t: Ms,
    /// Bytes of payload as stored — after compression, and including the
    /// magic when this is the scenario frame.
    pub payload_len: u32,
    /// crc32c over the sixteen header bytes ahead of it and then the stored
    /// payload.
    pub crc32c: u32,
}

impl FrameHeader {
    /// Read the twenty header bytes, refusing every field this container does
    /// not define and any length past the ceiling. The checksum is not checked
    /// here — it covers the payload too, so only [`Frame::from_parts`] can
    /// check it — which is what lets a reader see how long a payload claims to
    /// be before it reads a byte of one.
    pub fn decode_at(bytes: &[u8; HEADER_LEN], offset: u64) -> Result<Self> {
        let tag = Tag::from_byte(bytes[0]).ok_or(Error::UnknownTag { offset, tag: bytes[0] })?;
        let flags = bytes[1];
        if flags & !FLAG_ZSTD != 0 {
            return Err(Error::UnknownFlags { offset, flags });
        }
        let reserved = u16::from_le_bytes([bytes[2], bytes[3]]);
        if reserved != 0 {
            return Err(Error::ReservedNonZero { offset, reserved });
        }
        let t = Ms(u64::from_le_bytes(bytes[4..12].try_into().expect("eight bytes")));
        let payload_len = u32::from_le_bytes(bytes[12..16].try_into().expect("four bytes"));
        if payload_len > MAX_PAYLOAD_LEN {
            return Err(Error::PayloadTooLarge { offset, declared: u64::from(payload_len), limit: MAX_PAYLOAD_LEN });
        }
        let crc32c = u32::from_le_bytes(bytes[16..20].try_into().expect("four bytes"));
        Ok(Self { tag, flags, t, payload_len, crc32c })
    }

    pub fn compressed(self) -> bool {
        self.flags & FLAG_ZSTD != 0
    }

    /// Bytes of whole frame, header included.
    pub fn frame_len(self) -> u64 {
        HEADER_LEN as u64 + u64::from(self.payload_len)
    }

    fn to_bytes(self) -> [u8; HEADER_LEN] {
        let mut bytes = [0u8; HEADER_LEN];
        bytes[0] = self.tag.as_byte();
        bytes[1] = self.flags;
        // bytes[2..4] stay zero: reserved.
        bytes[4..12].copy_from_slice(&self.t.0.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.payload_len.to_le_bytes());
        bytes[16..20].copy_from_slice(&self.crc32c.to_le_bytes());
        bytes
    }
}

/// One frame: its header and its payload exactly as the file stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub header: FrameHeader,
    pub payload: Vec<u8>,
}

impl Frame {
    /// Build a frame around an already-encoded body, compressing it when
    /// asked and stamping the checksum. A scenario frame takes the magic
    /// ahead of the compressed body, and both are what the length and the
    /// checksum cover.
    pub fn build(tag: Tag, t: Ms, body: &[u8], zstd: bool) -> Result<Self> {
        // The ceiling that matters to a producer is the one the reader will
        // apply to what comes back out: a 64 MiB body of zeros compresses to
        // a few kilobytes, so a stored-size check alone would let a writer
        // make a frame its own reader refuses.
        if body.len() > MAX_DECOMPRESSED_LEN {
            return Err(Error::BodyTooLarge { len: body.len(), limit: MAX_DECOMPRESSED_LEN });
        }
        let mut stored = Vec::new();
        if tag == Tag::Scenario {
            stored.extend_from_slice(&MAGIC);
            stored.push(VERSION);
        }
        match zstd {
            true => stored.extend_from_slice(&compress(body)?),
            false => stored.extend_from_slice(body),
        }
        let declared = stored.len() as u64;
        if declared > u64::from(MAX_PAYLOAD_LEN) {
            return Err(Error::PayloadTooLarge { offset: 0, declared, limit: MAX_PAYLOAD_LEN });
        }
        let mut header =
            FrameHeader { tag, flags: if zstd { FLAG_ZSTD } else { 0 }, t, payload_len: declared as u32, crc32c: 0 };
        header.crc32c = checksum(&header.to_bytes()[..16], &stored);
        Ok(Self { header, payload: stored })
    }

    /// One frame's bytes, ready to put on a socket or into a file. A producer
    /// that streams frames and keeps no index writes these and nothing else.
    pub fn encode(tag: Tag, t: Ms, body: &[u8], zstd: bool) -> Result<Vec<u8>> {
        Ok(Self::build(tag, t, body, zstd)?.to_bytes())
    }

    /// Read one frame from the front of `bytes`, which begin at `offset` in
    /// the file. Returns the frame and how many bytes it consumed.
    pub fn decode_at(bytes: &[u8], offset: u64) -> Result<(Self, usize)> {
        let Some(head) = bytes.get(..HEADER_LEN) else {
            return Err(Error::ShortHeader { offset, available: bytes.len() });
        };
        let header = FrameHeader::decode_at(head.try_into().expect("twenty bytes"), offset)?;
        let end = HEADER_LEN + header.payload_len as usize;
        if bytes.len() < end {
            return Err(Error::ShortPayload {
                offset,
                declared: header.payload_len,
                available: bytes.len() - HEADER_LEN,
            });
        }
        Ok((Self::from_parts(header, bytes[HEADER_LEN..end].to_vec(), offset)?, end))
    }

    /// Build a frame from a header already read and the payload bytes that
    /// followed it, checking the length the header declared, the checksum it
    /// recorded, and — for the scenario frame — the magic the payload opens
    /// with, so a reader refuses a file it cannot read after thirty-one bytes.
    pub fn from_parts(header: FrameHeader, payload: Vec<u8>, offset: u64) -> Result<Self> {
        if payload.len() != header.payload_len as usize {
            return Err(Error::ShortPayload { offset, declared: header.payload_len, available: payload.len() });
        }
        let computed = checksum(&header.to_bytes()[..16], &payload);
        if computed != header.crc32c {
            return Err(Error::ChecksumMismatch { offset, recorded: header.crc32c, computed });
        }
        if header.tag == Tag::Scenario {
            check_magic(&payload)?;
        }
        Ok(Self { header, payload })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER_LEN + self.payload.len());
        bytes.extend_from_slice(&self.header.to_bytes());
        bytes.extend_from_slice(&self.payload);
        bytes
    }

    /// The body the producer handed the container: the stored payload
    /// decompressed, with the scenario frame's magic stripped off the front.
    pub fn contents(&self, offset: u64) -> Result<Vec<u8>> {
        let body = match self.header.tag {
            Tag::Scenario => self.payload.get(MAGIC_LEN..).ok_or_else(|| Error::NotAWorldview {
                found: format!("a scenario frame of {} bytes, short of the eleven the magic needs", self.payload.len()),
            })?,
            _ => &self.payload[..],
        };
        match self.header.compressed() {
            true => decompress(body, offset),
            false => Ok(body.to_vec()),
        }
    }

    pub fn byte_len(&self) -> u64 {
        self.header.frame_len()
    }
}

/// crc32c over the sixteen header bytes and then the stored payload, the one
/// checksum the container uses.
fn checksum(header_prefix: &[u8], payload: &[u8]) -> u32 {
    let partial = crc32c::crc32c(header_prefix);
    crc32c::crc32c_append(partial, payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(zstd: bool) -> Frame {
        Frame::build(Tag::Delta, Ms(30_000), &vec![7u8; 600], zstd).expect("builds")
    }

    #[test]
    fn a_frame_round_trips_uncompressed() {
        let frame = sample(false);
        let bytes = frame.to_bytes();
        assert_eq!(bytes.len(), HEADER_LEN + 600);
        let (read, consumed) = Frame::decode_at(&bytes, 0).expect("decodes");
        assert_eq!(consumed, bytes.len());
        assert_eq!(read, frame);
        assert_eq!(read.contents(0).expect("contents"), vec![7u8; 600]);
        assert!(!read.header.compressed());
    }

    #[test]
    fn a_frame_round_trips_compressed() {
        let frame = sample(true);
        assert!(frame.header.compressed());
        assert!(frame.payload.len() < 600, "600 identical bytes should shrink");
        let (read, _) = Frame::decode_at(&frame.to_bytes(), 0).expect("decodes");
        assert_eq!(read.contents(0).expect("contents"), vec![7u8; 600]);
    }

    #[test]
    fn the_scenario_frame_carries_the_magic_ahead_of_its_compressed_body() {
        let frame = Frame::build(Tag::Scenario, Ms::ZERO, b"the run", true).expect("builds");
        assert_eq!(&frame.payload[..MAGIC.len()], &MAGIC, "the magic is stored, not compressed");
        assert_eq!(frame.payload[MAGIC.len()], VERSION);
        assert_eq!(frame.contents(0).expect("contents"), b"the run", "and the body reads back without it");
    }

    #[test]
    fn a_scenario_frame_carrying_other_bytes_is_refused_by_magic() {
        let mut bytes = Frame::build(Tag::Scenario, Ms::ZERO, b"the run", true).expect("builds").to_bytes();
        bytes[HEADER_LEN] = b'X';
        let refixed = fix_checksum(bytes);
        let error = Frame::decode_at(&refixed, 0).expect_err("that is not a .worldview");
        assert!(matches!(error, Error::NotAWorldview { .. }), "got {error}");
    }

    #[test]
    fn a_scenario_frame_of_a_version_this_crate_does_not_read_says_which() {
        let mut bytes = Frame::build(Tag::Scenario, Ms::ZERO, b"the run", true).expect("builds").to_bytes();
        bytes[HEADER_LEN + MAGIC.len()] = 9;
        let error = Frame::decode_at(&fix_checksum(bytes), 0).expect_err("version 9 is not version 1");
        assert!(matches!(error, Error::UnsupportedVersion { found: 9, expected: VERSION }), "got {error}");
    }

    /// Re-stamp the checksum after a test has changed a payload byte, so the
    /// assertion below it is about the field it changed and not the crc.
    fn fix_checksum(mut bytes: Vec<u8>) -> Vec<u8> {
        let crc = checksum(&bytes[..16], &bytes[HEADER_LEN..]);
        bytes[16..20].copy_from_slice(&crc.to_le_bytes());
        bytes
    }

    #[test]
    fn a_flipped_payload_byte_is_a_checksum_error_naming_the_offset() {
        let mut bytes = sample(false).to_bytes();
        bytes[HEADER_LEN + 12] ^= 0x01;
        let error = Frame::decode_at(&bytes, 4_096).expect_err("the checksum should refuse it");
        assert!(matches!(error, Error::ChecksumMismatch { offset: 4_096, .. }), "got {error}");
    }

    #[test]
    fn a_payload_cut_short_names_the_offset_and_what_is_missing() {
        let bytes = sample(false).to_bytes();
        let error = Frame::decode_at(&bytes[..bytes.len() - 40], 1_024).expect_err("a cut payload is not a frame");
        assert!(matches!(error, Error::ShortPayload { offset: 1_024, declared: 600, available: 560 }), "got {error}");
    }

    #[test]
    fn a_header_cut_short_names_the_offset() {
        let error = Frame::decode_at(&sample(false).to_bytes()[..11], 64).expect_err("eleven bytes are not a header");
        assert!(matches!(error, Error::ShortHeader { offset: 64, available: 11 }), "got {error}");
    }

    #[test]
    fn an_undefined_flag_bit_is_refused() {
        let mut bytes = sample(false).to_bytes();
        bytes[1] = 0x80;
        let error = Frame::decode_at(&bytes, 0).expect_err("this container defines one flag bit");
        assert!(matches!(error, Error::UnknownFlags { flags: 0x80, .. }), "got {error}");
    }

    #[test]
    fn a_reserved_byte_that_is_not_zero_is_refused() {
        let mut bytes = sample(false).to_bytes();
        bytes[2] = 0x01;
        let error = Frame::decode_at(&bytes, 40).expect_err("the reserved bytes hold zero");
        assert!(matches!(error, Error::ReservedNonZero { offset: 40, reserved: 1 }), "got {error}");
    }

    #[test]
    fn an_unknown_tag_is_refused_before_any_allocation() {
        let mut bytes = sample(false).to_bytes();
        bytes[0] = 0x7f;
        let error = Frame::decode_at(&bytes, 8).expect_err("0x7f names no frame kind");
        assert!(matches!(error, Error::UnknownTag { offset: 8, tag: 0x7f }), "got {error}");
    }

    #[test]
    fn a_body_past_the_ceiling_its_reader_applies_is_refused_before_it_is_compressed() {
        let body = vec![0u8; MAX_DECOMPRESSED_LEN + 1];
        let error = Frame::build(Tag::Delta, Ms::ZERO, &body, true).expect_err("no reader would take it back");
        assert!(matches!(error, Error::BodyTooLarge { limit, .. } if limit == MAX_DECOMPRESSED_LEN), "got {error}");
    }

    #[test]
    fn a_scenario_frame_put_together_by_hand_without_the_magic_errors_rather_than_panicking() {
        let header = FrameHeader { tag: Tag::Scenario, flags: 0, t: Ms::ZERO, payload_len: 3, crc32c: 0 };
        let frame = Frame { header, payload: vec![1, 2, 3] };
        let error = frame.contents(0).expect_err("three bytes are not the eleven the magic needs");
        assert!(matches!(error, Error::NotAWorldview { .. }), "got {error}");
    }

    #[test]
    fn an_absurd_payload_length_is_refused_before_any_allocation() {
        let mut bytes = sample(false).to_bytes();
        bytes[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        let error = Frame::decode_at(&bytes, 8).expect_err("a torn length field must not ask for 4 GB");
        assert!(
            matches!(error, Error::PayloadTooLarge { offset: 8, declared, .. } if declared == u64::from(u32::MAX)),
            "got {error}"
        );
    }
}
