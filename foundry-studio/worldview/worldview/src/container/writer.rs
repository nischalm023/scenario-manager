//! Appending frames to a file or a buffer, and closing one off.
//!
//! The writer's promise is that the index never names bytes the file may not
//! hold: it writes a frame, flushes it, and only then pushes that frame's
//! record. A reader opening the file between two appends therefore finds
//! either a whole frame or a torn tail it can drop, never a record pointing
//! at bytes that were never written. A write that fails part way stops the
//! writer for good, for the same reason. The ordering rules are enforced here
//! as well, because a producer that stamps a frame behind the one ahead of it
//! has a bug, and restamping it would hide the bug and break every seek past
//! that point.

#[cfg(not(target_arch = "wasm32"))]
use std::fs::{File, create_dir_all};
use std::io::{BufWriter, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use crate::container::Ms;
use crate::container::error::{Error, Result};
use crate::container::frame::{Frame, Tag};
use crate::container::index::{Index, IndexRecord, OrderCheck, Trailer};
use crate::container::payload::Payload;
use crate::container::sections::Sections;

/// An append-only sink that can be told to make what it has taken durable.
///
/// A file syncs; an in-memory buffer has nothing to sync and says so by doing
/// nothing. Without this the writer could only ever write to files, and the
/// tests and a producer teeing a run into memory both want the same code
/// path.
pub trait AppendSink: Write {
    fn durable(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl AppendSink for File {
    fn durable(&mut self) -> std::io::Result<()> {
        self.sync_data()
    }
}

impl AppendSink for Vec<u8> {}

impl<W: AppendSink> AppendSink for BufWriter<W> {
    fn durable(&mut self) -> std::io::Result<()> {
        self.flush()?;
        self.get_mut().durable()
    }
}

impl<W: AppendSink + ?Sized> AppendSink for &mut W {
    fn durable(&mut self) -> std::io::Result<()> {
        (**self).durable()
    }
}

impl<W: AppendSink + ?Sized> AppendSink for Box<W> {
    fn durable(&mut self) -> std::io::Result<()> {
        (**self).durable()
    }
}

/// Writes one run's `.worldview`.
#[derive(Debug)]
pub struct Writer<S: AppendSink> {
    sink: S,
    offset: u64,
    index: Index,
    order: OrderCheck,
    frames_written: u64,
    /// What the first write that did not land said. See [`Writer::guard`].
    failed: Option<String>,
    /// The section ids every keyframe and then every delta appended by its
    /// sections carries, once the file has declared them. See
    /// [`Writer::carry_sections`].
    carried: Option<(Vec<u8>, Vec<u8>)>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Writer<File> {
    /// Start a fresh file: see [`create_file`].
    pub fn create(path: &Path) -> Result<Self> {
        Ok(Self::new(create_file(path)?))
    }
}

/// A fresh file for a writer, truncating whatever was there and making the
/// folder it goes in when that folder does not exist yet — a run told to write
/// into a new output directory names one that nothing has made. A producer
/// teeing the file into another sink starts from this too.
#[cfg(not(target_arch = "wasm32"))]
pub fn create_file(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        create_dir_all(parent).map_err(|e| Error::io(format!("creating {}", parent.display()), e))?;
    }
    File::create(path).map_err(|e| Error::io(format!("creating {}", path.display()), e))
}

impl<S: AppendSink> Writer<S> {
    /// Start writing into a sink that holds nothing yet. Records count from
    /// byte zero of the sink, because that is where the scenario frame's
    /// magic goes.
    pub fn new(sink: S) -> Self {
        Self {
            sink,
            offset: 0,
            index: Index::new(),
            order: OrderCheck::default(),
            frames_written: 0,
            failed: None,
            carried: None,
        }
    }

    /// Hold every keyframe appended by its sections from here on to exactly
    /// the ids in `keyframe`, and every delta to exactly those in `delta`:
    /// the sections the file declared its frames of each kind carry. The
    /// container never learns what an id means; it keeps a file's frames of
    /// one kind from differing in which sections they carry, so a reader
    /// never has to tell a section that says nothing from one never written.
    pub fn carry_sections(&mut self, keyframe: Vec<u8>, delta: Vec<u8>) {
        self.carried = Some((keyframe, delta));
    }

    /// Write the scenario frame, which opens the file: the magic goes in
    /// ahead of the compressed body, and no other frame may come first.
    pub fn append_scenario(&mut self, t: Ms, body: &[u8]) -> Result<IndexRecord> {
        if self.offset != 0 {
            return Err(Error::ScenarioNotFirst { offset: self.offset });
        }
        self.append(&Frame::build(Tag::Scenario, t, body, true)?)
    }

    /// Encode a payload and append it as one frame.
    pub fn append_payload<P: Payload>(&mut self, tag: Tag, t: Ms, payload: &P) -> Result<IndexRecord> {
        self.append(&Frame::build(tag, t, &payload.to_bytes()?, true)?)
    }

    /// Lay sections out behind their table and append that as one frame —
    /// how every keyframe and every delta is written.
    pub fn append_sections(&mut self, tag: Tag, t: Ms, sections: &[(u8, &[u8])]) -> Result<IndexRecord> {
        let carried = match (&self.carried, tag) {
            (Some((keyframe, _)), Tag::Keyframe) => Some(keyframe),
            (Some((_, delta)), Tag::Delta) => Some(delta),
            _ => None,
        };
        if let Some(carried) = carried
            && !sections.iter().map(|(id, _)| *id).eq(carried.iter().copied())
        {
            return Err(Error::RefusedFrame {
                tag,
                detail: "a frame in a file carries exactly the sections the file declared for its kind",
            });
        }
        self.append(&Frame::build(tag, t, &Sections::encode(sections), true)?)
    }

    /// Append one frame: write it, flush it, then index it. The frames that
    /// close a file are not appended — [`Writer::finish`] writes those — and
    /// neither is a frame stored raw, because a file this writer makes holds
    /// one zstd frame per payload.
    pub fn append(&mut self, frame: &Frame) -> Result<IndexRecord> {
        if matches!(frame.header.tag, Tag::Index | Tag::End) {
            return Err(Error::RefusedFrame {
                tag: frame.header.tag,
                detail: "the index block and the end frame are what finish writes, and it writes them once",
            });
        }
        if !frame.header.compressed() {
            return Err(Error::RefusedFrame {
                tag: frame.header.tag,
                detail: "a stored payload is one zstd frame; a reader takes a raw one, a writer does not make one",
            });
        }
        self.append_one(frame)
    }

    fn append_one(&mut self, frame: &Frame) -> Result<IndexRecord> {
        if let Some(detail) = &self.failed {
            return Err(Error::WriterFailed { detail: detail.clone() });
        }
        if frame.header.tag == Tag::Scenario && self.offset != 0 {
            return Err(Error::ScenarioNotFirst { offset: self.offset });
        }
        if frame.header.tag != Tag::Scenario && self.offset == 0 {
            return Err(Error::ScenarioNotFirst { offset: 0 });
        }
        let mut order = self.order;
        order.admit(frame.header.tag, frame.header.t, self.offset)?;
        let record = IndexRecord {
            offset: self.offset,
            t: frame.header.t,
            tag: frame.header.tag,
            len: u32::try_from(frame.byte_len()).expect("a frame length fits in u32: the payload ceiling says so"),
        };
        self.write(&frame.to_bytes(), "appending a frame")?;
        self.flush()?;
        self.order = order;
        self.offset += frame.byte_len();
        self.frames_written += 1;
        self.index.push(record);
        Ok(record)
    }

    fn write(&mut self, bytes: &[u8], what: &'static str) -> Result<()> {
        let outcome = self.sink.write_all(bytes);
        self.guard(outcome, what)
    }

    fn flush(&mut self) -> Result<()> {
        let outcome = self.sink.flush();
        self.guard(outcome, "flushing the file")
    }

    /// Stop the writer at the first write that did not land, and keep what it
    /// said.
    ///
    /// `write_all` can hand some bytes to the sink and then fail — a disk
    /// filling up is the ordinary way that happens — and the bytes that
    /// landed stay in the file. A writer that carried on would put the next
    /// frame after them and record it where the failed one began, so every
    /// record from there on, and both of the trailer's offsets, would name
    /// bytes that are not the frame they describe. Stopping leaves a torn
    /// tail, which a reader drops, keeping everything written before it.
    fn guard(&mut self, outcome: std::io::Result<()>, what: &'static str) -> Result<()> {
        match outcome {
            Ok(()) => Ok(()),
            Err(source) => {
                let error = Error::io(what, source);
                self.failed = Some(error.to_string());
                Err(error)
            }
        }
    }

    /// Close the file: the index block naming every frame written so far,
    /// then the end frame, then the trailer that points at both. Nothing can
    /// be appended afterwards, and the sink comes back so a caller writing
    /// into memory gets its bytes.
    ///
    /// A producer putting frames on a socket never calls this — a stream has
    /// no end to seek back from, so it carries no trailer.
    pub fn finish(mut self, t_last: Ms, end_body: &[u8]) -> Result<S> {
        let block = self.index.block_bytes();
        let index_offset = self.offset;
        self.append_one(&Frame::build(Tag::Index, t_last, &block, true)?)?;
        let end_offset = self.offset;
        self.append_one(&Frame::build(Tag::End, t_last, end_body, true)?)?;
        let trailer = Trailer { index_offset, end_offset }.to_bytes();
        self.write(&trailer, "appending the trailer")?;
        self.flush()?;
        let outcome = self.sink.durable();
        self.guard(outcome, "syncing the file")?;
        self.offset += trailer.len() as u64;
        Ok(self.sink)
    }

    /// Every record written so far, in file order.
    pub fn index(&self) -> &Index {
        &self.index
    }

    /// Where the next frame will start.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn frames_written(&self) -> u64 {
        self.frames_written
    }
}

#[cfg(test)]
mod tests {
    use std::io::Error as IoError;

    use super::*;

    fn opened() -> Writer<Vec<u8>> {
        let mut writer = Writer::new(Vec::new());
        writer.append_scenario(Ms::ZERO, b"the run").expect("the scenario frame opens the file");
        writer
    }

    #[test]
    fn a_frame_beginning_before_the_one_ahead_of_it_is_refused_and_not_restamped() {
        let mut writer = opened();
        for ms in [0, 10_000, 20_000] {
            writer.append_sections(Tag::Delta, Ms(ms), &[(1, b"draw".as_slice())]).expect("forward is fine");
        }
        let error = writer
            .append_sections(Tag::Delta, Ms(9_000), &[(1, b"draw".as_slice())])
            .expect_err("a seek would read this one and every one after it wrong");
        assert!(matches!(error, Error::FrameOutOfOrder { t: Ms(9_000), previous: Ms(20_000), .. }), "got {error}");
        assert_eq!(writer.index().len(), 4, "the frame the writer refused is not in the index");
    }

    #[test]
    fn a_keyframe_behind_the_delta_of_its_own_instant_is_refused() {
        let mut writer = opened();
        writer.append_sections(Tag::Delta, Ms(30_000), &[(1, b"draw".as_slice())]).expect("the delta of a slice");
        let error = writer
            .append_sections(Tag::Keyframe, Ms(30_000), &[(1, b"draw".as_slice())])
            .expect_err("a seek to 30000 ms would step over the keyframe");
        assert!(matches!(error, Error::FrameOutOfOrder { t: Ms(30_000), .. }), "got {error}");
    }

    #[test]
    fn frames_sharing_an_instant_in_the_order_a_boundary_writes_them_are_the_ordinary_case() {
        let mut writer = opened();
        writer.append_sections(Tag::Delta, Ms(20_000), &[(1, b"before".as_slice())]).expect("the slice that ends");
        writer.append_sections(Tag::Keyframe, Ms(30_000), &[(1, b"whole".as_slice())]).expect("the boundary");
        writer.append_sections(Tag::Delta, Ms(30_000), &[(1, b"after".as_slice())]).expect("the slice that starts");
        assert_eq!(writer.index().len(), 4);
    }

    #[test]
    fn a_file_that_does_not_open_with_the_scenario_frame_is_refused() {
        let mut writer = Writer::new(Vec::new());
        let error = writer
            .append_sections(Tag::Keyframe, Ms::ZERO, &[(1, b"whole".as_slice())])
            .expect_err("the magic lives in the scenario frame");
        assert!(matches!(error, Error::ScenarioNotFirst { offset: 0 }), "got {error}");
    }

    #[test]
    fn a_second_scenario_frame_is_refused() {
        let mut writer = opened();
        let error = writer.append_scenario(Ms::ZERO, b"again").expect_err("one file, one scenario frame");
        assert!(matches!(error, Error::ScenarioNotFirst { .. }), "got {error}");
    }

    /// A sink that takes bytes until it is full and then fails part way
    /// through a write, the way a disk filling up does.
    #[derive(Debug)]
    struct Filling {
        taken: Vec<u8>,
        room: usize,
    }

    impl Write for Filling {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let left = self.room.saturating_sub(self.taken.len());
            if left == 0 {
                return Err(IoError::other("no space left on device"));
            }
            let take = left.min(buf.len());
            self.taken.extend_from_slice(&buf[..take]);
            Ok(take)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl AppendSink for Filling {}

    #[test]
    fn a_writer_whose_sink_fills_up_stops_for_good_rather_than_recording_the_wrong_offsets() {
        let mut writer = Writer::new(Filling { taken: Vec::new(), room: 200 });
        writer.append_scenario(Ms::ZERO, b"the run").expect("the scenario frame fits");
        let mut slice = 0u64;
        let failure = loop {
            let at = Ms(slice * 10_000);
            slice += 1;
            match writer.append_sections(Tag::Delta, at, &[(1, b"draw".as_slice())]) {
                Ok(_) => assert!(slice < 20, "the sink should have filled up by now"),
                Err(error) => break error,
            }
        };
        assert!(matches!(failure, Error::Io { .. }), "got {failure}");
        let written = writer.frames_written();

        let after = writer
            .append_sections(Tag::Delta, Ms(900_000), &[(1, b"draw".as_slice())])
            .expect_err("bytes of the failed frame landed, so every later offset would be wrong");
        assert!(matches!(after, Error::WriterFailed { .. }), "got {after}");
        assert_eq!(writer.frames_written(), written, "and nothing more reaches the index");
        let closing = writer.finish(Ms(900_000), b"stopped").expect_err("nor can the file be closed");
        assert!(matches!(closing, Error::WriterFailed { .. }), "got {closing}");
    }

    #[test]
    fn the_frames_that_close_a_file_are_refused_from_the_public_append() {
        for tag in [Tag::Index, Tag::End] {
            let mut writer = opened();
            let error = writer
                .append(&Frame::build(tag, Ms::ZERO, b"closing", true).expect("builds"))
                .expect_err("finish writes these, once");
            assert!(matches!(error, Error::RefusedFrame { .. }), "got {error}");
        }
    }

    #[test]
    fn a_frame_stored_raw_is_refused_because_a_file_this_writer_makes_is_zstd_throughout() {
        let mut writer = opened();
        let error = writer
            .append(&Frame::build(Tag::Delta, Ms::ZERO, b"draw", false).expect("builds"))
            .expect_err("a reader takes a raw frame, a writer does not make one");
        assert!(matches!(error, Error::RefusedFrame { tag: Tag::Delta, .. }), "got {error}");
    }

    #[test]
    fn once_a_file_declares_its_sections_a_frame_carrying_others_is_refused() {
        let mut writer = opened();
        writer.carry_sections(vec![1, 6], vec![1, 4, 6]);
        let error = writer
            .append_sections(Tag::Keyframe, Ms::ZERO, &[(1, b"whole".as_slice())])
            .expect_err("the file declared that its keyframes carry section 6 too");
        assert!(matches!(error, Error::RefusedFrame { tag: Tag::Keyframe, .. }), "got {error}");
        let error = writer
            .append_sections(Tag::Delta, Ms::ZERO, &[(1, b"".as_slice()), (4, b"".as_slice()), (5, b"".as_slice())])
            .expect_err("and its deltas no section 5");
        assert!(matches!(error, Error::RefusedFrame { tag: Tag::Delta, .. }), "got {error}");
        writer
            .append_sections(Tag::Keyframe, Ms::ZERO, &[(1, b"whole".as_slice()), (6, b"".as_slice())])
            .expect("exactly the declared sections, an empty one among them");
        assert_eq!(writer.frames_written(), 2, "only the scenario frame and the keyframe that carried them");
    }

    #[test]
    fn finishing_writes_the_index_block_the_end_frame_and_the_trailer() {
        let mut writer = opened();
        writer.append_sections(Tag::Keyframe, Ms::ZERO, &[(1, b"whole".as_slice())]).expect("a keyframe");
        let frames_before = writer.frames_written();
        let bytes = writer.finish(Ms(40_000), b"stopped").expect("closes");
        assert_eq!(frames_before, 2);
        let trailer = Trailer::read(&bytes[bytes.len() - 24..]).expect("a trailer sits at the end");
        assert!(trailer.index_offset < trailer.end_offset, "the block comes before the end frame");
    }
}
