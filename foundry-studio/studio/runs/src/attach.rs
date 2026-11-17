//! Within `crucible`, `attach` is ONE BROWSER'S READING OF ONE RECORDING: its
//! own reader over the stored file, and the frames from the instant it asked
//! for to the end of the run.
//!
//! Every attachment reads for itself. Nothing is shared, nothing is broadcast,
//! and a browser that reads slowly slows its own reader and no one else's —
//! which is what lets the server drop that one attachment, and only that one,
//! when it stops keeping up.
//!
//! Two things are done to the frames on the way out, and nothing else is. The
//! scenario block is FILTERED: the run's data files and the producer's own
//! rebuild document are taken out, every declaration made at or before the
//! instant asked for is folded into the descriptors, and the compiled floor and
//! census are put in. A keyframe or a delta is RE-ENCODED with only the
//! sections the attachment asked for, under the same tag and the same instant.
//! Everything else — a declaration made later, the end block — is forwarded as
//! the bytes the file holds, so the crate that wrote them reads them back.

use std::fs::File;
use std::path::Path;

use worldview::container::frame::Frame;
use worldview::container::index::{Index, IndexRecord};
use worldview::frames::end::End;
use worldview::frames::{HEAT, MOTION, PLAYBACK, SCALARS, SECTIONS, section_name};
use worldview::{Declare, Error as WorldviewError, Ms, Payload, Reader, Scenario, Sections, Tag};

use crate::floor::Scene;
use crate::protocol::AttachFrom;

/// The sections an attachment that named none is sent: what the viewer draws,
/// and the values the charts are made of.
pub const DEFAULT_SECTIONS: [u8; 2] = [PLAYBACK, SCALARS];

/// Every section a browser may ask for. `tables` is absent on purpose: it is
/// the run's journal, the largest thing in a recording by far, and nothing a
/// browser draws reads it. A request naming it is refused rather than quietly
/// dropped, so a client learns the rule from the server.
pub const FOR_A_BROWSER: [u8; 4] = [PLAYBACK, SCALARS, HEAT, MOTION];

/// Which sections of each frame this attachment wants, in ascending id and
/// without repeats — the order a section table is laid out in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted(Vec<u8>);

/// What an attachment can fail to open.
#[derive(Debug, thiserror::Error)]
pub enum AttachError {
    #[error("`{name}` is not a section this format has; the sections are {}", named(&SECTIONS))]
    NoSuchSection { name: String },
    #[error("`{name}` does not travel to a browser; what does is {}", named_ids(&FOR_A_BROWSER))]
    NotForABrowser { name: String },
    #[error("{0}")]
    Recording(#[from] WorldviewError),
    #[error("{0}")]
    Damaged(#[from] ReadFailed),
}

impl Wanted {
    /// Read the names a browser asked for. An empty list is the default set —
    /// a browser that says nothing about sections gets what it needs to draw.
    pub fn parse(names: &[String]) -> Result<Self, AttachError> {
        if names.is_empty() {
            return Ok(Self(DEFAULT_SECTIONS.to_vec()));
        }
        let mut ids = Vec::with_capacity(names.len());
        for name in names {
            let (id, _) = SECTIONS
                .iter()
                .find(|(_, known)| *known == name)
                .ok_or_else(|| AttachError::NoSuchSection { name: name.clone() })?;
            if !FOR_A_BROWSER.contains(id) {
                return Err(AttachError::NotForABrowser { name: name.clone() });
            }
            ids.push(*id);
        }
        ids.sort_unstable();
        ids.dedup();
        Ok(Self(ids))
    }
}

/// A list of sections in words, for an error that has to say what it would
/// have accepted.
fn named(ids: &[(u8, &str)]) -> String {
    ids.iter().map(|(_, name)| *name).collect::<Vec<_>>().join(", ")
}

/// The same, over bare ids.
fn named_ids(ids: &[u8]) -> String {
    ids.iter().filter_map(|id| section_name(*id)).collect::<Vec<_>>().join(", ")
}

/// What the browser is told before the first frame arrives.
#[derive(Debug, Clone, Copy)]
pub struct Opened {
    /// The instant this attachment answers at: what was asked for, or the run's
    /// last instant when that is earlier. A browser that asked for an hour into
    /// a ten-minute run is told where it really landed rather than being left
    /// to set its cursor past the end.
    pub from_ms: u64,
    /// The last instant the recording reaches.
    pub live_edge_ms: u64,
}

/// One attachment's reader, and where it has got to.
pub struct Attachment {
    reader: Reader<File>,
    /// The frames still to send, in the order to send them: the keyframe, then
    /// the deltas and later declarations, then the end block.
    records: Vec<IndexRecord>,
    at: usize,
    /// The two frames the server builds rather than forwards, sent ahead of the
    /// records: the filtered scenario block and the partial index.
    opening: Vec<Vec<u8>>,
    wanted: Wanted,
    /// Why the run stopped, in the recording's own words — the end block's, not
    /// the registry's, because that is the spelling every other reader of this
    /// file sees.
    outcome: String,
}

/// One frame on its way to a browser, and the instant it states.
pub struct Sent {
    pub t_ms: u64,
    pub bytes: Vec<u8>,
}

/// A frame of the recording that would not read, and where in the file it sits.
///
/// The offset is kept beside the error because "the recording could not be
/// read" is not something anyone can act on: what a person needs is which run,
/// which attempt of it, and which frame — and the first two belong to the
/// caller, which puts them in front of this.
#[derive(Debug, thiserror::Error)]
#[error("the frame at offset {offset}: {source}")]
pub struct ReadFailed {
    pub offset: u64,
    #[source]
    pub source: WorldviewError,
}

impl Attachment {
    /// Open a reader over a finished recording at the instant asked for.
    pub fn open(
        recording: &Path,
        from: &AttachFrom,
        wanted: Wanted,
        scene: Scene,
    ) -> Result<(Opened, Self), AttachError> {
        let mut reader = Reader::open(recording)?;
        let live_edge = reader.live_edge().unwrap_or(Ms::ZERO);
        let from = resolve(from, live_edge).min(live_edge);
        let plan = reader.seek(from)?;

        let scenario = filtered_scenario(&mut reader, from, scene)?;
        let opening = vec![
            Frame::encode(Tag::Scenario, Ms::ZERO, &scenario.to_bytes()?, true)?,
            // Stamped at zero like the scenario block it follows: the records in
            // it reach `from`, but a frame stamped there would run ahead of the
            // keyframe behind it, and a consumer checking that frames only ever
            // move forward would refuse the stream.
            Frame::encode(Tag::Index, Ms::ZERO, &records_through(reader.index(), from).block_bytes(), true)?,
        ];

        let records = to_send(reader.index(), &plan.keyframe, from);
        let outcome = stop_reason(&mut reader)?;
        let opened = Opened { from_ms: from.0, live_edge_ms: live_edge.0 };
        Ok((opened, Self { reader, records, at: 0, opening, wanted, outcome }))
    }

    /// Why the run this attachment is reading stopped, as its end block says.
    pub fn outcome(&self) -> &str {
        &self.outcome
    }

    /// The next frame to send, or `None` once the end block has gone.
    pub fn next_frame(&mut self) -> Option<Result<Sent, ReadFailed>> {
        if !self.opening.is_empty() {
            return Some(Ok(Sent { t_ms: 0, bytes: self.opening.remove(0) }));
        }
        let record = *self.records.get(self.at)?;
        self.at += 1;
        Some(
            self.encode(&record)
                .map(|bytes| Sent { t_ms: record.t.0, bytes })
                .map_err(|source| ReadFailed { offset: record.offset, source }),
        )
    }

    /// One record as the browser receives it: a keyframe or a delta cut down to
    /// the asked sections, anything else byte for byte.
    fn encode(&mut self, record: &IndexRecord) -> Result<Vec<u8>, WorldviewError> {
        let frame = self.reader.frame_at(record.offset)?;
        if !record.tag.carries_sections() {
            return Ok(frame.to_bytes());
        }
        let sections = Sections::decode(frame.contents(record.offset)?, record.offset)?;
        let kept: Vec<(u8, &[u8])> =
            self.wanted.0.iter().filter_map(|id| sections.get(*id).map(|body| (*id, body))).collect();
        Frame::encode(record.tag, record.t, &Sections::encode(&kept), true)
    }
}

/// Why the run stopped, read from the end block the file closes with. A
/// recording with no end block is one the run never finished writing, and it
/// says so rather than inventing a reason.
fn stop_reason(reader: &mut Reader<File>) -> Result<String, AttachError> {
    let Some(record) = reader.index().records().iter().rev().find(|record| record.tag == Tag::End).copied() else {
        return Ok("the run left no end block".to_string());
    };
    let end: End = reader.payload_at(&record).map_err(|source| ReadFailed { offset: record.offset, source })?;
    Ok(end.stop_reason)
}

/// The instant an attachment starts at. The live edge over a FINISHED recording
/// is the run's last instant: a viewer opening on a run that has stopped opens
/// where it stopped.
fn resolve(from: &AttachFrom, live_edge: Ms) -> Ms {
    match from {
        AttachFrom::At(ms) => Ms(*ms),
        AttachFrom::Live => live_edge,
    }
}

/// The scenario block as a browser receives it.
fn filtered_scenario(reader: &mut Reader<File>, from: Ms, scene: Scene) -> Result<Scenario, WorldviewError> {
    let mut scenario = Scenario::read_from(reader)?;
    for record in reader.index().declares_through(from) {
        match reader.payload_at::<Declare>(&record)? {
            Declare::Metric(descriptor) => scenario.metrics.push(descriptor),
            Declare::Table(descriptor) => scenario.tables.push(descriptor),
        }
    }
    // The data files are the workload a run was given, tens of megabytes of
    // it, and nothing a browser draws reads them; the producer document is the
    // simulator's own rebuild state, which a browser has no simulator to
    // rebuild into. The world document stays: it is what the entity explorer
    // reads, and the floor beside it is what the browser cannot compile.
    scenario.data = None;
    scenario.producer = None;
    scenario.floor = Some(scene.floor);
    scenario.census = Some(scene.census);
    Ok(scenario)
}

/// The partial index: every record the file states at or before `from`, which
/// is a prefix of the file and therefore an index block in its own right.
///
/// **On a wire, only `t` and `tag` mean anything.** An index record's offset
/// and length describe where a frame sits in the FILE, and a browser has no
/// file: it is being sent re-encoded frames over a socket. What the block is
/// for there is the list of instants the run has — which ones carry a keyframe,
/// where the declarations fall — so a viewer can offer a scrub bar before a
/// single frame has arrived. The two records that close a file describe the
/// file rather than the run, and a block naming either is refused by the reader
/// that would read it, so they are left out.
fn records_through(index: &Index, from: Ms) -> Index {
    let mut partial = Index::new();
    for record in index.records().iter().take_while(|record| record.t <= from) {
        if matches!(record.tag, Tag::Index | Tag::End) {
            break;
        }
        partial.push(*record);
    }
    partial
}

/// Which records this attachment sends, in file order: the one keyframe it
/// starts from, every delta after it, every declaration made after the instant
/// asked for — the earlier ones are already in the scenario block — and the end
/// block.
///
/// **Later keyframes never travel, and that rests on an invariant of the
/// format: the deltas alone reconstruct every later keyframe's state.** A
/// browser holding the keyframe it was sent and applying each delta stands
/// exactly where a keyframe written at that instant says it stands. If a
/// recording ever breaks that, a viewer drifts silently from the run and a
/// re-attach jumps — so `tests/serving.rs` walks a real recording and checks
/// the accumulated state against every keyframe in it. A failure there is a
/// defect in the writer, not a reason to start sending keyframes.
fn to_send(index: &Index, keyframe: &IndexRecord, from: Ms) -> Vec<IndexRecord> {
    index
        .records()
        .iter()
        .filter(|record| record.offset >= keyframe.offset)
        .filter(|record| match record.tag {
            Tag::Keyframe => record.offset == keyframe.offset,
            Tag::Declare => record.t > from,
            Tag::Delta | Tag::End => true,
            Tag::Scenario | Tag::Index => false,
        })
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sections_a_browser_names_become_a_table_in_ascending_order() {
        let wanted = Wanted::parse(&["heat".to_string(), "playback".to_string(), "heat".to_string()]).expect("known");
        assert_eq!(wanted.0, vec![PLAYBACK, HEAT], "sorted, and one entry per section");
        assert_eq!(Wanted::parse(&[]).expect("the default set").0, DEFAULT_SECTIONS.to_vec());
        let refused = Wanted::parse(&["everything".to_string()]).expect_err("there is no such section");
        assert!(refused.to_string().contains("playback"), "the refusal says what it would take: {refused}");
    }

    #[test]
    fn live_on_a_finished_run_is_the_instant_it_stopped() {
        let edge = Ms(610_000);
        assert_eq!(resolve(&AttachFrom::Live, edge), edge);
        assert_eq!(resolve(&AttachFrom::At(300_000), edge), Ms(300_000));
    }
}
