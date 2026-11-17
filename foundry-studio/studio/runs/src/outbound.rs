//! Within `crucible`, `outbound` is THE SOCKET'S WRITE SIDE: the one place
//! anything is put on a browser's websocket, and the place an attachment is
//! given up when that browser stops taking the frames it asked for.
//!
//! It exists because a browser that reads slowly must cost the server a bounded
//! amount and no more. Everything to be written goes through one queue of two,
//! so a reader is at most one frame ahead of the one being written, and nothing
//! is ever queued unboundedly or dropped silently.
//!
//! The wait is on the SOCKET rather than on the queue on purpose: a queue that
//! emptied into a socket nobody was reading would report progress that had not
//! happened. It applies to FRAMES only. A frame the socket will not take within
//! the wait ends that one attachment — the browser is told the last instant it
//! actually received, every frame still queued for that attachment is thrown
//! away unsent, and the socket carries on, so the browser may re-attach on it.
//! A control reply never causes that: a reply the socket will not take within
//! the wait is a socket that has gone, and the writer ends.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use axum::extract::ws::Message;
use futures::{Sink, SinkExt};
use tokio::sync::mpsc::{Receiver, Sender, channel};
use tokio::sync::oneshot;
use tokio::time::timeout;
use tracing::{debug, warn};

use crate::protocol::Reply;

/// How many messages may wait for the socket. Two, so a reader is at most one
/// frame ahead of the one being written and the memory an attachment costs is
/// two frames.
const QUEUE: usize = 2;

/// The shortest wait a frame may be given. A wait of zero would end every
/// attachment on its first frame, so it is refused by name in `config`; this is
/// the floor that holds even so.
pub const SHORTEST_WAIT: Duration = Duration::from_secs(1);

/// One attachment as both sides of the socket know it: what to call it in
/// `lagged`, and the flag the write side stops it with.
///
/// The flag is how a reader learns it has been given up on without the write
/// side having to reach into the task: the reader checks it before each frame
/// and stops, and any frame of that attachment still in the queue is discarded.
#[derive(Clone, Debug)]
pub struct Attached {
    pub id: Arc<str>,
    stopped: Arc<AtomicBool>,
    /// The last instant the SOCKET took a frame at. It is where a browser that
    /// was given up on resumes, so it advances only on a send that returned.
    reached: Arc<AtomicU64>,
}

impl Attached {
    pub fn new(id: &str) -> Self {
        Self { id: Arc::from(id), stopped: Arc::new(AtomicBool::new(false)), reached: Arc::new(AtomicU64::new(0)) }
    }

    /// Whether the write side has given up on this attachment.
    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    /// The instant a browser resumes from: the last frame the socket took.
    pub fn reached_ms(&self) -> u64 {
        self.reached.load(Ordering::Relaxed)
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
}

/// Something on its way to a browser.
pub enum Outgoing {
    /// A JSON reply, and — for a caller that must not act until the browser has
    /// actually been told — a channel this side sends on once it has gone out.
    Control {
        reply: Reply,
        written: Option<oneshot::Sender<()>>, // keep-qualified: `Sender` here is the queue's, from mpsc
    },
    /// One worldview frame: which attachment asked for it, the instant it
    /// states, and the frame itself.
    Frame { of: Attached, t_ms: u64, bytes: Vec<u8> },
}

/// How the write side finished.
#[derive(Debug, PartialEq, Eq)]
pub enum Wrote {
    /// Everything queued was written and the queue was closed.
    Everything,
    /// The socket failed, went away, or would not take a control reply.
    SocketGone,
    /// A reply could not be encoded; the writer stopped rather than putting
    /// something the protocol does not define on the socket.
    Unencodable(String),
}

/// Open the one queue everything this socket sends goes through.
pub fn queue() -> (Sender<Outgoing>, Receiver<Outgoing>) {
    channel(QUEUE)
}

/// Write everything the queue hands over, until it closes or the socket goes.
pub async fn pump<S>(mut sink: S, mut queued: Receiver<Outgoing>, wait: Duration) -> Wrote
where
    S: Sink<Message> + Unpin,
{
    let wait = wait.max(SHORTEST_WAIT);
    while let Some(outgoing) = queued.recv().await {
        let outcome = match outgoing {
            Outgoing::Control { reply, written } => control(&mut sink, wait, reply, written).await,
            Outgoing::Frame { of, t_ms, bytes } => frame(&mut sink, wait, of, t_ms, bytes).await,
        };
        match outcome {
            Ok(()) => {}
            Err(ended) => {
                let _ = timeout(wait, sink.close()).await;
                return ended;
            }
        }
    }
    let _ = timeout(wait, sink.close()).await;
    Wrote::Everything
}

/// One control reply. It is never the cause of a dropped attachment: a socket
/// that will not take a few hundred bytes of JSON within the wait is a socket
/// that has gone.
async fn control<S>(
    sink: &mut S,
    wait: Duration,
    reply: Reply,
    written: Option<oneshot::Sender<()>>, // keep-qualified: as above
) -> Result<(), Wrote>
where
    S: Sink<Message> + Unpin,
{
    let message = encoded(&reply)?;
    match timeout(wait, sink.send(message)).await {
        Ok(Ok(())) => {
            if let Some(written) = written {
                let _ = written.send(());
            }
            Ok(())
        }
        Ok(Err(_)) => Err(Wrote::SocketGone),
        Err(_) => {
            warn!("the socket would not take a reply within the wait; giving up on it");
            Err(Wrote::SocketGone)
        }
    }
}

/// One frame of one attachment. A frame of an attachment already given up on is
/// thrown away here rather than sent, which is how its queued backlog clears.
async fn frame<S>(sink: &mut S, wait: Duration, of: Attached, t_ms: u64, bytes: Vec<u8>) -> Result<(), Wrote>
where
    S: Sink<Message> + Unpin,
{
    if of.stopped() {
        return Ok(());
    }
    match timeout(wait, sink.send(Message::Binary(bytes.into()))).await {
        // The resume instant advances only HERE, after the socket took the
        // frame: a frame that timed out never reached the browser, and naming
        // its instant would tell a browser to resume past what it holds.
        Ok(Ok(())) => {
            of.reached.store(t_ms, Ordering::Relaxed);
            Ok(())
        }
        Ok(Err(_)) => Err(Wrote::SocketGone),
        Err(_) => {
            of.stop();
            debug!(attachment = %of.id, t_ms, "the socket would not take this frame; giving up on the attachment");
            let lagged = Reply::Lagged { id: Some(of.id.to_string()), resume_from_ms: of.reached_ms() };
            control(sink, wait, lagged, None).await
        }
    }
}

fn encoded(reply: &Reply) -> Result<Message, Wrote> {
    match serde_json::to_string(reply) {
        Ok(text) => Ok(Message::Text(text.into())),
        Err(failure) => {
            warn!(%failure, "a reply would not encode; the socket is given nothing rather than half a message");
            Err(Wrote::Unencodable(failure.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::sync::Mutex;
    use std::task::{Context, Poll};

    use futures::StreamExt;
    use futures::channel::mpsc::unbounded;

    use super::*;

    /// A socket whose buffer has room for `frames_first` frames and then only
    /// for small replies: what a browser that stopped reading looks like from
    /// here, where a 12 KB frame no longer fits and a line of JSON still does.
    struct Picky {
        frames_first: usize,
        frames_taken: usize,
        pending: Option<Message>,
        written: Arc<Mutex<Vec<Message>>>,
    }

    impl Sink<Message> for Picky {
        type Error = ();

        fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
            Poll::Ready(Ok(()))
        }

        fn start_send(mut self: Pin<&mut Self>, message: Message) -> Result<(), ()> {
            // A send whose flush never returned is abandoned by its caller, and
            // the message it was carrying with it.
            self.pending = Some(message);
            Ok(())
        }

        fn poll_flush(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
            let Some(message) = self.pending.take() else { return Poll::Ready(Ok(())) };
            if matches!(message, Message::Binary(_)) && self.frames_taken >= self.frames_first {
                self.pending = Some(message);
                return Poll::Pending;
            }
            self.frames_taken += usize::from(matches!(message, Message::Binary(_)));
            self.written.lock().expect("nothing panics holding this").push(message);
            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn frame_of(of: &Attached, t_ms: u64) -> Outgoing {
        Outgoing::Frame { of: of.clone(), t_ms, bytes: vec![0; 64] }
    }

    /// The socket takes one frame and then no more. The browser is told to
    /// resume at the instant of THAT frame — the one it actually holds — and
    /// not at the instant of the frame that never went; the frames behind it
    /// are thrown away; and the socket is left open, so the `lagged` reaches
    /// the browser and it may re-attach on the same one.
    #[tokio::test(start_paused = true)]
    async fn a_dropped_attachment_resumes_at_the_last_frame_the_socket_took() {
        let of = Attached::new("7");
        let written = Arc::new(Mutex::new(Vec::new()));
        let socket = Picky { frames_first: 1, frames_taken: 0, pending: None, written: Arc::clone(&written) };
        let (sender, queued) = queue();
        let writing = tokio::spawn(pump(socket, queued, Duration::from_secs(10)));
        for slice in 1..=6u64 {
            if sender.send(frame_of(&of, slice * 10_000)).await.is_err() {
                break;
            }
        }
        drop(sender);
        assert_eq!(writing.await.expect("the pump ends"), Wrote::Everything, "the socket itself never errored");
        assert!(of.stopped(), "the attachment was given up on");
        assert_eq!(of.reached_ms(), 10_000, "the last instant the socket took, not the one that timed out");

        let written = written.lock().expect("nothing panics holding this");
        assert_eq!(written.len(), 2, "one frame and the `lagged` behind it; the other five were discarded");
        let Message::Text(ref text) = written[1] else { panic!("the second message is the reply") };
        assert!(text.contains("\"resume_from_ms\":10000"), "and it says where to resume: {text}");
    }

    /// The frames of an attachment already given up on are thrown away rather
    /// than sent, which is what clears its backlog without unbounded memory.
    #[tokio::test]
    async fn frames_of_a_stopped_attachment_are_discarded() {
        let of = Attached::new("7");
        of.stop();
        let (socket, written) = unbounded::<Message>();
        let (sender, queued) = queue();
        let writing = tokio::spawn(pump(socket, queued, Duration::from_secs(10)));
        sender.send(frame_of(&of, 10_000)).await.expect("queued");
        drop(sender);
        assert_eq!(writing.await.expect("the pump ends"), Wrote::Everything);
        assert!(written.collect::<Vec<Message>>().await.is_empty(), "nothing reached the socket");
    }

    #[tokio::test]
    async fn everything_queued_is_written_in_the_order_it_was_queued() {
        let of = Attached::new("7");
        let (socket, written) = unbounded::<Message>();
        let (sender, queued) = queue();
        let writing = tokio::spawn(pump(socket, queued, Duration::from_secs(10)));
        sender.send(frame_of(&of, 10_000)).await.expect("queued");
        let detached = Outgoing::Control { reply: Reply::Detached { id: "3".to_string() }, written: None };
        sender.send(detached).await.expect("queued");
        drop(sender);
        assert_eq!(writing.await.expect("the pump ends"), Wrote::Everything);

        let written: Vec<Message> = written.collect().await;
        assert!(matches!(written[0], Message::Binary(_)), "the frame first");
        assert!(matches!(written[1], Message::Text(_)), "then the reply behind it");
    }
}
