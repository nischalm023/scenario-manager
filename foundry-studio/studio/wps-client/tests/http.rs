//! Spec 04 section 3.9's retries, held against a scripted server: a read is tried three times in all on a 502 to
//! 504 and twice on a 500, a 4xx never again, and a changeset is sent once, an unanswered one reported lost, as is one
//! answered by anything but a readable 201 or WPS's own refusal object; and a read's refusal says nothing is there
//! only under the code its route answers for that.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use studio_protocol::{ScopePath, Seq};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;
use wps_client::routes::{RouteShape, Routes};
use wps_client::wire::WireChangeset;
use wps_client::wps::{PostFailure, Wps};
use wps_client::{HttpConfig, HttpWps, WpsError};

/// A server answering each connection with the next of `answers` (status, body), counting the requests.
async fn scripted(answers: Vec<(u16, &'static str)>) -> (HttpWps, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
    let address = listener.local_addr().expect("an address");
    let served = Arc::new(AtomicUsize::new(0));
    let counter = served.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let at = counter.fetch_add(1, Ordering::SeqCst);
            let (status, body) = answers.get(at).copied().unwrap_or((500, "{\"error\":\"script ended\"}"));
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                match socket.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(read) => request.extend_from_slice(&chunk[..read]),
                }
            }
            let answer = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(answer.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
    });
    let routes =
        Routes::new(Url::parse(&format!("http://{address}")).expect("a URL"), RouteShape::Path).expect("routes");
    let mut config = HttpConfig::new(routes);
    config.retry_waits = [Duration::from_millis(5), Duration::from_millis(5)];
    (HttpWps::new(config).expect("a client"), served)
}

const PROTOCOL: &str = "{\"proto_version\":\"0.7.0\"}";
const BUSY: &str = "{\"error\":\"busy\"}";

/// A read meeting two 503s is answered on its third try; a third 502 ends it `Unavailable` after three.
#[tokio::test]
async fn a_read_is_tried_three_times_on_an_unavailable_answer() {
    let (wps, served) = scripted(vec![(503, BUSY), (503, BUSY), (200, PROTOCOL)]).await;
    assert_eq!(wps.protocol().await.expect("the third try answers").proto_version, "0.7.0");
    assert_eq!(served.load(Ordering::SeqCst), 3);
    let (wps, served) = scripted(vec![(502, BUSY), (502, BUSY), (502, BUSY), (200, PROTOCOL)]).await;
    assert!(matches!(wps.protocol().await, Err(WpsError::Unavailable { attempts: 3, .. })));
    assert_eq!(served.load(Ordering::SeqCst), 3);
}

const NO_ROUTE: &str = "{\"error\":\"no such\",\"code\":\"no_route\",\"details\":{}}";

/// A 500 is tried once more, a 404 never.
#[tokio::test]
async fn a_server_fault_is_tried_twice_and_a_refusal_once() {
    let (wps, served) = scripted(vec![(500, BUSY), (500, BUSY), (200, PROTOCOL)]).await;
    assert!(matches!(wps.protocol().await, Err(WpsError::ServerFault { status: 500, .. })));
    assert_eq!(served.load(Ordering::SeqCst), 2);
    let (wps, served) = scripted(vec![(404, NO_ROUTE), (200, PROTOCOL)]).await;
    assert!(matches!(wps.protocol().await, Err(WpsError::NotFound { .. })));
    assert_eq!(served.load(Ordering::SeqCst), 1);
}

/// A component's definition and an entity's history are absent only under the code each route answers for nothing
/// there (`component_undefined`, `entity_unknown`); any other refusal of the same status is an error, which before
/// the codes read as nothing there too.
#[tokio::test]
async fn a_read_is_absent_only_under_its_own_code() {
    let undefined = "{\"error\":\"a\",\"code\":\"component_undefined\",\"details\":{\"component\":\"x\"}}";
    let unknown = "{\"error\":\"b\",\"code\":\"entity_unknown\",\"details\":{}}";
    let no_scope = "{\"error\":\"c\",\"code\":\"unknown_scope\",\"details\":{\"scope\":\"customer\"}}";
    let (wps, _) = scripted(vec![(404, undefined), (404, NO_ROUTE), (404, unknown), (404, no_scope)]).await;
    assert!(matches!(wps.component("x").await, Ok(None)));
    assert!(matches!(wps.component("x").await, Err(WpsError::NotFound { .. })));
    let (scope, id) = ("customer/c".parse().expect("a path"), "1d931662-e00e-4cb8-b431-38d5c08f119d".parse().unwrap());
    assert!(matches!(wps.trail(&scope, &id).await, Ok(None)));
    assert!(matches!(wps.trail(&scope, &id).await, Err(WpsError::NotFound { .. })));
}

/// A changeset is sent once: a 503 is a lost answer for `reconcile` to settle, never a resend.
#[tokio::test]
async fn a_changeset_is_never_resent_blindly() {
    let (wps, served) = scripted(vec![(503, BUSY), (201, "{}")]).await;
    let scope: ScopePath = "customer/c/site/s/map".parse().expect("a path");
    let changeset = WireChangeset::of_one_scope(scope, Seq::new(0), "a", "m", Vec::new());
    assert!(matches!(wps.post_changeset(&changeset).await, Err(PostFailure::Lost { .. })));
    assert_eq!(served.load(Ordering::SeqCst), 1);
}

/// LEAD-DECISIONS entry 54: only WPS's own refusal object says a changeset wrote nothing. A 201 whose answer does not
/// read and a 409 or 404 without the object (a proxy's page, an empty body) leave the outcome unknown, reported lost
/// for a resend of the same body to settle; a 409 with the object is the refusal its code names. Before this, the 201
/// and the bare 404 read as refusals, `Invalid` and `NotFound`, which the API took as definitive.
#[tokio::test]
async fn only_world_proto_servers_refusal_says_nothing_was_written() {
    let scope: ScopePath = "customer/c/site/s/map".parse().expect("a path");
    let changeset = WireChangeset::of_one_scope(scope, Seq::new(0), "a", "m", Vec::new());
    let exists = "{\"error\":\"worded anew\",\"code\":\"scope_exists\",\"details\":{\"scope\":\"customer\"}}";
    let (wps, _) = scripted(vec![(201, "{\"scopes\": 3}"), (409, "<html>busy</html>"), (404, ""), (409, exists)]).await;
    for lost in ["an undecodable 201", "a 409 without the refusal object", "an empty 404"] {
        let answered = wps.post_changeset(&changeset).await;
        assert!(matches!(answered, Err(PostFailure::Lost { .. })), "{lost}: {answered:?}");
    }
    let refused = wps.post_changeset(&changeset).await;
    let scope_exists = WpsError::ScopeExists { scope: "customer".parse().expect("a path"), head: Seq::new(0) };
    assert_eq!(refused.map(|_| ()), Err(PostFailure::Answered(scope_exists)));
}
