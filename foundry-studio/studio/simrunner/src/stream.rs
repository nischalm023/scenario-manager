//! Within `simrunner`, `stream` is the runner's live stream to the streaming server its job spec names (ruling D174):
//! `started` once the run is prepared, its `progress` at the heartbeat's pace from then on, and its final progress
//! and `ended` last, as `studio-protocol`'s `run_stream` spells them, queued for the connection that carries them to
//! the server (`connection`), and between them the run's picture: the frames of the run's recording, read from its
//! file as the harness's tap tells the file grew (`recording`). The stream is observation, never the run's business:
//! the run never waits on it, neither to queue a message nor to have its frames read, nor, once it is over, past the
//! bound its last messages have to go out. A spec naming no server opens nothing.

mod connection;
mod recording;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use dessim::sim_types::Counters;
use studio_protocol::{
    AttemptNumber, Ending, RunnerMessage, STREAM_FORMAT, StreamEnded, StreamProgress, StreamingServer, Ulid,
};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::{Sender, channel};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, timeout_at};

use crate::door::heartbeat::{EVERY, beat_of};
use recording::RecordingReader;

pub use recording::RecordingTap;

/// The messages waiting for the server at most: over an hour of progress, far more than piles up while the runner
/// reconnects. A full queue drops the newest message and counts it, since the run never waits on its stream: what a
/// viewer misses is observation lost, never the run's.
const QUEUE: usize = 1024;
/// How long the run's last messages have to go out once it is over (wall clock), so a dead server never holds the
/// runner's report.
const FINISH_WITHIN: Duration = Duration::from_secs(10);

/// Where messages are put, by the attempt and by the task putting its progress on the stream. Sending never waits.
#[derive(Clone)]
struct StreamSender {
    queue: Sender<RunnerMessage>,
    dropped: Arc<AtomicU64>,
}

impl StreamSender {
    /// `message` queued for the server, or dropped and counted when the queue is full.
    fn send(&self, message: RunnerMessage) {
        if let Err(TrySendError::Full(_)) = self.queue.try_send(message) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The open stream: its sender, the run's progress figures its progress is read off, the tap on the run's recording its
/// frames are read by, the task carrying its messages and frames to the server, and the task putting the run's progress
/// on it once the run started.
pub struct Streaming {
    sender: StreamSender,
    counters: Counters,
    tap: RecordingTap,
    carrying: JoinHandle<()>,
    progressing: OnceLock<JoinHandle<()>>,
}

impl Streaming {
    /// Open the stream to `server` for the attempt, whose progress is read off `counters` as the heartbeat reads them
    /// and whose frames off the recording the run writes at `recording`; the task connects, so this returns at once.
    pub fn open(
        server: &StreamingServer,
        run: &Ulid,
        attempt: AttemptNumber,
        token: &str,
        counters: Counters,
        recording: PathBuf,
    ) -> Self {
        let (queue, waiting) = channel(QUEUE);
        let hello = RunnerMessage::Hello { v: STREAM_FORMAT, run: run.clone(), attempt, token: token.to_owned() };
        let (tap, reader) = RecordingReader::new(recording);
        let carrying = tokio::spawn(connection::carry(server.endpoint.clone(), hello, waiting, reader));
        let sender = StreamSender { queue, dropped: Arc::default() };
        Self { sender, counters, tap, carrying, progressing: OnceLock::new() }
    }

    /// The tap the harness hands the recording's bytes (`RecordingTarget::tap`), which tells the stream how far the
    /// recording's file has grown.
    pub fn tap(&self) -> RecordingTap {
        self.tap.clone()
    }

    /// The run is prepared: `started` is queued, and from then on its progress at the heartbeat's pace, until the
    /// stream finishes.
    pub fn started(&self) {
        self.sender.send(RunnerMessage::Started { v: STREAM_FORMAT });
        let (sender, counters) = (self.sender.clone(), self.counters.clone());
        self.progressing.get_or_init(|| {
            tokio::spawn(async move {
                loop {
                    sleep(EVERY).await;
                    sender.send(progress_of(&counters));
                }
            })
        });
    }

    /// The run is over with `ending`: its progress stops, then its final progress and `ended` wait for room in the
    /// queue, which then closes, and what it holds goes out, all within `FINISH_WITHIN`; what has not gone out by
    /// then is dropped.
    pub async fn finish(self, ending: Ending) {
        let deadline = Instant::now() + FINISH_WITHIN;
        let Self { sender, counters, tap: _, mut carrying, progressing } = self;
        // Awaited once aborted, so no progress lands behind the ending and the task's sender no longer holds the
        // queue open.
        if let Some(progressing) = progressing.into_inner() {
            progressing.abort();
            let _ = progressing.await;
        }
        let ended = RunnerMessage::Ended(StreamEnded { v: STREAM_FORMAT, ending });
        for message in [progress_of(&counters), ended] {
            if timeout_at(deadline, sender.queue.send(message)).await.is_err() {
                break;
            }
        }
        let dropped = sender.dropped.load(Ordering::Relaxed);
        if dropped > 0 {
            eprintln!("simrunner: {dropped} stream messages were dropped while the stream's queue was full");
        }
        drop(sender);
        if timeout_at(deadline, &mut carrying).await.is_err() {
            carrying.abort();
            eprintln!("simrunner: the stream's last messages did not go out within {FINISH_WITHIN:?}; going on");
        }
    }
}

/// The run's progress as the stream carries it: the simtime the run's final slices reach and what they state, as the
/// heartbeat reads them.
fn progress_of(counters: &Counters) -> RunnerMessage {
    let beat = beat_of(counters);
    RunnerMessage::Progress(StreamProgress { v: STREAM_FORMAT, sim_ms: beat.sim_ms, values: beat.values })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, OnceLock};
    use std::time::Duration;

    use dessim::sim_types::Counters;
    use desunits::SimTime;
    use studio_protocol::{EndedBy, Ending, RunState, RunnerMessage, STREAM_FORMAT, StreamProgress};
    use tempfile::tempdir;
    use tokio::sync::mpsc::channel;
    use tokio::time::{sleep, timeout};

    use super::{EVERY, RecordingReader, StreamSender, Streaming};

    fn progress(sim_ms: u64) -> RunnerMessage {
        RunnerMessage::Progress(StreamProgress { v: STREAM_FORMAT, sim_ms: SimTime::ms(sim_ms), values: Vec::new() })
    }

    fn ending() -> Ending {
        Ending {
            state: RunState::Finished,
            ended_by: EndedBy::Simulator,
            stop_reason: "SimTimeLimit".to_owned(),
            stop_detail: None,
            sim_ms: SimTime::ms(600_000),
        }
    }

    /// A full queue drops the newest message and counts it, and the sender never waits: the run goes on whatever the
    /// server does.
    #[tokio::test]
    async fn a_full_queue_drops_the_newest_messages_and_never_waits() {
        let (queue, mut waiting) = channel(2);
        let sender = StreamSender { queue, dropped: Arc::default() };
        timeout(Duration::from_millis(100), async {
            for sim_ms in 1..=5 {
                sender.send(progress(sim_ms));
            }
        })
        .await
        .expect("sending never waits");
        assert_eq!(sender.dropped.load(Ordering::Relaxed), 3);
        assert_eq!(waiting.recv().await, Some(progress(1)), "the oldest kept, the newest dropped");
        assert_eq!(waiting.recv().await, Some(progress(2)));
    }

    /// From `started` on, the run's progress is queued at the heartbeat's pace; `finish` stops it and queues the final
    /// progress and `ended`, which comes last, and the queue then closes.
    #[tokio::test(start_paused = true)]
    async fn the_progress_goes_out_at_the_heartbeats_pace_from_started_until_the_ending() {
        let (queue, mut waiting) = channel(16);
        let streaming = Streaming {
            sender: StreamSender { queue, dropped: Arc::default() },
            counters: Counters::new(),
            tap: RecordingReader::new(tempdir().unwrap().path().join("recording.worldview")).0,
            carrying: tokio::spawn(async {}),
            progressing: OnceLock::new(),
        };
        streaming.started();
        sleep(EVERY * 2 + EVERY / 2).await;
        streaming.finish(ending()).await;
        let kinds = timeout(EVERY * 10, async {
            let mut kinds = Vec::new();
            while let Some(message) = waiting.recv().await {
                kinds.push(match message {
                    RunnerMessage::Hello { .. } => "hello",
                    RunnerMessage::Started { .. } => "started",
                    RunnerMessage::Progress(_) => "progress",
                    RunnerMessage::Ended(_) => "ended",
                });
            }
            kinds
        })
        .await
        .expect("the queue closes once the stream finished");
        assert_eq!(kinds, ["started", "progress", "progress", "progress", "ended"]);
    }
}
