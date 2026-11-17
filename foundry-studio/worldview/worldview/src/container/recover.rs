//! Reading a file whose index block is missing or lying: the scan that walks
//! the frames themselves and rebuilds the records.
//!
//! A file with no trailer is a run still being written, or one that crashed.
//! Both are ordinary, because frames are appended and nothing already written
//! is ever rewritten. The scan ends in one of two ways, and they are not the
//! same thing. A frame the file ends inside is the torn tail of a crashed
//! run: it is dropped, and what came before it stands. A frame whose bytes
//! are all there and which still does not decode is damage, and the scan
//! raises it rather than pretending the file stops there — every frame after
//! it would otherwise vanish with no word said.
//!
//! What the scan verifies is the framing: the header's fields, the checksum
//! over the header and the stored payload, the scenario magic, and the order
//! the frames run in. It does not decompress a payload. A payload whose
//! checksum passes and whose zstd does not can only come from a writer
//! defect, never from a torn write, and decompressing every frame of a
//! crashed 400 MB file to open it is the wrong price; that failure is raised,
//! with the frame's offset, when the frame is read.
//!
//! Nothing here reserves memory from a length a file declares: each buffer
//! grows as the bytes actually arrive, so a torn length field costs the bytes
//! that are really there and not the ones it asks for.

#[cfg(not(target_arch = "wasm32"))]
use std::fs::{File, metadata};
use std::io::{Cursor, Read, Seek, SeekFrom};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use crate::container::error::{Error, Result};
use crate::container::frame::{Frame, FrameHeader, HEADER_LEN, Tag};
use crate::container::index::{Index, IndexRecord, OrderCheck, TRAILER_LEN, Trailer};
use crate::container::reader::frame_at;
#[cfg(not(target_arch = "wasm32"))]
use crate::container::reader::{Opening, check_opening};

/// Walk frames from `from` and index the ones that read whole, carrying on
/// the order rule in `order`. Returns the index and the offset the scan
/// stopped at; when that offset is short of the file's length, the bytes past
/// it are a torn frame, or the trailer, and either way they are not another
/// frame.
///
/// The scan stops after the end frame, which is the last frame a file holds.
/// A scenario frame is the first frame of a file and there is only one, so a
/// scan that starts at byte zero demands one there and refuses one anywhere
/// else — that is where the magic is, and a file without it is not this
/// container's.
pub fn scan_source<R: Read + Seek>(source: &mut R, from: u64, order: &mut OrderCheck) -> Result<(Index, u64)> {
    let (index, end, damage) = walk(source, from, order)?;
    match damage {
        Some(error) => Err(error),
        None => Ok((index, end)),
    }
}

/// Walk the frames of a file already in memory. See [`scan_source`].
pub fn scan(bytes: &[u8], from: u64, order: &mut OrderCheck) -> Result<(Index, u64)> {
    scan_source(&mut Cursor::new(bytes), from, order)
}

/// The frames verified from `from` up to the first damage, the offset that
/// damage sits at, and what it was.
///
/// This is for showing a person what a damaged recording still holds — the
/// listing someone looking at a crashed run wants. A reader never takes this
/// route: [`scan_source`] refuses damage, because reading past it would hand
/// a consumer frames whose place in the file is no longer known.
pub fn verified_prefix<R: Read + Seek>(
    source: &mut R,
    from: u64,
    order: &mut OrderCheck,
) -> Result<(Index, u64, Option<Error>)> {
    walk(source, from, order)
}

fn walk<R: Read + Seek>(source: &mut R, from: u64, order: &mut OrderCheck) -> Result<(Index, u64, Option<Error>)> {
    let mut index = Index::new();
    let mut offset = from;
    source.seek(SeekFrom::Start(from)).map_err(|e| Error::io(format!("seeking to offset {from}"), e))?;
    loop {
        let mut head = Vec::with_capacity(HEADER_LEN);
        (&mut *source)
            .take(HEADER_LEN as u64)
            .read_to_end(&mut head)
            .map_err(|e| Error::io(format!("reading the frame header at {offset}"), e))?;
        let Ok(head) = <[u8; HEADER_LEN]>::try_from(head.as_slice()) else {
            return Ok((index, offset, None));
        };
        let header = match FrameHeader::decode_at(&head, offset) {
            Ok(header) => header,
            Err(error) => return Ok((index, offset, Some(error))),
        };
        if (header.tag == Tag::Scenario) != (offset == 0) {
            return Ok((index, offset, Some(Error::ScenarioNotFirst { offset })));
        }
        let mut payload = Vec::new();
        let read = (&mut *source)
            .take(u64::from(header.payload_len))
            .read_to_end(&mut payload)
            .map_err(|e| Error::io(format!("reading the frame payload at {offset}"), e))?;
        if read < header.payload_len as usize {
            return Ok((index, offset, None));
        }
        let frame = match Frame::from_parts(header, payload, offset) {
            Ok(frame) => frame,
            Err(error) => return Ok((index, offset, Some(error))),
        };
        if let Err(error) = order.admit(frame.header.tag, frame.header.t, offset) {
            return Ok((index, offset, Some(error)));
        }
        let consumed = frame.byte_len();
        index.push(IndexRecord {
            offset,
            t: frame.header.t,
            tag: frame.header.tag,
            len: u32::try_from(consumed).expect("a frame length fits in u32: the payload ceiling guarantees it"),
        });
        offset += consumed;
        if frame.header.tag == Tag::End {
            return Ok((index, offset, None));
        }
    }
}

/// What reading a file's index found, and how.
///
/// Which way it came matters to anyone looking at a file: an index read
/// through the trailer is a run that closed cleanly, and one rebuilt by
/// scanning is a live file, a crashed one, or a file whose trailer does not
/// describe it.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub index: Index,
    /// The byte the verified frames end at. Short of the file's length when a
    /// torn tail or a trailer follows them.
    pub verified_end: u64,
    pub through_trailer: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl Loaded {
    /// Nothing verified yet: a file whose first frame is still being written.
    fn nothing() -> Self {
        Self { index: Index::new(), verified_end: 0, through_trailer: false }
    }
}

/// The index of a file: the index block when the trailer points at one that
/// describes the frames ahead of it, and a scan of the frames otherwise.
#[cfg(not(target_arch = "wasm32"))]
pub fn load(path: &Path) -> Result<Loaded> {
    let len = metadata(path).map_err(|e| Error::io(format!("measuring {}", path.display()), e))?.len();
    let mut file = File::open(path).map_err(|e| Error::io(format!("opening {}", path.display()), e))?;
    if check_opening(&mut file)? == Opening::Incomplete {
        return Ok(Loaded::nothing());
    }
    if let Some((index, verified_end)) = through_trailer(&mut file, len)? {
        return Ok(Loaded { index, verified_end, through_trailer: true });
    }
    let (index, verified_end) = scan_source(&mut file, 0, &mut OrderCheck::default())?;
    Ok(Loaded { index, verified_end, through_trailer: false })
}

/// Rebuild a file's index from its frames, ignoring any trailer.
#[cfg(not(target_arch = "wasm32"))]
pub fn recover(path: &Path) -> Result<Loaded> {
    let mut file = File::open(path).map_err(|e| Error::io(format!("opening {}", path.display()), e))?;
    if check_opening(&mut file)? == Opening::Incomplete {
        return Ok(Loaded::nothing());
    }
    let (index, verified_end) = scan_source(&mut file, 0, &mut OrderCheck::default())?;
    Ok(Loaded { index, verified_end, through_trailer: false })
}

/// The index a valid trailer leads to, or `None` when there is no trailer or
/// it does not describe this file.
///
/// Reading the block instead of the frames is only sound if the block is
/// exactly what a scan would have found, so everything the trailer claims is
/// checked against the file: the block sits where its own last record ends,
/// the end frame follows the block, the trailer follows the end frame, the
/// file stops there, and both closing frames carry on the order the block's
/// records run in. Anything else and the caller scans, which is always the
/// truth.
pub(crate) fn through_trailer<R: Read + Seek>(source: &mut R, len: u64) -> Result<Option<(Index, u64)>> {
    let Some(tail_start) = len.checked_sub(TRAILER_LEN as u64) else {
        return Ok(None);
    };
    source.seek(SeekFrom::Start(tail_start)).map_err(|e| Error::io("seeking to the trailer", e))?;
    let mut tail = [0u8; TRAILER_LEN];
    if source.read_exact(&mut tail).is_err() {
        return Ok(None);
    }
    let Some(trailer) = Trailer::read(&tail) else {
        return Ok(None);
    };
    let Ok(block) = frame_at(source, trailer.index_offset) else {
        return Ok(None);
    };
    let Ok(contents) = block.contents(trailer.index_offset) else {
        return Ok(None);
    };
    let Ok(mut index) = Index::from_block(&contents, trailer.index_offset) else {
        return Ok(None);
    };
    let Ok(end) = frame_at(source, trailer.end_offset) else {
        return Ok(None);
    };
    let describes_this_file = block.header.tag == Tag::Index
        && end.header.tag == Tag::End
        && index.end_offset() == Some(trailer.index_offset)
        && trailer.index_offset + block.byte_len() == trailer.end_offset
        && trailer.end_offset + end.byte_len() == tail_start;
    if !describes_this_file {
        return Ok(None);
    }
    let mut order = OrderCheck::resuming(index.records());
    for (frame, offset) in [(&block, trailer.index_offset), (&end, trailer.end_offset)] {
        if order.admit(frame.header.tag, frame.header.t, offset).is_err() {
            return Ok(None);
        }
        index.push(IndexRecord {
            offset,
            t: frame.header.t,
            tag: frame.header.tag,
            len: u32::try_from(frame.byte_len()).expect("a frame length fits in u32"),
        });
    }
    Ok(Some((index, tail_start)))
}
