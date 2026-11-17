//! Within `stream`, `connection` is how the stream reaches the server: a websocket to its runners' port, a `hello`
//! with the attempt's token and the server's welcome within their bounds, then each message as it is queued and, from
//! `started` on, each frame of the run's recording as the file holds it whole (`recording`), every one before `ended`.
//! A write that does not go within its bound, the server's close or its silence loses the connection, which is made
//! again at most `RETRY_AT_MOST` apart, `hello`, `started` and the scenario block said again, the open segment again
//! from its keyframe (with no keyframe yet, everything past the scenario block, an end block among it), and then the
//! message the lost one did not take, so a short outage does not end the stream while the queue keeps what comes
//! meanwhile and the file the frames. A refusal the protocol calls final ends the stream for the run.

use std::time::Duration;

use futures::{Sink, SinkExt, Stream, StreamExt};
use studio_protocol::run_stream::{RETRY_AT_MOST, SILENT_AFTER};
use studio_protocol::{RunnerMessage, RunnerReply, StreamRefusal};
use tokio::net::TcpStream;
use tokio::select;
use tokio::sync::mpsc::Receiver;
use tokio::time::{Instant, sleep, sleep_until, timeout};
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::{Error as SocketError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use super::recording::RecordingReader;
use crate::door::chain;

/// How long connecting and the server's welcome may take (wall clock).
const ANSWER_WITHIN: Duration = Duration::from_secs(5);
/// How long one write to the server may take before the connection is taken for lost (wall clock).
const WRITE_WITHIN: Duration = Duration::from_secs(10);
/// The first wait after a lost connection, which doubles with each try up to `RETRY_AT_MOST`.
const RETRY_FIRST: Duration = Duration::from_secs(1);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Why a connection ended or could not be made: worth another connection, in these words, or the server's final
/// refusal.
enum Lost {
    Again(String),
    Final(StreamRefusal),
}

/// Connect, say hello, carry the queue and the recording's frames to the server until the queue closes; on a lost
/// connection, connect again with the wait doubling up to `RETRY_AT_MOST`, saying `started`, the scenario block and
/// the open segment again first and then the message the lost connection did not take. A final refusal drops
/// everything that comes.
pub(super) async fn carry(
    endpoint: String,
    hello: RunnerMessage,
    mut queue: Receiver<RunnerMessage>,
    mut recording: RecordingReader,
) {
    let mut started: Option<RunnerMessage> = None;
    let mut unsent: Option<RunnerMessage> = None;
    let mut wait = RETRY_FIRST;
    let mut said = false;
    loop {
        let lost = match connected(&endpoint, &hello).await {
            Ok(socket) => {
                if said {
                    eprintln!("simrunner: the stream to {endpoint} is back");
                    said = false;
                }
                wait = RETRY_FIRST;
                let (sink, source) = socket.split();
                match carried(sink, source, &mut queue, &mut started, &mut unsent, &mut recording).await {
                    Ok(()) => return,
                    Err(lost) => lost,
                }
            }
            Err(lost) => lost,
        };
        // A closed queue with nothing left in it, and no message a lost connection did not take, has nothing to send.
        if queue.is_closed() && queue.is_empty() && unsent.is_none() {
            return;
        }
        match lost {
            Lost::Final(refusal) => {
                eprintln!(
                    "simrunner: the stream to {endpoint} was refused ({}); the run goes on unstreamed",
                    refusal.reason()
                );
                while queue.recv().await.is_some() {}
                return;
            }
            Lost::Again(words) if !said => {
                eprintln!("simrunner: no stream to {endpoint}: {words}; trying again, the run going on");
                said = true;
            }
            Lost::Again(_) => {}
        }
        sleep(wait).await;
        wait = (wait * 2).min(RETRY_AT_MOST);
    }
}

/// A socket to the server's runners' port, hello said and welcome read within the bound.
async fn connected(endpoint: &str, hello: &RunnerMessage) -> Result<Socket, Lost> {
    let (mut socket, _) = timeout(ANSWER_WITHIN, connect_async(endpoint))
        .await
        .map_err(|_| Lost::Again("it did not answer".to_owned()))?
        .map_err(|fault| Lost::Again(chain(&fault)))?;
    match timeout(WRITE_WITHIN, socket.send(Message::text(hello.text()))).await {
        Ok(Ok(())) => {}
        Ok(Err(fault)) => return Err(Lost::Again(chain(&fault))),
        Err(_) => return Err(Lost::Again("the hello did not go out in time".to_owned())),
    }
    match timeout(ANSWER_WITHIN, socket.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => match RunnerReply::read(&text) {
            Ok(RunnerReply::Welcome { .. }) => Ok(socket),
            Err(refusal) => Err(refused(refusal)),
        },
        Ok(Some(Ok(Message::Close(frame)))) => Err(closed(frame)),
        Ok(Some(Ok(_)) | None) => Err(Lost::Again("it closed on the hello".to_owned())),
        Ok(Some(Err(fault))) => Err(Lost::Again(chain(&fault))),
        Err(_) => Err(Lost::Again("it did not welcome the hello in time".to_owned())),
    }
}

/// The queue and the recording's frames carried on `socket` until the queue closes, `Ok`, or the connection is lost:
/// a write that fails or does not go in time, the server's close, or the server silent past `SILENT_AFTER` though it
/// pings every few seconds, which is a connection lost without a close. Its pings are answered as its socket is read.
/// No frame goes before `started`, which the server would refuse.
async fn carried(
    mut sink: impl Sink<Message, Error = SocketError> + Unpin,
    mut source: impl Stream<Item = Result<Message, SocketError>> + Unpin,
    queue: &mut Receiver<RunnerMessage>,
    started: &mut Option<RunnerMessage>,
    unsent: &mut Option<RunnerMessage>,
    recording: &mut RecordingReader,
) -> Result<(), Lost> {
    // `started` again first, which stays remembered whether or not this goes, and the scenario block, then what the
    // last connection lost.
    if let Some(started) = started {
        sent(&mut sink, Message::text(started.text())).await?;
        if let Some(scenario) = recording.rewound().await {
            sent(&mut sink, Message::binary(scenario)).await?;
        }
    }
    if let Some(message) = unsent.take() {
        written(&mut sink, message, unsent, recording).await?;
    }
    let mut heard_at = Instant::now();
    loop {
        select! {
            next = queue.recv() => match next {
                Some(message) => {
                    if let RunnerMessage::Started { .. } = message {
                        *started = Some(message.clone());
                    }
                    written(&mut sink, message, unsent, recording).await?;
                }
                None => {
                    // Everything went out, the ending last; the closing handshake is a courtesy.
                    let _ = timeout(WRITE_WITHIN, sink.close()).await;
                    return Ok(());
                }
            },
            () = recording.readable(), if started.is_some() => {
                if let Some(frame) = recording.next_frame().await {
                    sent(&mut sink, Message::binary(frame)).await?;
                }
            }
            heard = source.next() => {
                heard_at = Instant::now();
                heard_from(heard)?;
            }
            () = sleep_until(heard_at + SILENT_AFTER) => {
                return Err(Lost::Again(format!("the server was silent for {SILENT_AFTER:?}")));
            }
        }
    }
}

/// `message` written within the bound, or kept in `unsent` for the next connection. `ended` goes after every frame the
/// recording holds, which the run finished writing before its ending was queued.
async fn written(
    sink: &mut (impl Sink<Message, Error = SocketError> + Unpin),
    message: RunnerMessage,
    unsent: &mut Option<RunnerMessage>,
    recording: &mut RecordingReader,
) -> Result<(), Lost> {
    let outcome = async {
        if let RunnerMessage::Ended(_) = message {
            while let Some(frame) = recording.next_frame().await {
                sent(sink, Message::binary(frame)).await?;
            }
        }
        sent(sink, Message::text(message.text())).await
    }
    .await;
    if outcome.is_err() {
        *unsent = Some(message);
    }
    outcome
}

/// One websocket message written within the bound.
async fn sent(sink: &mut (impl Sink<Message, Error = SocketError> + Unpin), message: Message) -> Result<(), Lost> {
    match timeout(WRITE_WITHIN, sink.send(message)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(fault)) => Err(Lost::Again(chain(&fault))),
        Err(_) => Err(Lost::Again(format!("a write did not go within {WRITE_WITHIN:?}"))),
    }
}

/// What the server sent: nothing but pings and pongs is expected before its close.
fn heard_from(heard: Option<Result<Message, SocketError>>) -> Result<(), Lost> {
    match heard {
        Some(Ok(Message::Close(frame))) => Err(closed(frame)),
        Some(Ok(_)) => Ok(()),
        Some(Err(fault)) => Err(Lost::Again(chain(&fault))),
        None => Err(Lost::Again("the server closed the connection".to_owned())),
    }
}

fn closed(frame: Option<CloseFrame>) -> Lost {
    let reason = frame.map(|frame| frame.reason.to_string()).unwrap_or_default();
    match StreamRefusal::from_reason(&reason) {
        Some(refusal) => refused(refusal),
        None => Lost::Again(format!("it closed the connection ({reason:?})")),
    }
}

fn refused(refusal: StreamRefusal) -> Lost {
    if refusal.is_final_for_runner() {
        Lost::Final(refusal)
    } else {
        Lost::Again(format!("it refused: {}", refusal.reason()))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;

    use desunits::SimTime;
    use futures::channel::mpsc::unbounded;
    use futures::{SinkExt, StreamExt, sink, stream};
    use studio_protocol::{EndedBy, Ending, RunState, RunnerMessage, STREAM_FORMAT, StreamEnded};
    use tempfile::tempdir;
    use tokio::sync::mpsc::channel;
    use tokio_tungstenite::tungstenite::{Error as SocketError, Message};
    use worldview::{Frame, Ms, Tag};

    use super::{Lost, RecordingReader, carried};

    fn ended() -> RunnerMessage {
        let ending = Ending {
            state: RunState::Finished,
            ended_by: EndedBy::Simulator,
            stop_reason: "SimTimeLimit".to_owned(),
            stop_detail: None,
            sim_ms: SimTime::ms(600_000),
        };
        RunnerMessage::Ended(StreamEnded { v: STREAM_FORMAT, ending })
    }

    /// The message a lost connection did not take, the ending say, waits for the next connection even when that one
    /// fails too while saying `started` again, rather than giving way to it.
    #[tokio::test]
    async fn the_message_a_lost_connection_did_not_take_outlives_a_failed_reconnection() {
        let failing =
            Box::pin(sink::unfold((), |(), _: Message| async { Err::<(), _>(SocketError::ConnectionClosed) }));
        let silent = stream::pending::<Result<Message, SocketError>>();
        let (_queue, mut waiting) = channel(4);
        let started = RunnerMessage::Started { v: STREAM_FORMAT };
        let (mut said, mut unsent) = (Some(started.clone()), Some(ended()));
        let folder = tempdir().unwrap();
        let (_tap, mut recording) = RecordingReader::new(folder.path().join("recording.worldview"));
        let lost = carried(failing, silent, &mut waiting, &mut said, &mut unsent, &mut recording).await;
        assert!(matches!(lost, Err(Lost::Again(_))), "the write failed, so the connection is lost");
        assert_eq!((said, unsent), (Some(started), Some(ended())), "both are kept for the next connection");
    }

    /// The recording's frames go out after `started` and in the file's order, the end block last of them, every one
    /// before `ended`, and the index block not at all.
    #[tokio::test]
    async fn the_recordings_frames_follow_started_and_all_go_before_the_ending() {
        let folder = tempdir().unwrap();
        let path = folder.path().join("recording.worldview");
        let (mut tap, mut recording) = RecordingReader::new(path.clone());
        let frames: Vec<Vec<u8>> = [(Tag::Scenario, 0), (Tag::Keyframe, 0), (Tag::Delta, 0), (Tag::Delta, 1_000)]
            .into_iter()
            .map(|(tag, t)| Frame::encode(tag, Ms(t), b"body", true).expect("a frame"))
            .collect();
        let [index, end] =
            [Tag::Index, Tag::End].map(|tag| Frame::encode(tag, Ms(1_000), b"closing", true).expect("a frame"));
        let file = [frames.concat(), index, end.clone()].concat();
        fs::write(&path, &file).expect("the recording");
        tap.write_all(&file).expect("the tap never refuses");
        let (written, said) = unbounded::<Message>();
        let sink = written.sink_map_err(|_| SocketError::ConnectionClosed);
        let (queue, mut waiting) = channel(4);
        queue.send(RunnerMessage::Started { v: STREAM_FORMAT }).await.unwrap();
        queue.send(ended()).await.unwrap();
        drop(queue);
        let silent = stream::pending::<Result<Message, SocketError>>();
        let (mut started, mut unsent) = (None, None);
        let carried = carried(sink, silent, &mut waiting, &mut started, &mut unsent, &mut recording).await;
        assert!(carried.is_ok(), "the queue closed with everything sent");
        let said: Vec<Message> = said.collect().await;
        let mut expected = vec![Message::text(RunnerMessage::Started { v: STREAM_FORMAT }.text())];
        expected.extend(frames.into_iter().map(Message::binary));
        expected.push(Message::binary(end));
        expected.push(Message::text(ended().text()));
        assert_eq!(said, expected);
    }
}
