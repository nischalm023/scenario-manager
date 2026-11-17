//! What the terminal client reads off a socket that is saying more than one
//! thing: the server greets every socket before anything is asked for, and a
//! socket carrying several requests carries their answers in the order they
//! are ready.
//!
//! The server here is written by hand rather than being `ws::serve`, because
//! what is under test is the ORDER of what arrives, and no real server can be
//! asked to send a greeting, then somebody else's answer, and only then this
//! request's. Nothing here needs a database.

use crucible::client::{ask, max_message_bytes};
use crucible::protocol::{Reply, Request};
use futures::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

const HELLO: &str = r#"{"type":"hello","max_message_bytes":33554432}"#;
const LISTING: &str = r#"{"type":"list","id":"1","runs":[]}"#;

/// A server that says exactly these things, in this order, to the one client
/// that connects, and then holds the socket open until that client hangs up.
/// Reports the address to reach it at.
async fn saying(lines: &'static [&'static str]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
    let address = format!("ws://{}", listener.local_addr().expect("bound"));
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("a client");
        let mut socket = accept_async(stream).await.expect("a websocket");
        for line in lines {
            socket.send(Message::Text((*line).into())).await.expect("sent");
        }
        // Held open, so what the client read was read off a live socket rather
        // than raced against a close.
        while socket.next().await.is_some() {}
    });
    address
}

#[tokio::test]
async fn the_greeting_is_not_the_answer_to_the_request_behind_it() {
    let server = saying(&[HELLO, LISTING]).await;
    let reply = ask(&server, Request::List { id: "1".into() }).await.expect("the list is answered");
    assert!(matches!(reply, Reply::List { .. }), "the greeting answers no request: got {reply:?}");
}

#[tokio::test]
async fn another_requests_answer_is_stepped_over() {
    const ELSEWHERE: &str = r#"{"type":"error","id":"9","message":"there is no run `01NOSUCHRUN`"}"#;
    let server = saying(&[HELLO, ELSEWHERE, LISTING]).await;
    let reply = ask(&server, Request::List { id: "1".into() }).await.expect("the list is answered");
    assert!(matches!(reply, Reply::List { .. }), "a reply carrying another id is not this one's: got {reply:?}");
}

#[tokio::test]
async fn a_refusal_of_this_request_is_the_failure_reported() {
    const REFUSED: &str = r#"{"type":"error","id":"1","message":"there is no run `01NOSUCHRUN`"}"#;
    let server = saying(&[HELLO, REFUSED]).await;
    let failure = ask(&server, Request::List { id: "1".into() }).await.expect_err("the list is refused");
    assert_eq!(failure, "there is no run `01NOSUCHRUN`");
}

#[tokio::test]
async fn the_greeting_says_what_a_submission_is_checked_against() {
    let server = saying(&[HELLO, LISTING]).await;
    assert_eq!(max_message_bytes(&server).await.expect("the server answers"), Some(33_554_432));

    // A server that opens with something else greeted with no number, and the
    // client's own ceiling stands.
    let silent = saying(&[LISTING]).await;
    assert_eq!(max_message_bytes(&silent).await.expect("the server answers"), None);
}
