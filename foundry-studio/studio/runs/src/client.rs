//! Within `crucible`, `client` is THE ASKING SIDE of what [`ws`](crate::ws)
//! answers: it opens a socket, sends one request, and hands back the reply that
//! answers THAT request.
//!
//! It is a module of the server's own crate rather than private code inside the
//! terminal client, so that a test can drive it against a socket that says what
//! it likes. Two of the things a real socket says are the reason this module
//! exists: the server greets every socket with what it accepts in one message
//! before anything is asked for, and a socket carrying several requests carries
//! their answers in whatever order they are ready. A client that read the first
//! message as its answer would read the greeting instead.
//!
//! Attaching is not here. It reads binary frames and decodes them, which is the
//! terminal client's own work; this module is the one-request-one-reply half.

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use crate::protocol::{Reply, Request};

/// How long a socket waits for a greeting before taking the server for one that
/// does not greet. Nothing has been asked for on that socket, so there is
/// nothing else the wait could be spent on.
const GREETING_WAIT: Duration = Duration::from_secs(5);

/// A socket to a Crucible server.
pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Open a socket to a server, or say which server would not answer.
pub async fn connect(server: &str) -> Result<Socket, String> {
    let (socket, _) = connect_async(server).await.map_err(|failure| format!("cannot reach {server}: {failure}"))?;
    Ok(socket)
}

/// One request over one socket, and the reply that answers it. Every command
/// opens its own socket, so hanging up is never a cancel.
pub async fn ask(server: &str, request: Request) -> Result<Reply, String> {
    let mut socket = connect(server).await?;
    let text = serde_json::to_string(&request).map_err(|failure| failure.to_string())?;
    socket.send(Message::Text(text.into())).await.map_err(|failure| failure.to_string())?;
    while let Some(message) = socket.next().await {
        match message.map_err(|failure| failure.to_string())? {
            Message::Text(text) => {
                let reply: Reply = serde_json::from_str(&text).map_err(|failure| format!("{failure}: {text}"))?;
                // The greeting answers no request, and a reply carrying another
                // id answers somebody else's. Neither is this one's answer.
                let elsewhere = reply.answers().is_some_and(|answered| answered != request.id());
                if elsewhere || matches!(reply, Reply::Hello { .. }) {
                    continue;
                }
                // A refusal that names no request at all is still about the one
                // request this socket sent.
                return match reply {
                    Reply::Error { message, .. } => Err(message),
                    answer => Ok(answer),
                };
            }
            Message::Close(_) => break,
            _ => continue,
        }
    }
    Err("the server closed the socket without answering".to_string())
}

/// What the server says it accepts in one message, heard on a socket of its own
/// before a submission is read and encoded against it.
///
/// `None` is a server that greeted with nothing, and the caller's own ceiling
/// stands. Nothing is asked for on this socket, so the greeting is the only
/// thing a server sends on it: a socket that stays silent for the wait, or
/// opens with something else, is one that does not say what it takes.
pub async fn max_message_bytes(server: &str) -> Result<Option<usize>, String> {
    let mut socket = connect(server).await?;
    let heard = tokio::time::timeout(GREETING_WAIT, socket.next()).await;
    let _ = socket.close(None).await;
    let Ok(Some(Ok(Message::Text(text)))) = heard else { return Ok(None) };
    match serde_json::from_str(&text) {
        Ok(Reply::Hello { max_message_bytes }) => Ok(usize::try_from(max_message_bytes).ok()),
        _ => Ok(None),
    }
}
