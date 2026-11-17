//! The door's tries for a delivery once the run is over (`door`): an answer cut off after its status is a try that
//! failed, asked again; and a listener that takes the connection and never answers is given up on within the delivery's
//! four minutes of wall clock, its requests included, rather than ten requests of two minutes each.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use simrunner::door::{Carried, Door};
use studio_protocol::{ResultsAccepted, ResultsEnded};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::Instant;

const PATH: &str = "/runner/v1/attempts/01KZAVPQNES65HV4EZADPCS7E8/1/results";

/// The request on `stream`, read whole: its head and the body its length names.
async fn read_request(stream: &mut TcpStream) {
    let mut read = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let got = stream.read(&mut chunk).await.expect("the request");
        assert!(got > 0, "the request ended early");
        read.extend_from_slice(&chunk[..got]);
        let Some(head) = read.windows(4).position(|window| window == b"\r\n\r\n") else { continue };
        let head_text = String::from_utf8_lossy(&read[..head]).to_ascii_lowercase();
        let length = head_text
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .map_or(0, |length| length.trim().parse::<usize>().expect("a length"));
        if read.len() >= head + 4 + length {
            return;
        }
    }
}

/// A listener that answers its connections with `answers` in turn, each after reading the request whole and then
/// closing; the connections it took.
async fn scripted(answers: Vec<&'static [u8]>) -> (String, Arc<AtomicUsize>) {
    let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", socket.local_addr().unwrap());
    let taken = Arc::new(AtomicUsize::new(0));
    let counted = taken.clone();
    tokio::spawn(async move {
        for answer in answers {
            let (mut stream, _) = socket.accept().await.unwrap();
            counted.fetch_add(1, Ordering::SeqCst);
            read_request(&mut stream).await;
            stream.write_all(answer).await.unwrap();
            stream.shutdown().await.unwrap();
        }
    });
    (endpoint, taken)
}

#[tokio::test]
async fn an_answer_cut_off_after_its_status_is_asked_again() {
    let cut = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 64\r\n\r\n{\"ended\"";
    let whole = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 17\r\n\r\n{\"ended\":\"wrote\"}";
    let (endpoint, taken) = scripted(vec![cut, whole]).await;
    let door = Door::new(&endpoint, "door-test-token");
    let accepted: ResultsAccepted =
        door.deliver(PATH, Carried::Json(b"{}")).await.expect("delivered on its second try");
    assert_eq!((accepted.ended, taken.load(Ordering::SeqCst)), (ResultsEnded::Wrote, 2));
}

#[tokio::test(start_paused = true)]
async fn a_listener_that_never_answers_is_given_up_on_within_four_minutes() {
    let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", socket.local_addr().unwrap());
    tokio::spawn(async move {
        let mut held = Vec::new();
        loop {
            let (stream, _) = socket.accept().await.unwrap();
            held.push(stream);
        }
    });
    let door = Door::new(&endpoint, "door-test-token");
    let started = Instant::now();
    let given_up = door.deliver::<ResultsAccepted>(PATH, Carried::Json(b"{}")).await.expect_err("never answered");
    let waited = started.elapsed();
    assert!(waited <= Duration::from_secs(241), "gave up after {waited:?}, past the delivery's four minutes");
    assert!(given_up.to_string().contains("within 240 s"), "{given_up}");
}
