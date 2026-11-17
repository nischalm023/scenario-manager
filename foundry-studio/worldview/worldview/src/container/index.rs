//! Where every frame sits: the records a seek searches, the block that
//! carries them at the end of a finished file, and the trailer that points at
//! that block.
//!
//! The index is what makes reaching an instant cost the frames that instant
//! needs rather than the file ahead of it. The writer keeps it in memory as
//! it appends and writes it once, at the end; a reader that opens a file
//! without one — a live file, a crashed run — rebuilds the same records by
//! scanning. Either way the search is the same: one fixed-size record per
//! frame, in file order and therefore non-decreasing in simulated time.

use crate::container::Ms;
use crate::container::error::{Error, Result};
use crate::container::frame::{HEADER_LEN, MAGIC_LEN, Tag};

pub const RECORD_LEN: usize = 24;

/// The eight bytes the trailer opens with, and the trailer's whole length:
/// the magic, the index block's offset and the end frame's offset.
pub const TRAILER_MAGIC: [u8; 8] = *b"WVEND\0\0\0";
pub const TRAILER_LEN: usize = 24;

/// Where one frame sits and what is in it — everything a seek needs before
/// any of the file is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexRecord {
    pub offset: u64,
    pub t: Ms,
    pub tag: Tag,
    /// Frame length, header included.
    pub len: u32,
}

impl IndexRecord {
    /// The byte just past this frame — where the next frame starts.
    pub fn end_offset(&self) -> u64 {
        self.offset.saturating_add(u64::from(self.len))
    }

    fn to_bytes(self) -> [u8; RECORD_LEN] {
        let mut bytes = [0u8; RECORD_LEN];
        bytes[0..8].copy_from_slice(&self.offset.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.t.0.to_le_bytes());
        bytes[16] = self.tag.as_byte();
        // bytes[17..20] stay zero: padding, so a record is eight-byte aligned.
        bytes[20..24].copy_from_slice(&self.len.to_le_bytes());
        bytes
    }
}

/// Every frame's record, in file order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Index {
    records: Vec<IndexRecord>,
}

impl Index {
    pub fn new() -> Self {
        Self::default()
    }

    /// The index block's payload: every record, packed, nothing else.
    pub fn block_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.records.len() * RECORD_LEN);
        for record in &self.records {
            bytes.extend_from_slice(&record.to_bytes());
        }
        bytes
    }

    /// Read an index block's decompressed payload, refusing records that do
    /// not describe one unbroken run of frames from byte zero.
    ///
    /// A block is only worth trusting instead of a scan if it is exactly what
    /// a scan would have found, so everything a scan guarantees is checked
    /// here: frames end where the next begins, and time runs forward. A block
    /// with a gap would send every seek past the gap to the wrong offset.
    pub fn from_block(bytes: &[u8], offset: u64) -> Result<Self> {
        let damage = |detail: String| Error::BadIndexBlock { offset, detail };
        if !bytes.len().is_multiple_of(RECORD_LEN) {
            return Err(damage(format!("{} bytes is not a whole number of {RECORD_LEN}-byte records", bytes.len())));
        }
        let mut records: Vec<IndexRecord> = Vec::with_capacity(bytes.len() / RECORD_LEN);
        let mut expected = 0u64;
        let mut order = OrderCheck::default();
        for (position, chunk) in bytes.chunks_exact(RECORD_LEN).enumerate() {
            let at = u64::from_le_bytes(chunk[0..8].try_into().expect("eight bytes"));
            let t = Ms(u64::from_le_bytes(chunk[8..16].try_into().expect("eight bytes")));
            let tag = Tag::from_byte(chunk[16])
                .ok_or_else(|| damage(format!("record {position}: tag byte {:#04x} names no frame kind", chunk[16])))?;
            let len = u32::from_le_bytes(chunk[20..24].try_into().expect("four bytes"));
            if at != expected {
                return Err(damage(format!(
                    "record {position} names offset {at}, and the frame before it ends at {expected}"
                )));
            }
            // The floor on a frame's length is what makes the offsets climb:
            // without it a run of empty records all sits at offset zero, and
            // a seek then hands a ranged reader a zero-byte range for a real
            // frame.
            let floor = match position {
                0 => HEADER_LEN + MAGIC_LEN,
                _ => HEADER_LEN,
            };
            if (len as usize) < floor {
                return Err(damage(format!(
                    "record {position} claims a {len}-byte frame, and the shortest one is {floor}"
                )));
            }
            if (position == 0) != (tag == Tag::Scenario) {
                return Err(damage(format!(
                    "record {position} names a {tag} frame; a block opens with the one scenario frame and holds no other"
                )));
            }
            if matches!(tag, Tag::Index | Tag::End) {
                return Err(damage(format!(
                    "record {position} names a {tag} frame; a block lists the frames ahead of it, neither itself nor the end frame"
                )));
            }
            order.admit(tag, t, at).map_err(|e| damage(format!("record {position}: {e}")))?;
            let record = IndexRecord { offset: at, t, tag, len };
            expected = record.end_offset();
            records.push(record);
        }
        Ok(Self { records })
    }

    pub fn push(&mut self, record: IndexRecord) {
        self.records.push(record);
    }

    pub fn records(&self) -> &[IndexRecord] {
        &self.records
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The byte just past the last frame — where the next frame goes, and how
    /// far a scan has already accounted for.
    pub fn end_offset(&self) -> Option<u64> {
        self.records.last().map(IndexRecord::end_offset)
    }

    /// The simulated instant of the last frame: the live edge a reader tails
    /// from.
    pub fn live_edge(&self) -> Option<Ms> {
        self.records.last().map(|record| record.t)
    }

    /// The keyframe a seek starts from: the last one at or before `t`. `None`
    /// when `t` is before the first keyframe, which resolves to that first
    /// keyframe instead — the file states nothing earlier.
    pub fn last_keyframe_at_or_before(&self, t: Ms) -> Option<&IndexRecord> {
        // Records are non-decreasing in t, so the partition point bounds the
        // search and walking back from it passes at most one cadence of
        // deltas.
        let bound = self.records.partition_point(|record| record.t <= t);
        self.records[..bound].iter().rev().find(|record| record.tag == Tag::Keyframe)
    }

    pub fn first_keyframe(&self) -> Option<&IndexRecord> {
        self.records.iter().find(|record| record.tag == Tag::Keyframe)
    }

    /// Every declare frame in the file stamped at or before `t`, in file
    /// order. Declarations accumulate over the run, so a seek carries all of
    /// them and not only the ones after its keyframe.
    pub fn declares_through(&self, t: Ms) -> Vec<IndexRecord> {
        self.records.iter().filter(|record| record.tag == Tag::Declare && record.t <= t).copied().collect()
    }

    /// The deltas from `from_offset` onward stamped at or before `t`, in file
    /// order, stopping at the next keyframe — whose own deltas belong to a
    /// later seek.
    pub fn deltas_after(&self, from_offset: u64, t: Ms) -> Vec<IndexRecord> {
        let mut deltas = Vec::new();
        for record in self.records.iter().filter(|record| record.offset >= from_offset) {
            match record.tag {
                Tag::Keyframe => break,
                Tag::Delta if record.t <= t => deltas.push(*record),
                _ => {}
            }
        }
        deltas
    }
}

/// The order frames run in, checked one frame at a time.
///
/// A seek binary-searches records by simulated time, and applies a keyframe
/// and then the deltas after it, so two things have to hold or a seek returns
/// the wrong state and says nothing: time never runs backwards, and a
/// keyframe states an instant before the deltas of that instant. The writer
/// refuses a frame that breaks either, and every reader checks the same rule
/// over what it finds, whether it scanned the frames or read the block.
#[derive(Clone, Copy, Debug, Default)]
pub struct OrderCheck {
    last: Option<Ms>,
    delta_at: Option<Ms>,
}

impl OrderCheck {
    /// The state a file's frames leave behind them, so a reader that takes in
    /// more frames later — a trailer's two closing records, a live file's
    /// next appends — carries on checking where the earlier ones stopped
    /// instead of starting the rule again.
    pub fn resuming(records: &[IndexRecord]) -> Self {
        Self {
            last: records.last().map(|record| record.t),
            delta_at: records.iter().rev().find(|record| record.tag == Tag::Delta).map(|record| record.t),
        }
    }

    pub fn admit(&mut self, tag: Tag, t: Ms, offset: u64) -> Result<()> {
        if let Some(previous) = self.last
            && t < previous
        {
            return Err(Error::FrameOutOfOrder {
                offset,
                t,
                previous,
                detail: "a file runs forward in simulated time",
            });
        }
        if tag == Tag::Keyframe && self.delta_at == Some(t) {
            return Err(Error::FrameOutOfOrder {
                offset,
                t,
                previous: t,
                detail: "a keyframe states an instant ahead of the deltas stamped with it",
            });
        }
        if tag == Tag::Delta {
            self.delta_at = Some(t);
        }
        self.last = Some(t);
        Ok(())
    }
}

/// The twenty-four bytes after the end frame of a finished file: where the
/// index block is and where the end frame is, so opening a file costs one
/// read of its tail rather than a pass over it. Nothing on the wire carries
/// one — a stream has no end to seek back from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trailer {
    pub index_offset: u64,
    pub end_offset: u64,
}

impl Trailer {
    pub fn to_bytes(self) -> [u8; TRAILER_LEN] {
        let mut bytes = [0u8; TRAILER_LEN];
        bytes[0..8].copy_from_slice(&TRAILER_MAGIC);
        bytes[8..16].copy_from_slice(&self.index_offset.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.end_offset.to_le_bytes());
        bytes
    }

    /// Read the last bytes of a file as a trailer. `None` — not an error —
    /// when they are not one: a live file simply has not been closed yet, and
    /// the reader scans instead.
    pub fn read(tail: &[u8]) -> Option<Self> {
        if tail.len() != TRAILER_LEN || tail[..8] != TRAILER_MAGIC {
            return None;
        }
        Some(Self {
            index_offset: u64::from_le_bytes(tail[8..16].try_into().expect("eight bytes")),
            end_offset: u64::from_le_bytes(tail[16..24].try_into().expect("eight bytes")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Slice zero and every thirtieth slice carry a keyframe; the slices
    /// between carry deltas. Ten-second slices, as the cencora runs use.
    fn cadenced() -> Index {
        let mut index = Index::new();
        index.push(IndexRecord { offset: 0, t: Ms::ZERO, tag: Tag::Scenario, len: 100 });
        let mut offset = 100;
        for slice in 0..90u64 {
            let tag = if slice.is_multiple_of(30) { Tag::Keyframe } else { Tag::Delta };
            index.push(IndexRecord { offset, t: Ms(slice * 10_000), tag, len: 100 });
            offset += 100;
        }
        index
    }

    fn block_of(records: &[IndexRecord]) -> Vec<u8> {
        let mut index = Index::new();
        for record in records {
            index.push(*record);
        }
        index.block_bytes()
    }

    fn scenario_record() -> IndexRecord {
        IndexRecord { offset: 0, t: Ms::ZERO, tag: Tag::Scenario, len: 100 }
    }

    #[test]
    fn a_block_round_trips_through_its_bytes() {
        let index = cadenced();
        assert_eq!(Index::from_block(&index.block_bytes(), 9_000).expect("reads back"), index);
    }

    #[test]
    fn a_block_whose_frames_leave_a_gap_is_refused_rather_than_searched() {
        let bytes =
            block_of(&[scenario_record(), IndexRecord { offset: 140, t: Ms(10), tag: Tag::Keyframe, len: 100 }]);
        let error = Index::from_block(&bytes, 240).expect_err("40 bytes are unaccounted for");
        assert!(matches!(error, Error::BadIndexBlock { offset: 240, .. }), "got {error}");
    }

    #[test]
    fn a_block_whose_records_run_backwards_in_time_is_refused() {
        let bytes = block_of(&[
            scenario_record(),
            IndexRecord { offset: 100, t: Ms(60_000), tag: Tag::Keyframe, len: 100 },
            IndexRecord { offset: 200, t: Ms(20_000), tag: Tag::Delta, len: 100 },
        ]);
        assert!(matches!(Index::from_block(&bytes, 0).unwrap_err(), Error::BadIndexBlock { .. }));
    }

    #[test]
    fn a_block_of_frames_shorter_than_a_header_is_refused_so_the_offsets_climb() {
        let empty = block_of(&[IndexRecord { offset: 0, t: Ms::ZERO, tag: Tag::Scenario, len: 0 }]);
        assert!(matches!(Index::from_block(&empty, 0).unwrap_err(), Error::BadIndexBlock { .. }));

        let headerless =
            block_of(&[scenario_record(), IndexRecord { offset: 100, t: Ms::ZERO, tag: Tag::Keyframe, len: 19 }]);
        assert!(matches!(Index::from_block(&headerless, 0).unwrap_err(), Error::BadIndexBlock { .. }));

        let short_scenario = block_of(&[IndexRecord { offset: 0, t: Ms::ZERO, tag: Tag::Scenario, len: 30 }]);
        assert!(
            matches!(Index::from_block(&short_scenario, 0).unwrap_err(), Error::BadIndexBlock { .. }),
            "a scenario frame holds a header and eleven bytes of magic"
        );
    }

    #[test]
    fn a_block_that_does_not_open_with_one_scenario_record_is_refused() {
        let headless = block_of(&[IndexRecord { offset: 0, t: Ms::ZERO, tag: Tag::Keyframe, len: 100 }]);
        assert!(matches!(Index::from_block(&headless, 0).unwrap_err(), Error::BadIndexBlock { .. }));

        let twice =
            block_of(&[scenario_record(), IndexRecord { offset: 100, t: Ms::ZERO, tag: Tag::Scenario, len: 100 }]);
        assert!(matches!(Index::from_block(&twice, 0).unwrap_err(), Error::BadIndexBlock { .. }));
    }

    #[test]
    fn a_block_naming_an_index_or_end_frame_is_refused() {
        for tag in [Tag::Index, Tag::End] {
            let bytes = block_of(&[scenario_record(), IndexRecord { offset: 100, t: Ms::ZERO, tag, len: 100 }]);
            let error = Index::from_block(&bytes, 0).expect_err("a block lists the frames ahead of it");
            assert!(matches!(error, Error::BadIndexBlock { .. }), "got {error}");
        }
    }

    #[test]
    fn a_block_cut_mid_record_is_refused_by_size() {
        let index = cadenced();
        let mut bytes = index.block_bytes();
        bytes.truncate(bytes.len() - 9);
        assert!(matches!(Index::from_block(&bytes, 0).unwrap_err(), Error::BadIndexBlock { .. }));
    }

    #[test]
    fn the_search_lands_on_the_right_keyframe_at_every_boundary() {
        let index = cadenced();
        let at = |ms: u64| index.last_keyframe_at_or_before(Ms(ms)).map(|record| record.t);

        assert_eq!(at(0), Some(Ms::ZERO), "slice zero is itself a keyframe");
        assert_eq!(at(300_000), Some(Ms(300_000)), "exactly at the slice-30 keyframe");
        assert_eq!(at(305_000), Some(Ms(300_000)), "mid-slice resolves to the keyframe before it");
        assert_eq!(at(299_999), Some(Ms::ZERO), "a millisecond before the keyframe is still the previous one");
        assert_eq!(
            at(5_000_000),
            Some(Ms(600_000)),
            "past the live edge, the last keyframe stands and the caller tails"
        );
    }

    #[test]
    fn the_deltas_after_a_keyframe_stop_at_the_instant_asked_for() {
        let index = cadenced();
        let keyframe = index.last_keyframe_at_or_before(Ms(45_000)).expect("keyframe");
        let deltas = index.deltas_after(keyframe.end_offset(), Ms(45_000));
        assert_eq!(deltas.len(), 4, "slices 1 through 4");
        assert_eq!(deltas[0].t, Ms(10_000));
        assert_eq!(deltas[3].t, Ms(40_000));
    }

    #[test]
    fn a_trailer_reads_back_and_other_tail_bytes_are_simply_not_one() {
        let trailer = Trailer { index_offset: 4_096, end_offset: 5_000 };
        assert_eq!(Trailer::read(&trailer.to_bytes()), Some(trailer));
        assert_eq!(Trailer::read(&[0xab; TRAILER_LEN]), None, "a live file has no trailer and that is not an error");
        assert_eq!(Trailer::read(b"too short"), None);
    }
}
