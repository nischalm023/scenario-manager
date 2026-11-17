//! Within `studio-streaming`, `attach` is one frame client's reading of one attempt on `/frames/{run}/{attempt}`, in
//! the attach protocol the viewer's JS already speaks (`studio/viewer/js/src/protocol.ts`): the client's first text
//! message is `attach {id, from, sections, token}`, the token the attempt's watch ticket, which the listener checks
//! before anything here runs (`listeners::frames`); the server answers `attached {from_ms, live_edge_ms}`, then the
//! frames as binary messages, `lagged {resume_from_ms}` to a client that stopped reading, `ended {outcome}` after the
//! last frame, its outcome one of the metric stream's (`RunOutcome`), `error {message}` when the attempt has no copy
//! here or its copy fails. The frames come from the
//! provider, which reads the first copy it finds and follows it as it grows (`provider`). What travels: the scenario
//! block, the declarations made at or before `from`, the one keyframe at or before `from` (`"live"` is the newest, no
//! `from` the first), then every delta, later declaration and the end block in order and never another keyframe,
//! because the viewer takes one keyframe per attachment and the deltas alone carry it forward. A copy a fetch is still
//! landing is picked from once it reaches `from`. Frames travel as the bytes the copy holds, except the scenario
//! block, which travels with the floor a browser cannot compile (`floor`); `sections` is read and not applied.

use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use bytes::Bytes;
use desunits::SimTime;
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use studio_protocol::RunOutcome;
use tokio::time::timeout;
use worldview::Tag;

use crate::cache::opened;
use crate::metric_streams::Hubs;
use crate::provider::{Fault, Provider, Reading, Stamped, end_block_outcome, ended_outcome};
use crate::streams::{StreamKey, Streams};

/// Frames read from a copy per call.
const BATCH: usize = 32;

/// Where an attachment starts: a simulated instant in milliseconds, or `"live"`; spelled by hand, as the old server
/// spelled it, because a number and one word are not an untagged enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachFrom {
    At(u64),
    Live,
}

impl<'de> Deserialize<'de> for AttachFrom {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::Number(number) => {
                number.as_u64().map(Self::At).ok_or_else(|| D::Error::custom(format!("`{number}` is not an instant")))
            }
            Value::String(word) if word == "live" => Ok(Self::Live),
            other => Err(D::Error::custom(format!("`{other}` is neither an instant in ms nor \"live\""))),
        }
    }
}

/// The client's first message: the attach, with the attempt's watch ticket as `token`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Request {
    Attach {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        run: Option<String>,
        #[serde(default)]
        from: Option<AttachFrom>,
        #[serde(default)]
        sections: Vec<String>,
        #[serde(default)]
        token: Option<String>,
    },
}

/// What the server says in text, shaped as the old server's replies.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Reply {
    Attached { id: Option<String>, run: String, attempt: u32, from_ms: u64, live_edge_ms: u64 },
    Ended { id: Option<String>, run: String, outcome: RunOutcome },
    Lagged { id: Option<String>, resume_from_ms: u64 },
    Error { id: Option<String>, message: String },
}

/// Which frames of a sequence an attachment gets, and the two instants it is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    pub from: SimTime,
    pub live_edge: SimTime,
    /// Indices into the sequence, in sending order.
    pub indices: Vec<usize>,
    pub has_keyframe: bool,
}

/// The selection over a sequence of `(tag, instant)` in file order, the scenario block first when there is one. The
/// live edge is the newest slice's instant, the end block's own not counted.
pub fn pick(stamps: &[(Tag, SimTime)], from: Option<AttachFrom>) -> Pick {
    let live_edge = stamps.iter().rev().find(|(tag, _)| *tag != Tag::End).map_or(SimTime::ZERO, |(_, at)| *at);
    let keyframes = || stamps.iter().enumerate().filter(|(_, (tag, _))| *tag == Tag::Keyframe);
    let from = match from {
        Some(AttachFrom::At(ms)) => SimTime::ms(ms).min(live_edge),
        Some(AttachFrom::Live) => keyframes().last().map_or(live_edge, |(_, (_, at))| *at),
        None => keyframes().next().map_or(SimTime::ZERO, |(_, (_, at))| *at),
    };
    let keyframe = keyframes().filter(|(_, (_, at))| *at <= from).last().or_else(|| keyframes().next()).map(|(i, _)| i);
    let mut indices = Vec::new();
    for (index, (tag, at)) in stamps.iter().enumerate() {
        let wanted = match (tag, keyframe) {
            (Tag::Scenario, _) => index == 0,
            (Tag::Index, _) => false,
            (Tag::Keyframe, Some(k)) => index == k,
            (Tag::Keyframe, None) => false,
            (Tag::Declare, Some(k)) if index < k => *at <= from,
            (Tag::Declare, _) => true,
            (Tag::Delta | Tag::End, Some(k)) => index > k,
            (Tag::Delta, None) => false,
            // A run stopped before its first slice: no keyframe, and its end block all the same.
            (Tag::End, None) => true,
        };
        if wanted {
            indices.push(index);
        }
    }
    Pick { from, live_edge, indices, has_keyframe: keyframe.is_some() }
}

/// What of the frames after the pick an attached client still wants: the first keyframe only if it has none yet.
#[derive(Debug, Clone, Copy)]
pub struct Follow {
    pub has_scenario: bool,
    pub has_keyframe: bool,
}

impl Follow {
    pub fn wants(&mut self, tag: Tag) -> bool {
        match tag {
            Tag::Keyframe if !self.has_keyframe => {
                self.has_keyframe = true;
                true
            }
            Tag::Scenario if !self.has_scenario => {
                self.has_scenario = true;
                true
            }
            Tag::Delta | Tag::Declare | Tag::End => true,
            Tag::Keyframe | Tag::Scenario | Tag::Index => false,
        }
    }
}

/// What a viewer socket's route needs besides the socket.
#[derive(Clone)]
pub struct Attach {
    pub streams: Streams,
    pub pool: PgPool,
    pub provider: Provider,
    pub hubs: Hubs,
    /// How long one frame may wait for the client's socket before it is told it lagged (wall clock).
    pub lag: Duration,
}

type Sink = SplitSink<WebSocket, Message>;

/// Why a delivery stopped.
enum Stopped {
    Gone,
    Lagged,
}

/// One attachment as it goes out: the socket, the instant the client has reached, which a `lagged` names, and how the
/// end block says the run ended once it went.
struct Sending<'a> {
    sink: Sink,
    id: Option<String>,
    key: &'a StreamKey,
    attach: &'a Attach,
    reached: SimTime,
    end: Option<RunOutcome>,
}

/// The attempt's picture to an admitted client, from `from`, by the module doc's rule.
pub async fn serve_attached(
    sink: Sink,
    mut source: SplitStream<WebSocket>,
    key: StreamKey,
    id: Option<String>,
    from: Option<AttachFrom>,
    attach: Attach,
) {
    let known = opened(&attach.pool, &key).await.map_err(|fault| fault.to_string());
    let mut sending = Sending { sink, id, key: &key, attach: &attach, reached: SimTime::ZERO, end: None };
    let mut reading = match attach.provider.open(&key, known).await {
        Ok(reading) => reading,
        Err(unreached) => return sending.error(unreached.to_string()).await,
    };
    let history = match reaching(&mut reading, from).await {
        Ok(history) => history,
        Err(fault) => return sending.fault(fault).await,
    };
    let stamps: Vec<(Tag, SimTime)> = history.iter().map(|frame| (frame.tag, frame.at)).collect();
    let picked = pick(&stamps, from);
    sending.reached = picked.from;
    if let Err(stopped) = sending.say(&attached(&sending.id, &key, &picked)).await {
        return sending.finish(stopped).await;
    }
    let wanted: Vec<Stamped> = picked.indices.iter().map(|index| history[*index].clone()).collect();
    for batch in wanted.chunks(BATCH) {
        if let Err(fault) = sending.frames(&reading, batch).await {
            return sending.stop(fault).await;
        }
    }
    let mut follow = Follow { has_scenario: !history.is_empty(), has_keyframe: picked.has_keyframe };
    loop {
        let next = tokio::select! {
            next = reading.next() => next,
            from_client = source.next() => match from_client {
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                _ => return,
            },
        };
        match next {
            Ok(Some(frame)) if follow.wants(frame.tag) => {
                if let Err(fault) = sending.frames(&reading, &[frame]).await {
                    return sending.stop(fault).await;
                }
            }
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(fault) => return sending.fault(fault).await,
        }
    }
    let outcome = ended_outcome(&reading, sending.end.take());
    let ended = Reply::Ended { id: sending.id.clone(), run: key.run.as_str().to_owned(), outcome };
    let _ = sending.say(&ended).await;
    let _ = sending.sink.send(Message::Close(None)).await;
}

/// The reading's history, and for a copy a fetch is still landing, the frames that land until it reaches `from`, or
/// for `"live"` until it is whole, so the pick finds the keyframe it asks for as a whole copy would.
async fn reaching(reading: &mut Reading, from: Option<AttachFrom>) -> Result<Vec<Stamped>, Fault> {
    let mut history = reading.history().to_vec();
    let reached = |history: &[Stamped]| match from {
        Some(AttachFrom::At(ms)) => history.last().is_some_and(|frame| frame.at.as_ms() > ms),
        Some(AttachFrom::Live) => false,
        None => true,
    };
    while reading.landing() && !reached(&history) {
        match reading.next().await? {
            Some(frame) => history.push(frame),
            None => break,
        }
    }
    Ok(history)
}

/// Why sending stopped: the client, or the copy.
enum Stop {
    Client(Stopped),
    Copy(Fault),
}

impl Sending<'_> {
    /// `frames`, read from the copy and sent in order, the scenario block with its floor; how the end block says the
    /// run ended kept once it has gone, or, when no outcome names its stop reason, the copy failed.
    async fn frames(&mut self, reading: &Reading, frames: &[Stamped]) -> Result<(), Stop> {
        let bytes = reading.bytes(frames).await.map_err(Stop::Copy)?;
        for (frame, bytes) in frames.iter().zip(bytes) {
            let ending = (frame.tag == Tag::End).then(|| end_block_outcome(&bytes));
            let bytes = match frame.tag {
                Tag::Scenario => match self.attach.streams.frames().scenes().for_browser(self.key, &bytes).await {
                    Ok(bytes) => bytes,
                    Err(why) => {
                        return Err(Stop::Copy(Fault::Unreadable(format!(
                            "the run's floor could not be compiled: {why}"
                        ))));
                    }
                },
                _ => bytes,
            };
            self.deliver(bytes).await.map_err(Stop::Client)?;
            self.reached = frame.at;
            if let Some(ending) = ending {
                self.end = Some(ending.map_err(|why| Stop::Copy(Fault::Unreadable(why)))?);
            }
        }
        Ok(())
    }

    async fn stop(self, stop: Stop) {
        match stop {
            Stop::Client(stopped) => self.finish(stopped).await,
            Stop::Copy(fault) => self.fault(fault).await,
        }
    }

    /// The copy failed: the client is told why.
    async fn fault(self, fault: Fault) {
        eprintln!("studio-streaming: {}/{}: the picture stops: {fault}", self.key.run, self.key.attempt.0);
        self.error(fault.to_string()).await;
    }

    async fn error(mut self, message: String) {
        let _ = self.say(&Reply::Error { id: self.id.clone(), message }).await;
        let _ = self.sink.send(Message::Close(None)).await;
    }

    /// The client stopped taking frames, or went away: `lagged` names the last instant it took, best effort.
    async fn finish(mut self, stopped: Stopped) {
        if let Stopped::Lagged = stopped {
            let lagged = Reply::Lagged { id: self.id.clone(), resume_from_ms: self.reached.as_ms() };
            let _ = self.say(&lagged).await;
        }
        let _ = self.sink.send(Message::Close(None)).await;
    }

    async fn say(&mut self, reply: &Reply) -> Result<(), Stopped> {
        let text = serde_json::to_string(reply).expect("a reply is JSON");
        self.send(Message::Text(text.into())).await
    }

    async fn deliver(&mut self, bytes: Bytes) -> Result<(), Stopped> {
        self.send(Message::Binary(bytes)).await
    }

    async fn send(&mut self, message: Message) -> Result<(), Stopped> {
        match timeout(self.attach.lag, self.sink.send(message)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(Stopped::Gone),
            Err(_) => Err(Stopped::Lagged),
        }
    }
}

fn attached(id: &Option<String>, key: &StreamKey, picked: &Pick) -> Reply {
    Reply::Attached {
        id: id.clone(),
        run: key.run.as_str().to_owned(),
        attempt: key.attempt.0,
        from_ms: picked.from.as_ms(),
        live_edge_ms: picked.live_edge.as_ms(),
    }
}

#[cfg(test)]
mod tests {
    use desunits::SimTime;
    use worldview::Tag;

    use super::{AttachFrom, Follow, pick};

    fn stamps() -> Vec<(Tag, SimTime)> {
        let mut stamps = vec![(Tag::Scenario, SimTime::ZERO)];
        for segment in 0..3u64 {
            let start = segment * 30_000;
            stamps.push((Tag::Keyframe, SimTime::ms(start)));
            if segment == 1 {
                stamps.push((Tag::Declare, SimTime::ms(start)));
            }
            for slice in 0..3u64 {
                stamps.push((Tag::Delta, SimTime::ms(start + slice * 10_000)));
            }
        }
        stamps
    }

    fn tags(stamps: &[(Tag, SimTime)], indices: &[usize]) -> Vec<Tag> {
        indices.iter().map(|index| stamps[*index].0).collect()
    }

    #[test]
    fn no_from_starts_at_the_first_keyframe_and_sends_no_later_one() {
        let stamps = stamps();
        let picked = pick(&stamps, None);
        assert_eq!((picked.from, picked.live_edge), (SimTime::ZERO, SimTime::ms(80_000)));
        let sent = tags(&stamps, &picked.indices);
        assert_eq!(sent.iter().filter(|tag| **tag == Tag::Keyframe).count(), 1);
        assert_eq!(sent[0], Tag::Scenario);
        assert_eq!(sent.len(), 1 + 1 + 9 + 1, "scenario, one keyframe, nine deltas, the declaration");
    }

    #[test]
    fn an_instant_mid_run_starts_at_the_keyframe_before_it_with_the_earlier_declarations_ahead() {
        let stamps = stamps();
        let picked = pick(&stamps, Some(AttachFrom::At(65_000)));
        assert_eq!(picked.from, SimTime::ms(65_000));
        let sent = tags(&stamps, &picked.indices);
        assert_eq!(sent, [Tag::Scenario, Tag::Declare, Tag::Keyframe, Tag::Delta, Tag::Delta, Tag::Delta]);
        assert_eq!(stamps[picked.indices[2]].1, SimTime::ms(60_000), "the keyframe at or before the instant");
        let past = pick(&stamps, Some(AttachFrom::At(999_999)));
        assert_eq!(past.from, SimTime::ms(80_000), "an instant past the edge lands on the edge");
    }

    /// A whole copy ends with its end block: it travels last, and the live edge stays the newest slice's instant
    /// however far past it dessim stamped the end block.
    #[test]
    fn the_end_block_goes_last_and_moves_no_edge() {
        let mut stamps = stamps();
        stamps.push((Tag::End, SimTime::ms(85_000)));
        let picked = pick(&stamps, Some(AttachFrom::At(999_999)));
        assert_eq!((picked.from, picked.live_edge), (SimTime::ms(80_000), SimTime::ms(80_000)));
        assert_eq!(tags(&stamps, &picked.indices).last(), Some(&Tag::End));
        let mut follow = Follow { has_scenario: true, has_keyframe: true };
        assert!(follow.wants(Tag::End), "a live client is sent the end block as a replay is");
    }

    /// A run stopped before its first slice has no keyframe: its copy sends the scenario block and the end block,
    /// however the client attaches.
    #[test]
    fn a_run_stopped_before_its_first_slice_sends_its_end_block_without_a_keyframe() {
        let stamps = [(Tag::Scenario, SimTime::ZERO), (Tag::End, SimTime::ms(4_000))];
        for from in [None, Some(AttachFrom::Live), Some(AttachFrom::At(0))] {
            let picked = pick(&stamps, from);
            assert!(!picked.has_keyframe);
            assert_eq!(tags(&stamps, &picked.indices), [Tag::Scenario, Tag::End], "{from:?}");
        }
    }

    #[test]
    fn live_is_the_newest_keyframe_and_a_client_without_one_takes_the_next() {
        let stamps = stamps();
        let picked = pick(&stamps, Some(AttachFrom::Live));
        assert_eq!(picked.from, SimTime::ms(60_000));
        assert_eq!(
            tags(&stamps, &picked.indices),
            [Tag::Scenario, Tag::Declare, Tag::Keyframe, Tag::Delta, Tag::Delta, Tag::Delta]
        );
        let empty = pick(&[(Tag::Scenario, SimTime::ZERO)], Some(AttachFrom::Live));
        assert!(!empty.has_keyframe && empty.indices == [0]);
        let mut follow = Follow { has_scenario: true, has_keyframe: false };
        assert!(follow.wants(Tag::Keyframe) && follow.wants(Tag::Delta) && !follow.wants(Tag::Keyframe));
        assert!(!follow.wants(Tag::Scenario), "a client that has the scenario block is not sent it again");
        let mut early = Follow { has_scenario: false, has_keyframe: false };
        assert!(early.wants(Tag::Scenario) && !early.wants(Tag::Scenario), "one attached before it came gets it once");
    }
}
