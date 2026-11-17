//! The runner's live stream (`stream`), against stub streaming servers that record what arrives: a canned run of
//! minutes of simtime says hello with its token, is welcomed, and sends `started`, the progress its counters reached,
//! and `ended` last, and between them every frame of the run's recording, byte for byte and in the file's order, the
//! index block excepted and the end block last; a connection the server drops is made again, with `hello`, `started`
//! and the scenario block said again and the recording's frames again from a keyframe on, and the stream goes on to
//! its ending; a final refusal is taken as final; and a server that does not answer leaves the run alone and never
//! holds the stream's end.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use dessim::harness::{RunOverrides, prepare};
use dessim::sim_types::Counters;
use desunits::SimTime;
use simrunner::simulate::{RECORDING_FILE, simulate};
use simrunner::stop::RunnerStop;
use simrunner::stream::Streaming;
use studio_protocol::{
    AttemptNumber, EndedBy, Ending, RunState, RunnerMessage, RunnerReply, STREAM_FORMAT, StreamEnded, StreamRefusal,
    StreamingServer, Ulid,
};
use tempfile::{TempDir, tempdir};
use tokio::net::TcpListener;
use tokio::task::spawn_blocking;
use tokio::time::{Instant, sleep, timeout};
use worldview::{Frame, Tag};

const RUN: &str = "01KZAVPQNES65HV4EZADPCS7E8";
const TOKEN: &str = "the-attempts-token";

/// What a stub server does with each connection: welcome it and record what comes; welcome it, record a few messages
/// and drop it, the first time only; or refuse its hello.
#[derive(Clone, Copy)]
enum Behaviour {
    Welcome,
    DropFirstAfter(usize),
    Refuse(StreamRefusal),
}

/// What a connection received: a text message, read, or a binary frame, as its bytes.
#[derive(Debug, Clone)]
enum Got {
    Text(RunnerMessage),
    Frame(Vec<u8>),
}

struct Stub {
    behaviour: Behaviour,
    connections: AtomicUsize,
    /// What each connection received, in order.
    received: Mutex<Vec<Vec<Got>>>,
}

async fn stub(behaviour: Behaviour) -> (StreamingServer, Arc<Stub>) {
    let stub = Arc::new(Stub { behaviour, connections: AtomicUsize::new(0), received: Mutex::default() });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}/runners", listener.local_addr().unwrap());
    let router = Router::new().route("/runners", get(upgrade)).with_state(stub.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (StreamingServer { server: "01KZAVPQNES65HV4EZADPCSZZ1".parse().unwrap(), endpoint }, stub)
}

async fn upgrade(upgrade: WebSocketUpgrade, State(stub): State<Arc<Stub>>) -> impl IntoResponse {
    upgrade.on_upgrade(move |socket| record(socket, stub))
}

async fn record(mut socket: WebSocket, stub: Arc<Stub>) {
    let connection = stub.connections.fetch_add(1, Ordering::SeqCst);
    let index = {
        let mut received = stub.received.lock().unwrap();
        received.push(Vec::new());
        received.len() - 1
    };
    while let Some(Ok(message)) = socket.recv().await {
        let got = match message {
            Message::Text(text) => Got::Text(RunnerMessage::read(&text).expect("a runner's message")),
            Message::Binary(bytes) => Got::Frame(bytes.to_vec()),
            _ => continue,
        };
        let hello = matches!(got, Got::Text(RunnerMessage::Hello { .. }));
        let count = {
            let mut received = stub.received.lock().unwrap();
            received[index].push(got);
            received[index].len()
        };
        match stub.behaviour {
            Behaviour::Refuse(refusal) if hello => {
                let frame = CloseFrame { code: 1008, reason: refusal.reason().into() };
                let _ = socket.send(Message::Close(Some(frame))).await;
                return;
            }
            _ if hello => {
                let welcome = RunnerReply::Welcome { v: STREAM_FORMAT }.text();
                socket.send(Message::Text(welcome.into())).await.unwrap();
            }
            Behaviour::DropFirstAfter(after) if connection == 0 && count > after => return,
            _ => {}
        }
    }
}

fn run() -> Ulid {
    RUN.parse().unwrap()
}

/// An ending for a stream whose run this test does not simulate.
fn ending() -> Ending {
    Ending {
        state: RunState::Finished,
        ended_by: EndedBy::Simulator,
        stop_reason: "SimTimeLimit".to_owned(),
        stop_detail: None,
        sim_ms: SimTime::ms(300_000),
    }
}

/// The stream to `server` for the attempt, whose progress is read off `counters` and whose frames off the recording a
/// run writes into the folder that comes with it.
fn opened(server: &StreamingServer, counters: &Counters) -> (Streaming, TempDir) {
    let out = tempdir().unwrap();
    let recording = out.path().join(RECORDING_FILE);
    (Streaming::open(server, &run(), AttemptNumber(1), TOKEN, counters.clone(), recording), out)
}

/// `seconds` of simtime of the canned fixture moving `counters` and writing its recording into `out`, `started` on
/// `streaming` once prepared and the recording tapped for it: the ending.
async fn streamed_run(streaming: &Streaming, counters: &Counters, out: &Path, seconds: u64) -> Ending {
    let spec = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/canned/spec.json");
    let overrides = RunOverrides {
        sim_duration: Some(Duration::from_secs(seconds)),
        num_threads: Some(1),
        ..RunOverrides::default()
    };
    let prepared = prepare(&spec, &overrides).expect("the fixture prepares");
    streaming.started();
    let (moved, tap, out) = (counters.clone(), Some(streaming.tap()), out.to_path_buf());
    let left =
        spawn_blocking(move || simulate(prepared, &out, &run(), AttemptNumber(1), RunnerStop::new(), moved, tap))
            .await
            .unwrap()
            .expect("the run simulates");
    left.ending
}

/// The frames of the recording the run left in `out`, as its file holds them through the end block, the index block
/// before it passed over.
fn recorded_frames(out: &Path) -> Vec<Vec<u8>> {
    let bytes = fs::read(out.join(RECORDING_FILE)).expect("the recording");
    let (mut frames, mut at) = (Vec::new(), 0);
    loop {
        let (frame, len) = Frame::decode_at(&bytes[at..], at as u64).expect("a whole frame");
        if frame.header.tag != Tag::Index {
            frames.push(bytes[at..at + len].to_vec());
        }
        if frame.header.tag == Tag::End {
            return frames;
        }
        at += len;
    }
}

/// Until `stub` has a message that is the ending, within 10 s: what each connection received, and the texts alone.
async fn ended_at(stub: &Stub) -> (Vec<Vec<RunnerMessage>>, Vec<Vec<Got>>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let received = stub.received.lock().unwrap().clone();
        let ended = received.last().and_then(|got| got.last());
        if matches!(ended, Some(Got::Text(RunnerMessage::Ended(_)))) || Instant::now() > deadline {
            return (received.iter().map(|got| texts(got)).collect(), received);
        }
        drop(received);
        sleep(Duration::from_millis(50)).await;
    }
}

fn texts(got: &[Got]) -> Vec<RunnerMessage> {
    got.iter()
        .filter_map(|got| match got {
            Got::Text(message) => Some(message.clone()),
            Got::Frame(_) => None,
        })
        .collect()
}

/// Where two runs of frames first differ, by each one's tag and instant there, for a failure to name.
fn first_difference(sent: &[Vec<u8>], recorded: &[Vec<u8>]) -> String {
    let at = sent
        .iter()
        .zip(recorded)
        .position(|(sent, recorded)| sent != recorded)
        .unwrap_or(sent.len().min(recorded.len()));
    let said = |frames: &[Vec<u8>]| {
        frames.get(at).map(|bytes| Frame::decode_at(bytes, 0).map(|(frame, _)| (frame.header.tag, frame.header.t.0)))
    };
    format!(
        "{} sent, {} recorded, first apart at {at}: sent {:?}, recorded {:?}",
        sent.len(),
        recorded.len(),
        said(sent),
        said(recorded)
    )
}

/// The frames received, each one whole container frame and nothing beside it.
fn frames(got: &[Got]) -> Vec<Vec<u8>> {
    got.iter()
        .filter_map(|got| match got {
            Got::Frame(bytes) => {
                let (_, used) = Frame::decode_at(bytes, 0).expect("one whole container frame");
                assert_eq!(used, bytes.len(), "and nothing beside it");
                Some(bytes.clone())
            }
            Got::Text(_) => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_streams_hello_started_its_progress_and_its_ending_last() {
    let (server, stub) = stub(Behaviour::Welcome).await;
    let counters = Counters::new();
    let (streaming, out) = opened(&server, &counters);
    let ending = streamed_run(&streaming, &counters, out.path(), 300).await;
    streaming.finish(ending.clone()).await;
    let (received, _) = ended_at(&stub).await;
    assert_eq!(received.len(), 1, "one connection");
    let messages = &received[0];
    assert!(
        matches!(&messages[0], RunnerMessage::Hello { token, attempt, .. } if token == TOKEN && attempt.0 == 1),
        "{:?}",
        messages[0]
    );
    assert!(matches!(&messages[1], RunnerMessage::Started { .. }), "{:?}", messages[1]);
    assert_eq!(messages.last(), Some(&RunnerMessage::Ended(StreamEnded { v: STREAM_FORMAT, ending })));
    let Some(RunnerMessage::Progress(last)) = messages.iter().rev().nth(1) else {
        panic!("the progress just before the ending: {messages:?}")
    };
    assert!(last.sim_ms.as_ms() >= 290_000, "the run's own progress, five minutes of it: {last:?}");
    assert!(last.values.iter().any(|value| value.metric == "picks"), "its counters by name: {last:?}");
}

/// Twenty minutes of simtime: every frame of the recording goes out once, byte for byte and in the file's order, the
/// first right after `started` and the end block last, just before the ending; the index block stays in the file.
#[tokio::test(flavor = "multi_thread")]
async fn a_run_streams_every_frame_of_its_recording_once_between_started_and_its_ending() {
    let (server, stub) = stub(Behaviour::Welcome).await;
    let counters = Counters::new();
    let (streaming, out) = opened(&server, &counters);
    let ending = streamed_run(&streaming, &counters, out.path(), 1_200).await;
    streaming.finish(ending).await;
    let (_, received) = ended_at(&stub).await;
    assert_eq!(received.len(), 1, "one connection");
    let got = &received[0];
    let (sent, recorded) = (frames(got), recorded_frames(out.path()));
    assert!(recorded.len() > 120, "a delta a slice and a keyframe every thirty: {}", recorded.len());
    assert!(sent == recorded, "the recording's frames: {}", first_difference(&sent, &recorded));
    let (last, _) = Frame::decode_at(sent.last().expect("frames"), 0).unwrap();
    assert_eq!(last.header.tag, Tag::End, "the end block last");
    assert_eq!(got.iter().position(|got| matches!(got, Got::Frame(_))), Some(2), "right after hello and started");
    assert!(matches!(got.last(), Some(Got::Text(RunnerMessage::Ended(_)))), "the ending after the last frame");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_connection_is_made_again_and_the_stream_goes_on_to_its_ending() {
    let (server, stub) = stub(Behaviour::DropFirstAfter(12)).await;
    let counters = Counters::new();
    let (streaming, out) = opened(&server, &counters);
    let ending = streamed_run(&streaming, &counters, out.path(), 300).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    while stub.connections.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
        sleep(Duration::from_millis(50)).await;
    }
    streaming.finish(ending.clone()).await;
    let (received, got) = ended_at(&stub).await;
    assert_eq!(received.len(), 2, "the dropped connection and the one made again");
    assert!(matches!(received[0][..2], [RunnerMessage::Hello { .. }, RunnerMessage::Started { .. }]), "{received:?}");
    assert!(frames(&got[0]).len() > 3, "the first connection had frames before it was dropped");
    let again = &received[1];
    assert!(matches!(again[0], RunnerMessage::Hello { .. }), "{:?}", again[0]);
    assert!(matches!(again[1], RunnerMessage::Started { .. }), "started said again: {:?}", again[1]);
    assert_eq!(again.last(), Some(&RunnerMessage::Ended(StreamEnded { v: STREAM_FORMAT, ending })));
    // The scenario block again, then the frames from a keyframe on: the open segment again, and everything after it.
    let (resent, recorded) = (frames(&got[1]), recorded_frames(out.path()));
    assert!(resent.first() == recorded.first(), "the scenario block again");
    let from = recorded.iter().position(|frame| *frame == resent[1]).expect("a frame of the recording");
    let (keyframe, _) = Frame::decode_at(&recorded[from], 0).unwrap();
    assert_eq!(keyframe.header.tag, Tag::Keyframe, "the frames again from a keyframe");
    assert!(
        resent[1..] == recorded[from..],
        "and every frame after it, once: {}",
        first_difference(&resent[1..], &recorded[from..])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_final_refusal_ends_the_stream_and_no_other_connection_is_made() {
    let (server, stub) = stub(Behaviour::Refuse(StreamRefusal::TokenInvalid)).await;
    let (streaming, _out) = opened(&server, &Counters::new());
    streaming.started();
    sleep(Duration::from_millis(2_500)).await;
    let began = Instant::now();
    timeout(Duration::from_secs(12), streaming.finish(ending())).await.expect("the end is bounded");
    assert!(began.elapsed() < Duration::from_secs(1), "nothing was left to wait for");
    assert_eq!(stub.connections.load(Ordering::SeqCst), 1, "a refused token is not tried again");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_does_not_answer_leaves_the_run_alone_and_never_holds_its_end() {
    // A port nothing listens on.
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}/runners", closed.local_addr().unwrap());
    drop(closed);
    let server = StreamingServer { server: "01KZAVPQNES65HV4EZADPCSZZ1".parse().unwrap(), endpoint };
    let (streaming, _out) = opened(&server, &Counters::new());
    streaming.started();
    let began = Instant::now();
    timeout(Duration::from_secs(12), streaming.finish(ending())).await.expect("the end is bounded");
    assert!(began.elapsed() <= Duration::from_secs(11), "within the stream's finishing bound");
}
