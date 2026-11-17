//! Where a run's output objects live: the key each object of a run is written
//! under, and the reading of such a key back into the run, attempt, and part it
//! names. Pure string work — nothing here touches storage.
//!
//! Relative to the crate: `runstore` moves a run's bytes to and from object
//! storage, [`key`](crate::key) says what a key may be spelled with, and this
//! module decides which of those keys each object of a run gets, so that the
//! Crucible server writing a run and anything later reading it — a viewer, the
//! workbook command — address the same objects without agreeing on anything
//! but a run id.
//!
//! What belongs to the run and what belongs to an attempt are kept apart. The
//! spec is the run's: every try at a run is run from the same spec, and the
//! registry keeps one pointer to it per run. The recording is an attempt's:
//! each attempt writes under its own prefix, so a second try at a run cannot
//! touch what the first one wrote.
//!
//! ```text
//! runs/<run id>/spec.json
//! runs/<run id>/attempts/<n>/recording.worldview
//! runs/<run id>/attempts/<n>/live/scenario.worldview
//! runs/<run id>/attempts/<n>/live/seg-000000.worldview, seg-000001.worldview, ...
//! ```
//!
//! An attempt writes ONE recording: a `.worldview` holds its own index block,
//! so a reader reaches any simulated instant by reading the file's trailer and
//! asking for the byte ranges the index names. Beside it, under `live/`, the
//! streaming server keeps the picture as it was locked while the run went
//! (live frames, 26 October 2026): the scenario block once, then one object
//! per locked segment, numbered from 0 in the order they lock and zero-padded
//! so a listing comes back in segment order. These are the only keys here that
//! carry a sequence number; a replay reads them in order until the recording
//! is stored, and the recording alone after that.

use std::fmt;

use thiserror::Error;

use crate::key::{KeyError, validate_segment};

/// Root segment under which every run's output lives.
const RUNS: &str = "runs";
/// Segment separating a run from its attempts.
const ATTEMPTS: &str = "attempts";
/// The object holding the spec the run was submitted with.
const SPEC: &str = "spec.json";
/// The attempt's recording: its frames, its index block and its ending.
const RECORDING: &str = "recording.worldview";
/// The attempt's live stream, as the streaming server locked and uploaded it.
const LIVE: &str = "live";
/// The live stream's scenario block, uploaded once before any segment.
const LIVE_SCENARIO: &str = "scenario.worldview";
/// A live segment's spelling: `seg-<k>.worldview`, `k` padded to `SEGMENT_DIGITS`.
const SEGMENT_PREFIX: &str = "seg-";
const SEGMENT_SUFFIX: &str = ".worldview";
/// Digits a segment number is padded to: lexical order is segment order up to
/// 999 999 segments, nine years of simtime at five minutes a segment.
pub const SEGMENT_DIGITS: usize = 6;

/// The name a run is known by, everywhere: in the registry, in this layout, and
/// in the broker's messages.
///
/// It is checked on the way in because it becomes one segment of every key the
/// run's objects live under. An id carrying a `/` would write under another
/// run's prefix and mint keys this module's own parser refuses; an id of `..`
/// would read as a filesystem's parent directory.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunId(String);

impl RunId {
    /// Takes an id the registry minted, refusing one that cannot be a key
    /// segment.
    pub fn new(id: impl Into<String>) -> Result<Self, LayoutError> {
        let id = id.into();
        validate_segment(&id).map_err(|source| LayoutError::BadRunId { run_id: id.clone(), source })?;
        Ok(Self(id))
    }

    /// The id itself, for a caller handing it to the registry or a message.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which attempt of a run: the registry records one row per attempt, numbered
/// from one, and each attempt writes under its own prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttemptNumber(u32);

impl AttemptNumber {
    /// The attempt a run starts with.
    pub const FIRST: Self = Self(1);

    /// Takes a number the registry minted, refusing zero: attempts count from
    /// one, so a zero here is a caller that lost count rather than an attempt.
    pub fn new(number: u32) -> Result<Self, LayoutError> {
        if number == 0 {
            return Err(LayoutError::AttemptZero);
        }
        Ok(Self(number))
    }

    /// The number itself, for a caller that has to hand it to the registry.
    pub fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for AttemptNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Everything every run wrote: the prefix a store is probed under, and the one
/// a restart lists to find recordings whose ending was never written down.
pub const RUNS_PREFIX: &str = "runs/";

/// Everything belonging to one run: its spec and all of its attempts.
pub fn run_prefix(run_id: &RunId) -> String {
    format!("{RUNS}/{run_id}/")
}

/// The spec the run was submitted with, stored whole because specs embed entire
/// warehouse designs and the registry keeps only a pointer and a hash. It
/// belongs to the run: every attempt is run from this one object.
pub fn spec_key(run_id: &RunId) -> String {
    format!("{}{SPEC}", run_prefix(run_id))
}

/// Everything one attempt of a run wrote.
pub fn attempt_prefix(run_id: &RunId, attempt: AttemptNumber) -> String {
    format!("{}{ATTEMPTS}/{attempt}/", run_prefix(run_id))
}

/// An attempt's recording, whole: the file a replaying reader issues ranged
/// gets against.
pub fn recording_key(run_id: &RunId, attempt: AttemptNumber) -> String {
    format!("{}{RECORDING}", attempt_prefix(run_id, attempt))
}

/// Everything the streaming server wrote of one attempt's live stream.
pub fn live_prefix(run_id: &RunId, attempt: AttemptNumber) -> String {
    format!("{}{LIVE}/", attempt_prefix(run_id, attempt))
}

/// The live stream's scenario block: the frame a reader of any segment needs
/// first.
pub fn live_scenario_key(run_id: &RunId, attempt: AttemptNumber) -> String {
    format!("{}{LIVE_SCENARIO}", live_prefix(run_id, attempt))
}

/// Locked segment `segment` of the live stream, one object: its keyframe and
/// the deltas up to the next keyframe. Numbered from 0 and zero-padded, so a
/// listing of the prefix comes back in segment order.
pub fn live_segment_key(run_id: &RunId, attempt: AttemptNumber, segment: u32) -> String {
    format!("{}{SEGMENT_PREFIX}{segment:0width$}{SEGMENT_SUFFIX}", live_prefix(run_id, attempt), width = SEGMENT_DIGITS)
}

/// Which of a run's objects a key names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPart {
    /// The spec the run was submitted with.
    Spec,
    /// The recording one attempt of the run wrote.
    Recording {
        /// Which attempt wrote it.
        attempt: AttemptNumber,
    },
    /// The scenario block of one attempt's live stream.
    LiveScenario {
        /// Which attempt's stream.
        attempt: AttemptNumber,
    },
    /// One locked segment of one attempt's live stream.
    LiveSegment {
        /// Which attempt's stream.
        attempt: AttemptNumber,
        /// Which segment, numbered from 0.
        segment: u32,
    },
}

/// A key read back into the run and the part it names — what a reader needs
/// when it finds an object by listing rather than by building its key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunObjectKey {
    /// The run the object belongs to.
    pub run_id: RunId,
    /// Which of the run's objects it is.
    pub part: RunPart,
}

impl RunObjectKey {
    /// Reads a key written by this module's builders. Every key this accepts
    /// renders back to itself through [`fmt::Display`], and every key it rejects
    /// is one no builder here produces.
    pub fn parse(key: &str) -> Result<Self, LayoutError> {
        let mut segments = key.split('/');
        let malformed = || LayoutError::NotARunObject { key: key.to_string() };

        if segments.next() != Some(RUNS) {
            return Err(malformed());
        }
        let run_id = RunId::new(segments.next().ok_or_else(malformed)?)?;

        let part = match (segments.next().ok_or_else(malformed)?, segments.next()) {
            (SPEC, None) => RunPart::Spec,
            (ATTEMPTS, Some(number)) => {
                let attempt = parse_attempt(key, number)?;
                match (segments.next().ok_or_else(malformed)?, segments.next(), segments.next()) {
                    (RECORDING, None, None) => RunPart::Recording { attempt },
                    (LIVE, Some(LIVE_SCENARIO), None) => RunPart::LiveScenario { attempt },
                    (LIVE, Some(object), None) => {
                        RunPart::LiveSegment { attempt, segment: parse_segment(key, object)? }
                    }
                    _ => return Err(malformed()),
                }
            }
            _ => return Err(malformed()),
        };

        Ok(Self { run_id, part })
    }
}

impl fmt::Display for RunObjectKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key = match self.part {
            RunPart::Spec => spec_key(&self.run_id),
            RunPart::Recording { attempt } => recording_key(&self.run_id, attempt),
            RunPart::LiveScenario { attempt } => live_scenario_key(&self.run_id, attempt),
            RunPart::LiveSegment { attempt, segment } => live_segment_key(&self.run_id, attempt, segment),
        };
        f.write_str(&key)
    }
}

/// Reads the attempt segment, insisting on the spelling the builders produce so
/// that a key and the [`RunObjectKey`] it parses to name each other one to one.
fn parse_attempt(key: &str, segment: &str) -> Result<AttemptNumber, LayoutError> {
    let bad = || LayoutError::BadAttempt { key: key.to_string(), segment: segment.to_string() };
    let number: u32 = segment.parse().map_err(|_| bad())?;
    if segment != number.to_string() {
        return Err(bad());
    }
    AttemptNumber::new(number)
}

/// Reads a live segment's object name, insisting on the padded spelling the
/// builder produces, for the same one-to-one reason as `parse_attempt`.
fn parse_segment(key: &str, object: &str) -> Result<u32, LayoutError> {
    let bad = || LayoutError::BadSegment { key: key.to_string(), object: object.to_string() };
    let digits =
        object.strip_prefix(SEGMENT_PREFIX).and_then(|rest| rest.strip_suffix(SEGMENT_SUFFIX)).ok_or_else(bad)?;
    if digits.len() != SEGMENT_DIGITS || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(bad());
    }
    digits.parse().map_err(|_| bad())
}

/// What a key, a run id, or a number can fail to be.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LayoutError {
    /// The key does not name an object of a run's output at all.
    #[error("`{key}` does not name an object of a run's output")]
    NotARunObject {
        /// The key as given.
        key: String,
    },
    /// The run id cannot be one segment of a key.
    #[error("`{run_id}` is not a run id: {source}")]
    BadRunId {
        /// The id as given.
        run_id: String,
        /// Which part of the spelling rule it broke.
        #[source]
        source: KeyError,
    },
    /// The key's attempt segment is not an attempt number as written here.
    #[error("`{segment}` in `{key}` is not an attempt number")]
    BadAttempt {
        /// The key as given.
        key: String,
        /// The segment that should have been the attempt number.
        segment: String,
    },
    /// Attempt numbering starts at one.
    #[error("attempts count from 1, so there is no attempt 0")]
    AttemptZero,
    /// The key's object under `live/` is neither the scenario block nor a
    /// segment as written here.
    #[error("`{object}` in `{key}` is not a live stream's object")]
    BadSegment {
        /// The key as given.
        key: String,
        /// The object name under `live/`.
        object: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{AttemptNumber, LayoutError, RUNS_PREFIX, RunId, RunObjectKey, RunPart};
    use super::{
        attempt_prefix, live_prefix, live_scenario_key, live_segment_key, recording_key, run_prefix, spec_key,
    };
    use crate::key::{KeyError, validate_key, validate_prefix};

    fn run() -> RunId {
        RunId::new("01J8Z5K3QW").expect("a ULID is one segment")
    }

    #[test]
    fn every_run_lives_under_the_prefix_a_store_is_probed_under() {
        assert!(run_prefix(&run()).starts_with(RUNS_PREFIX));
        validate_prefix(RUNS_PREFIX).expect("the probe's prefix is one a store accepts");
    }

    #[test]
    fn keys_read_as_written() {
        let attempt = AttemptNumber::new(2).expect("attempts count from 1");

        for (key, part) in [
            (spec_key(&run()), RunPart::Spec),
            (recording_key(&run(), attempt), RunPart::Recording { attempt }),
            (live_scenario_key(&run(), attempt), RunPart::LiveScenario { attempt }),
            (live_segment_key(&run(), attempt, 12), RunPart::LiveSegment { attempt, segment: 12 }),
        ] {
            let parsed = RunObjectKey::parse(&key).expect("a built key parses");
            assert_eq!(parsed, RunObjectKey { run_id: run(), part });
            assert_eq!(parsed.to_string(), key, "a parsed key renders back to itself");
        }
    }

    #[test]
    fn the_spec_belongs_to_the_run_and_the_recording_to_an_attempt() {
        let attempt = AttemptNumber::FIRST;
        assert_eq!(spec_key(&run()), "runs/01J8Z5K3QW/spec.json");
        assert_eq!(attempt_prefix(&run(), attempt), "runs/01J8Z5K3QW/attempts/1/");
        assert_eq!(recording_key(&run(), attempt), "runs/01J8Z5K3QW/attempts/1/recording.worldview");

        // A second attempt reads the same spec and writes somewhere else.
        let retry = AttemptNumber::new(2).expect("attempts count from 1");
        assert!(!spec_key(&run()).contains("attempts"), "the spec key names no attempt");
        assert_ne!(attempt_prefix(&run(), retry), attempt_prefix(&run(), attempt));
        assert!(attempt_prefix(&run(), retry).starts_with(&run_prefix(&run())));
        assert!(recording_key(&run(), attempt).starts_with(&attempt_prefix(&run(), attempt)));
        assert!(spec_key(&run()).starts_with(&run_prefix(&run())));
    }

    #[test]
    fn the_live_stream_lives_under_the_attempt_and_lists_in_segment_order() {
        let attempt = AttemptNumber::FIRST;
        assert_eq!(live_prefix(&run(), attempt), "runs/01J8Z5K3QW/attempts/1/live/");
        assert_eq!(live_scenario_key(&run(), attempt), "runs/01J8Z5K3QW/attempts/1/live/scenario.worldview");
        assert_eq!(live_segment_key(&run(), attempt, 0), "runs/01J8Z5K3QW/attempts/1/live/seg-000000.worldview");
        assert_eq!(live_segment_key(&run(), attempt, 1_234), "runs/01J8Z5K3QW/attempts/1/live/seg-001234.worldview");
        assert!(live_prefix(&run(), attempt).starts_with(&attempt_prefix(&run(), attempt)));
        assert_eq!(validate_prefix(&live_prefix(&run(), attempt)), Ok(()));
        for key in [live_scenario_key(&run(), attempt), live_segment_key(&run(), attempt, 7)] {
            assert_eq!(validate_key(&key), Ok(()), "`{key}` should be a usable key");
        }
        // A store lists keys in lexical order; the padding makes that segment order.
        let keys: Vec<String> =
            [9, 10, 100, 999_999].into_iter().map(|k| live_segment_key(&run(), attempt, k)).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }

    #[test]
    fn every_builder_produces_a_usable_key() {
        let attempt = AttemptNumber::new(37).expect("attempts count from 1");
        let run = RunId::new("a-run_id.42").expect("the alphabet allows these");

        for key in [spec_key(&run), recording_key(&run, attempt)] {
            assert_eq!(validate_key(&key), Ok(()), "`{key}` should be a usable key");
        }
        for prefix in [run_prefix(&run), attempt_prefix(&run, attempt)] {
            assert_eq!(validate_prefix(&prefix), Ok(()), "`{prefix}` should be a usable prefix");
            assert!(prefix.ends_with('/'), "`{prefix}` should name a whole level");
        }
    }

    #[test]
    fn a_run_id_has_to_be_one_key_segment() {
        // Each of these would otherwise write under a prefix the caller did not
        // name, or mint a key this module's own parser refuses.
        assert_eq!(
            RunId::new("tenant/r1"),
            Err(LayoutError::BadRunId {
                run_id: "tenant/r1".to_string(),
                source: KeyError::Character { character: '/' }
            })
        );
        assert_eq!(
            RunId::new(".."),
            Err(LayoutError::BadRunId {
                run_id: "..".to_string(),
                source: KeyError::DotSegment { segment: "..".to_string() },
            })
        );
        assert!(RunId::new("").is_err());
        assert!(RunId::new("r1#1").is_err());
        assert!(RunId::new("01J8Z5K3QW").is_ok());
    }

    #[test]
    fn there_is_no_attempt_zero() {
        assert_eq!(AttemptNumber::new(0), Err(LayoutError::AttemptZero));
        assert_eq!(AttemptNumber::new(1), Ok(AttemptNumber::FIRST));
        assert!(matches!(RunObjectKey::parse("runs/r1/attempts/0/recording.worldview"), Err(LayoutError::AttemptZero)));
    }

    #[test]
    fn keys_no_builder_produces_are_refused() {
        for key in [
            "",
            "runs",
            "runs//attempts/1/recording.worldview",
            "runs/r1/attempts/1",
            "runs/r1/attempts/1/",
            "runs/r1/spec.json/extra",
            "runs/r1/attempts/1/spec.json",
            "runs/r1/attempts/1/recording",
            "runs/r1/attempts/1/recording.worldview/extra",
            "runs/r1/attempts/1/recording.json",
            "runs/r1/attempts/1/live",
            "runs/r1/attempts/1/live/",
            "runs/r1/attempts/1/live/other.worldview",
            "runs/r1/attempts/1/live/scenario.worldview/extra",
            "runs/r1/attempts/1/live/seg-000001.worldview/extra",
            "runs/r1/attempts/1/live/seg-000001.json",
            "runs/r1/attempts/1/live/seg-00000a.worldview",
            "runs/tenant/r1/attempts/1/recording.worldview",
            "attempts/1/recording.worldview",
        ] {
            assert!(RunObjectKey::parse(key).is_err(), "`{key}` should not parse");
        }
    }

    #[test]
    fn a_key_spelled_another_way_is_refused() {
        // It would read as a number, but it is not what a builder writes, and
        // accepting it would give one object two spellings.
        assert!(matches!(
            RunObjectKey::parse("runs/r1/attempts/01/recording.worldview"),
            Err(LayoutError::BadAttempt { .. })
        ));
        for unpadded in ["seg-1.worldview", "seg-0000001.worldview"] {
            let key = format!("runs/r1/attempts/1/live/{unpadded}");
            assert!(matches!(RunObjectKey::parse(&key), Err(LayoutError::BadSegment { .. })), "{key}");
        }
    }
}
