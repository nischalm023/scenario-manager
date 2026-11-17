//! Reading a `.worldview`: any frame by its offset, and any instant by
//! simulated time.
//!
//! A seek is planned from the index alone — which keyframe, which
//! declarations, then which deltas — before a byte of the frames is touched.
//! That separation is the point: a caller holding the whole file applies the
//! plan straight away, and a caller with only ranged access to storage
//! fetches exactly the byte ranges the plan names and decodes those.

#[cfg(not(target_arch = "wasm32"))]
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::iter::once;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use crate::container::Ms;
use crate::container::error::{Error, Result};
use crate::container::frame::{FLAG_ZSTD, Frame, FrameHeader, HEADER_LEN, MAGIC, MAGIC_LEN, Tag, VERSION};
use crate::container::index::{Index, IndexRecord, OrderCheck};
use crate::container::payload::Payload;
#[cfg(not(target_arch = "wasm32"))]
use crate::container::recover::load;
use crate::container::recover::{scan_source, through_trailer};
use crate::container::sections::Sections;

/// The frames a seek has to read, in the order to read them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeekPlan {
    pub keyframe: IndexRecord,
    /// Every declaration made at or before the instant asked for. They
    /// accumulate over a run, so a seek carries all of them and not only the
    /// ones after its keyframe.
    pub declares: Vec<IndexRecord>,
    /// The deltas after the keyframe, in file order, none of them stamped
    /// after the instant asked for. A consumer applies all but the last
    /// whole and interpolates within the last, whose slice holds the instant.
    pub deltas: Vec<IndexRecord>,
}

impl SeekPlan {
    /// The instant the plan reaches: the start of its last delta, or the
    /// keyframe's own instant when there is none.
    pub fn reaches(&self) -> Ms {
        self.deltas.last().map_or(self.keyframe.t, |delta| delta.t)
    }

    /// The byte ranges to fetch, in file order — what a ranged reader asks
    /// its storage for.
    pub fn ranges(&self) -> Vec<(u64, u32)> {
        let mut ranges: Vec<(u64, u32)> = once(&self.keyframe)
            .chain(&self.declares)
            .chain(&self.deltas)
            .map(|record| (record.offset, record.len))
            .collect();
        ranges.sort_unstable();
        ranges
    }
}

/// Reads frames and instants out of one `.worldview`.
#[derive(Debug)]
pub struct Reader<R: Read + Seek> {
    source: R,
    index: Index,
    verified_end: u64,
    order: OrderCheck,
}

#[cfg(not(target_arch = "wasm32"))]
impl Reader<File> {
    /// Open a file through its trailer when that trailer describes the file,
    /// and by scanning the frames when it does not — a live file, a crashed
    /// run, or a trailer that lies.
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|e| Error::io(format!("opening {}", path.display()), e))?;
        let loaded = load(path)?;
        Self::new(file, loaded.index)
    }
}

impl<'a> Reader<Cursor<&'a [u8]>> {
    /// Read a whole file already in memory, which is how a browser reads one.
    /// The index comes from scanning the frames.
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self> {
        let mut source = Cursor::new(bytes);
        if check_opening(&mut source)? == Opening::Incomplete {
            return Self::new(source, Index::new());
        }
        let index = match through_trailer(&mut source, bytes.len() as u64)? {
            Some((index, _verified_end)) => index,
            None => scan_source(&mut source, 0, &mut OrderCheck::default())?.0,
        };
        Self::new(Cursor::new(bytes), index)
    }
}

impl<R: Read + Seek> Reader<R> {
    /// Wrap a source whose index is already known, refusing bytes this crate
    /// does not read after the first thirty-one — the frame header and the
    /// magic behind it.
    pub fn new(mut source: R, index: Index) -> Result<Self> {
        check_opening(&mut source)?;
        let verified_end = index.end_offset().unwrap_or(0);
        let order = OrderCheck::resuming(index.records());
        Ok(Self { source, index, verified_end, order })
    }

    pub fn index(&self) -> &Index {
        &self.index
    }

    /// The last indexed frame's instant: where a live reader starts tailing.
    pub fn live_edge(&self) -> Option<Ms> {
        self.index.live_edge()
    }

    pub fn frame_at(&mut self, offset: u64) -> Result<Frame> {
        frame_at(&mut self.source, offset)
    }

    pub fn payload_at<P: Payload>(&mut self, record: &IndexRecord) -> Result<P> {
        let frame = self.frame_at(record.offset)?;
        P::from_bytes(&frame.contents(record.offset)?, record.offset)
    }

    /// The index block the file carries at its end, read from the frames
    /// themselves. `None` for a file that was never closed.
    pub fn index_block(&mut self) -> Result<Option<Index>> {
        let Some(record) = self.index.records().iter().rev().find(|record| record.tag == Tag::Index).copied() else {
            return Ok(None);
        };
        let frame = self.frame_at(record.offset)?;
        Ok(Some(Index::from_block(&frame.contents(record.offset)?, record.offset)?))
    }

    /// The sections of one keyframe or delta, over its decompressed payload.
    pub fn sections_at(&mut self, record: &IndexRecord) -> Result<Sections> {
        if !record.tag.carries_sections() {
            return Err(Error::NotASectionedFrame { offset: record.offset, tag: record.tag });
        }
        let frame = self.frame_at(record.offset)?;
        Sections::decode(frame.contents(record.offset)?, record.offset)
    }

    /// Every frame from `offset` onward, in order, each with the offset it was
    /// read from. The walk stops at a frame the file ends inside — the torn
    /// tail of a crashed run — and reports damage as the iterator's last item.
    pub fn frames_from(&mut self, offset: u64) -> FrameWalk<'_, R> {
        FrameWalk { reader: self, next: offset, done: false }
    }

    /// Which keyframe, declarations and deltas reach `t`, from the index
    /// alone. A `t` before the first keyframe resolves to that keyframe; a
    /// `t` beyond the last frame resolves to the last frame, and the caller
    /// tails from there.
    pub fn seek(&self, t: Ms) -> Result<SeekPlan> {
        let keyframe = match self.index.last_keyframe_at_or_before(t) {
            Some(record) => *record,
            None => *self.index.first_keyframe().ok_or(Error::NoKeyframe)?,
        };
        Ok(SeekPlan {
            declares: self.index.declares_through(t),
            deltas: self.index.deltas_after(keyframe.end_offset(), t),
            keyframe,
        })
    }

    /// Take in the frames a live file has grown since this reader last
    /// looked, and say how many arrived. A torn tail is not an error — the
    /// producer is mid-write and the bytes will be there next time — but
    /// damage is.
    pub fn refresh(&mut self) -> Result<usize> {
        if self.index.records().last().map(|record| record.tag) == Some(Tag::End) {
            return Ok(0);
        }
        let mut order = self.order;
        let (added, verified_end) = scan_source(&mut self.source, self.verified_end, &mut order)?;
        for record in added.records() {
            self.index.push(*record);
        }
        self.order = order;
        self.verified_end = verified_end;
        Ok(added.len())
    }
}

/// Whether a source's opening is all there: `Complete` once the first frame's
/// header and the eleven bytes behind it have arrived and are this
/// container's, `Incomplete` while fewer than thirty-one bytes have arrived
/// and every one that has is what a `.worldview` opens with. An incomplete
/// opening is a file whose first frame is still being written — nothing is
/// verified yet, and nothing is wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opening {
    Complete,
    Incomplete,
}

/// Read a source's first thirty-one bytes and put them to
/// [`check_opening_bytes`].
pub fn check_opening<R: Read + Seek>(source: &mut R) -> Result<Opening> {
    source.seek(SeekFrom::Start(0)).map_err(|e| Error::io("seeking to the first frame", e))?;
    let mut opening = Vec::with_capacity(HEADER_LEN + MAGIC_LEN);
    source
        .take((HEADER_LEN + MAGIC_LEN) as u64)
        .read_to_end(&mut opening)
        .map_err(|e| Error::io("reading the first frame's magic", e))?;
    check_opening_bytes(&opening)
}

/// Refuse a source this crate cannot read before any of it is decoded: the
/// first frame's twenty-byte header, which is the scenario frame's, and the
/// eleven bytes of magic and version its payload opens with behind it. The
/// rule holds over however many of those thirty-one bytes have arrived, so a
/// file being watched as it is written is told apart from one that is not a
/// `.worldview` at all.
pub fn check_opening_bytes(opening: &[u8]) -> Result<Opening> {
    // The one byte that says whether this is our file at all. A header field
    // that is wrong further in is a damaged `.worldview` and is named as
    // such; a first byte that is not the scenario tag is somebody else's
    // file, and saying so is what a person who opened the wrong path needs.
    if let Some(&tag) = opening.first()
        && tag != Tag::Scenario.as_byte()
    {
        let found = format!("the file begins \"{}\", and a .worldview opens with its scenario frame", shown(opening));
        return Err(Error::NotAWorldview { found });
    }
    let Some(head) = opening.get(..HEADER_LEN) else {
        return partial_header(opening);
    };
    let header = FrameHeader::decode_at(head.try_into().expect("twenty bytes"), 0)?;
    if header.tag != Tag::Scenario {
        return Err(Error::ScenarioNotFirst { offset: 0 });
    }
    if (header.payload_len as usize) < MAGIC_LEN {
        let found = format!(
            "a first frame declaring a {}-byte payload, short of the eleven the magic needs",
            header.payload_len
        );
        return Err(Error::NotAWorldview { found });
    }
    let mut expected = [0u8; MAGIC_LEN];
    expected[..MAGIC.len()].copy_from_slice(&MAGIC);
    expected[MAGIC.len()] = VERSION;
    let arrived = &opening[HEADER_LEN..];
    let magic = arrived.len().min(MAGIC.len());
    if arrived[..magic] != expected[..magic] {
        let found = format!("the first frame's payload begins \"{}\", not the WORLDVIEW magic", shown(arrived));
        return Err(Error::NotAWorldview { found });
    }
    match arrived.get(MAGIC.len()) {
        Some(&found) if found != VERSION => Err(Error::UnsupportedVersion { found, expected: VERSION }),
        Some(_) => Ok(Opening::Complete),
        None => Ok(Opening::Incomplete),
    }
}

/// What can be said about an opening whose header has not all arrived: the
/// fields that are there must be the ones a scenario frame's header holds.
fn partial_header(opening: &[u8]) -> Result<Opening> {
    if let Some(&flags) = opening.get(1)
        && flags & !FLAG_ZSTD != 0
    {
        return Err(Error::UnknownFlags { offset: 0, flags });
    }
    if let Some(reserved) = opening.get(2..4) {
        let reserved = u16::from_le_bytes(reserved.try_into().expect("two bytes"));
        if reserved != 0 {
            return Err(Error::ReservedNonZero { offset: 0, reserved });
        }
    }
    Ok(Opening::Incomplete)
}

fn shown(bytes: &[u8]) -> String {
    bytes.iter().take(MAGIC.len()).map(|byte| byte.escape_ascii().to_string()).collect()
}

/// Read the one frame whose header starts at `offset` in `source`, growing
/// the payload buffer as the bytes arrive rather than reserving what the
/// header declares.
///
/// Free of [`Reader`] because opening a file through its trailer reads single
/// frames before there is an index to build a `Reader` around.
pub fn frame_at<R: Read + Seek>(source: &mut R, offset: u64) -> Result<Frame> {
    source.seek(SeekFrom::Start(offset)).map_err(|e| Error::io(format!("seeking to offset {offset}"), e))?;
    let mut head = Vec::with_capacity(HEADER_LEN);
    source
        .take(HEADER_LEN as u64)
        .read_to_end(&mut head)
        .map_err(|e| Error::io(format!("reading the frame header at {offset}"), e))?;
    let Ok(head) = <[u8; HEADER_LEN]>::try_from(head.as_slice()) else {
        return Err(Error::ShortHeader { offset, available: head.len() });
    };
    let header = FrameHeader::decode_at(&head, offset)?;
    let mut payload = Vec::new();
    let read = source
        .take(u64::from(header.payload_len))
        .read_to_end(&mut payload)
        .map_err(|e| Error::io(format!("reading the frame payload at {offset}"), e))?;
    if read < header.payload_len as usize {
        return Err(Error::ShortPayload { offset, declared: header.payload_len, available: read });
    }
    Frame::from_parts(header, payload, offset)
}

pub struct FrameWalk<'a, R: Read + Seek> {
    reader: &'a mut Reader<R>,
    next: u64,
    done: bool,
}

impl<R: Read + Seek> Iterator for FrameWalk<'_, R> {
    type Item = Result<(u64, Frame)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let offset = self.next;
        match self.reader.frame_at(offset) {
            Ok(frame) => {
                self.next = offset + frame.byte_len();
                self.done = frame.header.tag == Tag::End;
                Some(Ok((offset, frame)))
            }
            // A header or a payload the file ends inside is the end of what
            // was written, which is where the walk stops rather than
            // complaining. Anything else is damage and is handed up.
            Err(Error::ShortHeader { .. } | Error::ShortPayload { .. }) => {
                self.done = true;
                None
            }
            Err(error) => {
                self.done = true;
                Some(Err(error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Error as IoError, ErrorKind};

    use super::*;

    /// A file whose first frame header claims a payload the file does not
    /// hold — a flipped byte in the length field is all it takes.
    fn overstating(declared: u32) -> Vec<u8> {
        let mut bytes = Frame::build(Tag::Scenario, Ms::ZERO, b"short", false).expect("builds").to_bytes();
        bytes[12..16].copy_from_slice(&declared.to_le_bytes());
        bytes
    }

    /// A source that hands over the first `sound` bytes and then fails, the
    /// way a file on a mount that goes away mid-read does.
    struct Fails {
        bytes: Vec<u8>,
        sound: usize,
        at: usize,
    }

    impl Read for Fails {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.at >= self.sound {
                return Err(IoError::new(ErrorKind::ConnectionReset, "the mount went away"));
            }
            let take = buf.len().min(self.sound - self.at);
            buf[..take].copy_from_slice(&self.bytes[self.at..self.at + take]);
            self.at += take;
            Ok(take)
        }
    }

    impl Seek for Fails {
        fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
            let SeekFrom::Start(at) = to else { panic!("the reader only ever seeks from the start") };
            self.at = at as usize;
            Ok(at)
        }
    }

    #[test]
    fn a_header_declaring_more_than_the_file_holds_says_how_much_is_really_there() {
        let bytes = overstating(60 * 1024 * 1024);
        let error = frame_at(&mut Cursor::new(bytes.as_slice()), 0).expect_err("the payload is not there");
        assert!(matches!(error, Error::ShortPayload { offset: 0, available: 16, .. }), "got {error}");
    }

    #[test]
    fn a_source_that_stops_answering_is_an_io_failure_and_not_a_short_frame() {
        let bytes = overstating(64);
        let mut source = Fails { sound: bytes.len(), at: 0, bytes };
        let error = frame_at(&mut source, 0).expect_err("the payload never arrives");
        assert!(matches!(error, Error::Io { .. }), "a read that failed is not a frame cut short: got {error}");
    }

    #[test]
    fn a_first_frame_that_is_not_the_scenario_frame_is_refused_however_its_payload_begins() {
        let mut body = MAGIC.to_vec();
        body.push(VERSION);
        let bytes = Frame::build(Tag::Delta, Ms::ZERO, &body, false).expect("builds").to_bytes();
        let Err(error) = Reader::new(Cursor::new(bytes.as_slice()), Index::new()) else {
            panic!("the magic belongs to the scenario frame, and a delta carrying it opens nothing");
        };
        assert!(matches!(error, Error::NotAWorldview { .. }), "got {error}");
    }

    #[test]
    fn an_opening_still_being_written_is_not_yet_a_refusal() {
        let whole = Frame::build(Tag::Scenario, Ms::ZERO, b"the run", true).expect("builds").to_bytes();
        for arrived in 0..HEADER_LEN + MAGIC_LEN {
            let opening = check_opening_bytes(&whole[..arrived]).unwrap_or_else(|e| panic!("{arrived} bytes in: {e}"));
            assert_eq!(opening, Opening::Incomplete, "{arrived} bytes is short of the thirty-one an opening takes");
        }
        assert_eq!(check_opening_bytes(&whole).expect("all there"), Opening::Complete);
    }

    #[test]
    fn a_source_that_is_not_a_worldview_is_refused_after_thirty_one_bytes() {
        let mut bytes = Frame::build(Tag::Scenario, Ms::ZERO, b"the run", true).expect("builds").to_bytes();
        bytes[HEADER_LEN..HEADER_LEN + MAGIC.len()].copy_from_slice(b"SOMETHING\0");
        let Err(error) = Reader::new(Cursor::new(bytes.as_slice()), Index::new()) else {
            panic!("those bytes are not this container's");
        };
        assert!(matches!(error, Error::NotAWorldview { .. }), "got {error}");
    }
}
