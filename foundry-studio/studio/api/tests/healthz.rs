//! The two listeners over real sockets: the health probe on the gate-facing one, and the runners' one serving none
//! of the gate's routes. The gate-facing listener's `/runner/` 404 needs an identity and the database, so
//! `tests/identity.rs` holds it.

mod support;

use std::net::SocketAddr;

use studio_api::serve;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Send one bare request to `address` and read the answer to its end.
async fn ask(address: SocketAddr, path: &str, headers: &str) -> (String, String) {
    let mut socket = TcpStream::connect(address).await.expect("the service accepts a connection");
    let request = format!("GET {path} HTTP/1.1\r\nHost: studio\r\n{headers}Connection: close\r\n\r\n");
    socket.write_all(request.as_bytes()).await.expect("the request is sent");
    let mut answer = String::new();
    socket.read_to_string(&mut answer).await.expect("the answer is read to its end");
    let (head, body) = answer.split_once("\r\n\r\n").expect("an answer has a head and a body");
    (head.to_owned(), body.to_owned())
}

/// Spec 01 section 3.6: `GET /healthz` answers `200 ok` with no identity, on the gate-facing listener only; and
/// section 8, statement 15: the runners' listener ignores identity headers, serving none of the gate's routes. The
/// service starts on ports the system picks, with a database these requests never need.
#[tokio::test]
async fn healthz_answers_ok_without_identity_on_the_gate_listener_only() {
    let gate = TcpListener::bind("127.0.0.1:0").await.expect("a port on loopback");
    let runners = TcpListener::bind("127.0.0.1:0").await.expect("another port on loopback");
    let (gate_address, runner_address) = (gate.local_addr().expect("a port"), runners.local_addr().expect("a port"));
    let state = support::gate(support::state(support::unconnected_pool(), support::gate_rules())).await;
    tokio::spawn(serve(gate, runners, state));

    let (head, body) = ask(gate_address, "/healthz", "").await;
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
    assert_eq!(body, "ok");

    let identity =
        "x-auth-request-user: someone\r\nx-auth-request-email: a@b.c\r\nx-auth-request-preferred-username: A\r\n";
    for path in ["/healthz", "/api/me"] {
        let (head, _) = ask(runner_address, path, identity).await;
        assert!(head.starts_with("HTTP/1.1 404 Not Found\r\n"), "{path} on the runners' listener: {head}");
    }
}
