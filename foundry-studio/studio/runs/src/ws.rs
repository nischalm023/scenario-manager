//! Within `crucible`, `ws` is ONE BROWSER'S SOCKET: it reads control messages
//! off a websocket, answers each from `runs` and the registry, and — while an
//! attachment is open — carries that attachment's frames on the same socket.
//!
//! The socket has two sides and they are kept apart. This module reads; every
//! byte written goes through [`outbound`]'s one queue, so a reply and a frame
//! cannot interleave halfway and a browser that stops reading is noticed in one
//! place. One attachment at a time: a second `attach` replaces the first, and
//! `detach` ends it.
//!
//! It holds no state of its own beyond the socket and that one attachment. A
//! browser that reloads, or a client that opens a socket per command, loses
//! nothing, because everything a run is lives in the registry and in storage.

use std::mem::replace as put_in_place;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures::StreamExt;
use futures::stream::SplitStream;
use serde_json::Value;
use tokio::sync::mpsc::Sender;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::attach::{AttachError, Attachment, Wanted};
use crate::outbound::{self, Attached, Outgoing};
use crate::protocol::{AttachFrom, Reply, Request, RequestId, SubmittedSpec};
use crate::recording::{self, RecordingError};
use crate::runs::Runs;
use crate::series;

/// Read this socket's requests until it closes, or until the browser stops
/// accepting what it asked for.
pub async fn serve(socket: WebSocket, runs: Arc<Runs>, max_message_bytes: usize) {
    let (sink, incoming) = socket.split();
    let (outgoing, queued) = outbound::queue();
    let writing = tokio::spawn(outbound::pump(sink, queued, runs.config().attach_lag));

    // Before anything is asked for: what this server will take in one message.
    // A submission is built in the browser out of files a person chose, so the
    // form has to know the ceiling while they are still choosing.
    let hello = Reply::Hello { max_message_bytes: max_message_bytes as u64 };
    if outgoing.send(Outgoing::Control { reply: hello, written: None }).await.is_ok() {
        read_requests(incoming, &runs, &outgoing, max_message_bytes).await;
    }

    drop(outgoing);
    let outcome = writing.await;
    debug!(?outcome, "the socket is done");
}

/// The read side: one request at a time, until the socket closes or the write
/// side gives up on this browser.
async fn read_requests(
    mut incoming: SplitStream<WebSocket>,
    runs: &Arc<Runs>,
    outgoing: &Sender<Outgoing>,
    max_message_bytes: usize,
) {
    let mut attached: Option<JoinHandle<()>> = None;
    loop {
        let message = tokio::select! {
            biased;
            // The queue closes when the write side ends, which is how this
            // side learns that the browser stopped taking its frames.
            _ = outgoing.closed() => break,
            message = incoming.next() => message,
        };
        let reply = match message {
            None => break,
            Some(Ok(Message::Text(text))) if text.len() > max_message_bytes => Reply::Error {
                id: request_id(&text),
                message: format!("the message is {} bytes; this server accepts {max_message_bytes}", text.len()),
            },
            Some(Ok(Message::Text(text))) => match read(&text) {
                Err(reply) => reply,
                Ok(Request::Attach { id, run, from, sections }) => {
                    replace(&mut attached, Some(watch(runs, outgoing, id, run, from, sections)));
                    continue;
                }
                Ok(Request::Detach { id }) => match replace(&mut attached, None) {
                    true => Reply::Detached { id },
                    false => Reply::Error { id: Some(id), message: "nothing is attached on this socket".into() },
                },
                Ok(Request::Submit { id, spec }) => {
                    if submitted(runs, outgoing, id, &spec).await.is_err() {
                        break;
                    }
                    continue;
                }
                Ok(request) => answer(request, runs).await,
            },
            Some(Ok(Message::Binary(_))) => Reply::Error {
                id: None,
                message: "binary messages travel from the server to the browser, never the other way".to_string(),
            },
            Some(Ok(Message::Close(_))) => break,
            // Ping and Pong are axum's to answer.
            Some(Ok(_)) => continue,
            Some(Err(failure)) => {
                debug!(%failure, "the socket ended");
                break;
            }
        };
        if outgoing.send(Outgoing::Control { reply, written: None }).await.is_err() {
            break;
        }
    }
    replace(&mut attached, None);
}

/// Take a submission, tell the browser its run exists, and only then let a
/// worker claim it: until the reply has gone out, the client could be told a
/// run had finished before it was told the run existed. Reports whether the
/// socket is still there.
async fn submitted(
    runs: &Arc<Runs>,
    outgoing: &Sender<Outgoing>,
    id: RequestId,
    spec: &SubmittedSpec,
) -> Result<(), ()> {
    let accepted = match runs.submit(spec).await {
        Ok(accepted) => accepted,
        Err(failure) => {
            let reply = Reply::Error { id: Some(id), message: failure.to_string() };
            return outgoing.send(Outgoing::Control { reply, written: None }).await.map_err(|_| ());
        }
    };
    let run = accepted.run.clone();
    let (written, gone_out) = oneshot::channel();
    let reply = Reply::Submitted { id, run };
    outgoing.send(Outgoing::Control { reply, written: Some(written) }).await.map_err(|_| ())?;
    gone_out.await.map_err(|_| ())?;
    runs.activate(accepted).await;
    Ok(())
}

/// Put a new attachment on this socket in place of whatever was there, and say
/// whether anything was. One socket reads one recording at a time: a second
/// `attach` is a browser seeking outside the frames it holds, and the reader it
/// had is of no further use to it.
fn replace(attached: &mut Option<JoinHandle<()>>, next: Option<JoinHandle<()>>) -> bool {
    match put_in_place(attached, next) {
        Some(previous) => {
            previous.abort();
            true
        }
        None => false,
    }
}

fn read(text: &str) -> Result<Request, Reply> {
    serde_json::from_str(text).map_err(|failure| Reply::Error {
        // A message whose shape is wrong may still have said which request it
        // was; a client multiplexing several on one socket cannot match an
        // error that drops the id it plainly sent.
        id: request_id(text),
        message: format!("this is not a request: {failure}"),
    })
}

/// The id a message carried, whatever else was wrong with it.
fn request_id(text: &str) -> Option<String> {
    serde_json::from_str::<Value>(text).ok()?.get("id").and_then(Value::as_str).map(str::to_string)
}

/// Start reading a recording to this socket. The work is a task of its own so
/// that requests — `detach` above all — are still answered while the frames go.
fn watch(
    runs: &Arc<Runs>,
    outgoing: &Sender<Outgoing>,
    id: RequestId,
    run: String,
    from: AttachFrom,
    sections: Vec<String>,
) -> JoinHandle<()> {
    let runs = Arc::clone(runs);
    let outgoing = outgoing.clone();
    tokio::spawn(async move {
        if let Err(message) = stream(&runs, &outgoing, &id, &run, from, sections).await {
            let reply = Reply::Error { id: Some(id), message };
            let _ = outgoing.send(Outgoing::Control { reply, written: None }).await;
        }
    })
}

/// One attachment, from the recording it opens to the end frame it sends.
async fn stream(
    runs: &Arc<Runs>,
    outgoing: &Sender<Outgoing>,
    id: &str,
    run: &str,
    from: AttachFrom,
    sections: Vec<String>,
) -> Result<(), String> {
    let config = runs.config();
    let held = runs
        .cache()
        .hold(runs.registry(), runs.storage().as_ref(), &config.temp_dir, run)
        .await
        .map_err(|failure: RecordingError| failure.to_string())?;
    let wanted = Wanted::parse(&sections).map_err(|failure: AttachError| failure.to_string())?;

    // Opening reads and re-encodes the scenario block, and compiling the floor
    // loads a site-sized document, so both go somewhere blocking is allowed.
    let opening = held.path.clone();
    let attempt = held.attempt.get();
    let named = run.to_string();
    // Compiling a site-sized document is seconds of arithmetic and opening an
    // attachment re-encodes the scenario block, so both go where blocking is
    // allowed; the compile is guarded, so two attachments do it once.
    let scene =
        runs.compiling().of(&opening).await.map_err(|failure| damaged(&named, attempt, &failure.to_string()))?;
    let named = run.to_string();
    let (opened, mut attachment) = tokio::task::spawn_blocking(move || {
        Attachment::open(&opening, &from, wanted, scene)
            .map_err(|failure| damaged(&named, attempt, &failure.to_string()))
    })
    .await
    .map_err(|joined| joined.to_string())??;

    let attached = Reply::Attached {
        id: id.to_string(),
        run: run.to_string(),
        attempt,
        from_ms: opened.from_ms,
        live_edge_ms: opened.live_edge_ms,
    };
    if outgoing.send(Outgoing::Control { reply: attached, written: None }).await.is_err() {
        return Ok(());
    }

    // Everything this attachment sends is stamped with it, so the write side
    // can give up on THIS reading and no other, and so a `lagged` says which.
    let of = Attached::new(id);
    while let Some(next) = attachment.next_frame() {
        if of.stopped() {
            debug!(run, attempt, "the write side gave up on this attachment; the reader stops");
            return Ok(());
        }
        let sent = next.map_err(|failure| damaged(run, attempt, &failure.to_string()))?;
        let frame = Outgoing::Frame { of: of.clone(), t_ms: sent.t_ms, bytes: sent.bytes };
        if outgoing.send(frame).await.is_err() {
            return Ok(());
        }
    }
    let ended = Reply::Ended { id: id.to_string(), run: run.to_string(), outcome: attachment.outcome().to_string() };
    let _ = outgoing.send(Outgoing::Control { reply: ended, written: None }).await;
    Ok(())
}

/// What a browser is told when the recording it is being read will not read,
/// and what the log says about it: which run, which attempt of it, and which
/// frame. A message that named none of the three would send whoever read it
/// looking through every recording the server holds.
fn damaged(run: &str, attempt: u32, failure: &str) -> String {
    warn!(run, attempt, failure, "the recording would not read");
    format!("run {run} attempt {attempt}: the recording would not read: {failure}")
}

/// Read one request that is answered with one reply and nothing else.
async fn answer(request: Request, runs: &Arc<Runs>) -> Reply {
    match request {
        Request::List { id } => match runs.registry().list().await {
            Ok(runs) => Reply::List { id, runs },
            Err(failure) => Reply::Error { id: Some(id), message: failure.to_string() },
        },
        Request::Status { id, run } => match runs.registry().summary(&run).await {
            Ok(Some(mut summary)) => {
                let config = runs.config();
                recording::describe(runs.cache(), runs.storage().as_ref(), &config.temp_dir, &mut summary).await;
                Reply::Status {
                    id,
                    run: summary.run,
                    name: summary.name,
                    created_at: summary.created_at,
                    attempts: summary.attempts,
                    progress: summary.progress,
                }
            }
            Ok(None) => Reply::Error { id: Some(id), message: format!("there is no run `{run}`") },
            Err(failure) => Reply::Error { id: Some(id), message: failure.to_string() },
        },
        Request::Cancel { id, run } => match runs.cancel(&run).await {
            Ok(()) => Reply::Cancelled { id, run },
            Err(failure) => Reply::Error { id: Some(id), message: failure.to_string() },
        },
        Request::Series { id, run, metrics, until } => metric_series(runs, id, run, metrics, until).await,
        // Each is answered where it belongs: a submission by the caller, which
        // holds the run back until the reply has gone out, and an attachment by
        // the task the caller starts for it.
        Request::Submit { id, .. } | Request::Attach { id, .. } | Request::Detach { id } => {
            Reply::Error { id: Some(id), message: "this request is not answered here".to_string() }
        }
    }
}

/// The points each named metric had, read off the run's own recording.
async fn metric_series(
    runs: &Arc<Runs>,
    id: RequestId,
    run: String,
    metrics: Vec<String>,
    until: Option<u64>,
) -> Reply {
    let config = runs.config();
    let refused = |message: String| Reply::Error { id: Some(id.clone()), message };
    let held = match runs.cache().hold(runs.registry(), runs.storage().as_ref(), &config.temp_dir, &run).await {
        Ok(held) => held,
        Err(failure) => return refused(failure.to_string()),
    };
    let most = config.series_points;
    let assembled = tokio::task::spawn_blocking(move || series::of(&held.path, &metrics, until, most)).await;
    match assembled {
        Ok(Ok(metrics)) => Reply::Series { id, run, metrics },
        Ok(Err(failure)) => refused(failure.to_string()),
        Err(joined) => refused(joined.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::request_id;

    #[test]
    fn a_message_of_the_wrong_shape_still_answers_the_request_it_named() {
        assert_eq!(request_id(r#"{"type":"status","id":"7"}"#).as_deref(), Some("7"));
        assert_eq!(request_id("{"), None);
        assert_eq!(request_id(r#"{"type":"status"}"#), None);
    }
}
