//! Within `studio-streaming`, `streams` is every stream this server holds, in memory, one per attempt, opened by its
//! runner's hello (ruling D174 keeps no stream on disk: a server that stops loses its streams). A stream that closed,
//! ended by its runner or closed by this server, stays replayable until more than `keep` have closed, the oldest
//! forgotten first (`stream`). A replay is at most three messages (`replay`), so the replays hold at most three
//! messages for each open stream and for each of at most `keep` closed ones. A replayed `progress` or `ended` is a
//! runner's message encoded anew, which the runners' listener takes only within its message limit
//! (`listeners::runners`), and can be longer than it: an optional field the runner left out comes back as `null`, and
//! a number in serde_json's own spelling, `1e15` as `1000000000000000.0`. Each change a stream takes is offered to
//! the stats writer, which keeps it until it is written, whatever is forgotten here. The attempt's picture lives
//! beside its messages in the frame store (`frames`), which `Streams` carries so that the ending closes the
//! picture's last segment too, each segment locked is said to the viewers as an `edge`, and a forgotten stream takes
//! its frames with it.

mod replay;
mod stream;

use std::collections::VecDeque;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustc_hash::FxHashMap;
use studio_protocol::{AttemptNumber, RunnerMessage, StreamRefusal, Ulid};
use tokio::time::sleep;

use crate::cache::mark_cached;
use crate::frames::{AttemptFrames, ClosedSegment, FrameStore, Pushed};
use crate::stats::Stats;

pub use stream::{Attachment, Stream};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StreamKey {
    pub run: Ulid,
    pub attempt: AttemptNumber,
}

/// Every stream, shared by the two listeners and the heartbeat.
#[derive(Clone)]
pub struct Streams {
    held: Arc<Mutex<Held>>,
    /// The one counter every stream draws its snapshots' revisions from.
    revisions: Arc<AtomicU64>,
    keep: usize,
    /// How long a runner whose socket closed without its ending has to come back before it is lost (wall clock).
    grace: Duration,
    stats: Stats,
    frames: FrameStore,
}

#[derive(Default)]
struct Held {
    streams: FxHashMap<StreamKey, Arc<Stream>>,
    // ordered: the closed streams in the order they closed, the oldest forgotten first.
    closed: VecDeque<StreamKey>,
}

impl Streams {
    pub fn new(keep: usize, grace: Duration, stats: Stats, frames: FrameStore) -> Self {
        Self { held: Arc::default(), revisions: Arc::default(), keep, grace, stats, frames }
    }

    pub fn frames(&self) -> &FrameStore {
        &self.frames
    }

    /// A runner's hello for `key`: its stream, opened when none is held, and the number of the socket that owns it
    /// from here on, superseding any before; a closed stream opens again. An ended stream is refused.
    pub fn take_over(&self, key: StreamKey) -> Result<(Arc<Stream>, u64), StreamRefusal> {
        let mut held = self.held.lock().expect("the streams");
        let (stream, new) = match held.streams.get(&key) {
            Some(stream) => (stream.clone(), false),
            None => {
                let stream = Arc::new(Stream::new(key.clone(), self.revisions.clone()));
                held.streams.insert(key.clone(), stream.clone());
                (stream, true)
            }
        };
        // Under the lock that forgets streams, so a closed stream cannot be forgotten as its runner takes it back.
        let taken = stream.take_over()?;
        if taken.reopened {
            held.closed.retain(|closed| *closed != key);
        }
        drop(held);
        if new || taken.reopened {
            self.stats.offer(stream.snapshot());
        }
        Ok((stream, taken.socket))
    }

    pub fn get(&self, key: &StreamKey) -> Option<Arc<Stream>> {
        self.held.lock().expect("the streams").streams.get(key).cloned()
    }

    /// The streams neither closed by this server nor ended by their runner.
    pub fn open_count(&self) -> usize {
        self.held.lock().expect("the streams").streams.values().filter(|stream| stream.is_open()).count()
    }

    pub fn is_open(&self, key: &StreamKey) -> bool {
        self.get(key).is_some_and(|stream| stream.is_open())
    }

    /// A message from the socket `socket` of `stream`'s runner, kept and broadcast; `true` when it was the ending,
    /// which closes the picture's last segment first and says it as an `edge` ahead of the ending.
    pub fn take(&self, stream: &Stream, socket: u64, message: RunnerMessage) -> Result<bool, StreamRefusal> {
        if matches!(message, RunnerMessage::Ended(_)) && stream.owned_by(socket) && stream.is_open() {
            if let Some(closed) = self.frames.close(&stream.key) {
                stream.edge(closed.locked_sim_ms);
            }
        }
        let took = stream.take(socket, message)?;
        if took.ended {
            self.close(&mut self.held.lock().expect("the streams"), &stream.key);
        }
        if let Some(snapshot) = took.snapshot {
            self.stats.offer(snapshot);
        }
        if took.ended {
            self.stats.hurry();
        }
        Ok(took.ended)
    }

    /// The socket `socket` of `stream`'s runner closed without the ending: once the grace has passed, unless another
    /// socket took the stream over, its runner is lost and the stream closes, to open again if the runner comes back.
    pub fn runner_left(&self, stream: Arc<Stream>, socket: u64) {
        let streams = self.clone();
        tokio::spawn(async move {
            sleep(streams.grace).await;
            let mut held = streams.held.lock().expect("the streams");
            let Some(snapshot) = stream.lose(socket) else { return };
            let StreamKey { run, attempt } = &stream.key;
            eprintln!(
                "studio-streaming: {run}/{}'s runner went away and did not come back within the grace",
                attempt.0
            );
            streams.close(&mut held, &stream.key);
            drop(held);
            streams.stats.offer(snapshot);
            streams.stats.hurry();
        });
    }

    /// A binary frame from the socket `socket` of `stream`'s runner, into the attempt's `frames`: the scenario block
    /// goes to the upload queue at once, a segment the keyframe closed goes there and is said as an `edge`. Refused
    /// from a socket a newer one superseded, and as the frames refuse it, which closes the runner's socket.
    pub async fn frame(
        &self,
        stream: &Stream,
        socket: u64,
        frames: &Arc<AttemptFrames>,
        bytes: Vec<u8>,
    ) -> Result<(), StreamRefusal> {
        if !stream.owned_by(socket) {
            return Err(StreamRefusal::Superseded);
        }
        let pushed = frames.push(bytes).await.map_err(|refusal| {
            let StreamKey { run, attempt } = &stream.key;
            eprintln!("studio-streaming: {run}/{}'s frames stop: {refusal}", attempt.0);
            refusal.refusal()
        })?;
        match pushed {
            Pushed::Scenario => {
                let uploader = self.frames.uploader();
                uploader.scenario(frames);
                if let Some(rows) = uploader.rows() {
                    mark_cached(rows, &stream.key).await;
                }
            }
            Pushed::Taken | Pushed::Repeated => {}
            Pushed::Closed(closed) => self.segment_closed(stream, frames, closed),
        }
        Ok(())
    }

    /// The runner's socket went quiet after a batch of frames: the segment being sent is whole.
    pub fn settle(&self, stream: &Stream, frames: &Arc<AttemptFrames>) {
        if let Some(closed) = frames.settle() {
            self.segment_closed(stream, frames, closed);
        }
    }

    /// A segment of the attempt's picture closed: queued for upload, and said to the viewers as an `edge`.
    fn segment_closed(&self, stream: &Stream, frames: &Arc<AttemptFrames>, closed: ClosedSegment) {
        self.frames.uploader().segment(frames, closed);
        stream.edge(closed.locked_sim_ms);
    }

    /// The server is stopping: every open stream closes, its figures offered to be written once more.
    pub fn stop(&self) {
        let held = self.held.lock().expect("the streams");
        for snapshot in held.streams.values().filter_map(|stream| stream.stop()) {
            self.stats.offer(snapshot);
        }
    }

    /// `key`'s stream closed: kept replayable among the closed, the oldest forgotten past `keep`.
    fn close(&self, held: &mut Held, key: &StreamKey) {
        if !held.closed.contains(key) {
            held.closed.push_back(key.clone());
        }
        while held.closed.len() > self.keep {
            if let Some(oldest) = held.closed.pop_front()
                && let Some(stream) = held.streams.remove(&oldest)
            {
                stream.forget();
                self.frames.forget(&oldest);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ptr;
    use std::time::Duration;

    use desunits::SimTime;
    use studio_protocol::{
        AttemptNumber, EndedBy, Ending, ProgressValue, RunState, RunnerMessage, STREAM_FORMAT, StreamEnded,
        StreamProgress, StreamRefusal,
    };
    use tokio::time::{sleep, timeout};

    use super::{StreamKey, Streams};
    use crate::frames::tests::frame_store;
    use crate::stats::Stats;

    fn key(n: u32) -> StreamKey {
        StreamKey { run: "01KZAVPQNES65HV4EZADPCS7E8".parse().unwrap(), attempt: AttemptNumber(n) }
    }

    fn streams(keep: usize) -> Streams {
        Streams::new(keep, Duration::from_secs(30), Stats::default(), frame_store())
    }

    const KEEP: usize = 10;

    fn started() -> RunnerMessage {
        RunnerMessage::Started { v: STREAM_FORMAT }
    }

    fn progress(sim_ms: u64) -> StreamProgress {
        let value = ProgressValue { metric: "picks".to_owned(), label: "Picks".to_owned(), value: None };
        StreamProgress { v: STREAM_FORMAT, sim_ms: SimTime::ms(sim_ms), values: vec![value] }
    }

    fn ended() -> StreamEnded {
        let ending = Ending {
            state: RunState::Finished,
            ended_by: EndedBy::Simulator,
            stop_reason: "SimTimeLimit".to_owned(),
            stop_detail: None,
            sim_ms: SimTime::ms(600_000),
        };
        StreamEnded { v: STREAM_FORMAT, ending }
    }

    #[test]
    fn nothing_is_taken_after_the_ending_and_an_ended_stream_takes_no_socket() {
        let streams = streams(KEEP);
        let (stream, socket) = streams.take_over(key(1)).expect("open");
        assert_eq!(streams.take(&stream, socket, started()), Ok(false));
        assert_eq!(streams.take(&stream, socket, RunnerMessage::Ended(ended())), Ok(true));
        assert_eq!(streams.take(&stream, socket, RunnerMessage::Ended(ended())), Err(StreamRefusal::AttemptEnded));
        assert_eq!(streams.take_over(key(1)).err(), Some(StreamRefusal::AttemptEnded));
        let attachment = streams.get(&key(1)).expect("still replayable").attach();
        assert_eq!(attachment.first.len(), 3, "attached, started and ended");
        assert!(attachment.live.is_none(), "an ended stream has nothing more to follow");
    }

    /// The takeover and the fence are one step: once a second socket holds the stream, the first is refused.
    #[test]
    fn a_newer_socket_supersedes_the_one_before_it() {
        let streams = streams(KEEP);
        let (stream, first) = streams.take_over(key(1)).expect("open");
        let mut owner = stream.owner();
        let (again, second) = streams.take_over(key(1)).expect("taken over");
        assert!(ptr::eq(&*stream, &*again) && second > first);
        assert_eq!(*owner.borrow_and_update(), second, "the first socket's watch says it was superseded");
        let moved = || RunnerMessage::Progress(progress(10_000));
        assert_eq!(streams.take(&stream, first, moved()), Err(StreamRefusal::Superseded));
        let ending = RunnerMessage::Ended(ended());
        assert_eq!(streams.take(&stream, first, ending), Err(StreamRefusal::Superseded), "nor can it end the stream");
        assert_eq!(streams.take(&stream, second, moved()), Ok(false));
    }

    /// A viewer attaching while messages come gets each exactly once: what the replay holds, and after it only what
    /// was taken after the attachment.
    #[test]
    fn a_viewer_attaching_mid_stream_gets_no_message_twice() {
        let streams = streams(KEEP);
        let (stream, socket) = streams.take_over(key(1)).expect("open");
        streams.take(&stream, socket, started()).unwrap();
        streams.take(&stream, socket, RunnerMessage::Progress(progress(10_000))).unwrap();
        let mut attachment = stream.attach();
        streams.take(&stream, socket, RunnerMessage::Progress(progress(20_000))).unwrap();
        let live = attachment.live.as_mut().expect("following");
        let followed = live.try_recv().expect("the progress after the attachment");
        assert!(followed.as_str().contains("\"sim_ms\":20000"), "{followed}");
        assert!(live.try_recv().is_err(), "nothing else live");
        assert_eq!(attachment.first.len(), 3, "attached, started and the progress before");
    }

    /// A stream forgotten while closed and opened anew by its returning runner writes figures the row still takes:
    /// every snapshot's revision is drawn from the process's one counter.
    #[tokio::test]
    async fn a_stream_opened_anew_after_it_was_forgotten_writes_newer_revisions_than_before() {
        let streams = Streams::new(1, Duration::from_millis(10), Stats::default(), frame_store());
        let (lost, socket) = streams.take_over(key(1)).expect("open");
        for n in 1..=5 {
            streams.take(&lost, socket, RunnerMessage::Progress(progress(n * 10_000))).unwrap();
        }
        streams.runner_left(lost.clone(), socket);
        sleep(Duration::from_millis(100)).await;
        let before = lost.snapshot().revision;
        let (other, socket) = streams.take_over(key(2)).expect("open");
        streams.take(&other, socket, RunnerMessage::Ended(ended())).unwrap();
        assert!(streams.get(&key(1)).is_none(), "the lost stream was forgotten past `keep`");
        let (again, _) = streams.take_over(key(1)).expect("opened anew");
        assert!(!ptr::eq(&*lost, &*again));
        assert!(again.snapshot().revision > before, "the row takes the new stream's figures");
    }

    #[test]
    fn only_the_newest_closed_streams_stay_replayable() {
        let streams = streams(2);
        for n in 1..=3 {
            let (stream, socket) = streams.take_over(key(n)).expect("open");
            streams.take(&stream, socket, RunnerMessage::Ended(ended())).unwrap();
        }
        assert!(streams.get(&key(1)).is_none(), "the oldest closed stream is forgotten past the bound");
        assert!(streams.get(&key(2)).is_some() && streams.get(&key(3)).is_some());
        assert_eq!(streams.open_count(), 0);
    }

    #[tokio::test]
    async fn a_lost_stream_its_runner_took_back_is_not_forgotten_when_another_closes() {
        let streams = Streams::new(1, Duration::from_millis(10), Stats::default(), frame_store());
        let (lost, socket) = streams.take_over(key(1)).expect("open");
        streams.take(&lost, socket, started()).unwrap();
        streams.take(&lost, socket, RunnerMessage::Progress(progress(10_000))).unwrap();
        streams.runner_left(lost.clone(), socket);
        timeout(Duration::from_secs(5), async {
            while lost.is_open() {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("lost once the grace passed");
        let (back, _) = streams.take_over(key(1)).expect("taken back");
        assert!(ptr::eq(&*lost, &*back), "the same stream, reopened");
        let (other, socket) = streams.take_over(key(2)).expect("open");
        streams.take(&other, socket, RunnerMessage::Ended(ended())).unwrap();
        let held = streams.get(&key(1)).expect("still held");
        assert!(ptr::eq(&*held, &*lost) && held.is_open(), "the stream taken back, still open");
        let attachment = held.attach();
        assert_eq!(attachment.first.len(), 3, "attached, started and the progress");
        assert!(attachment.live.is_some(), "still followed live");
    }
}
