//! Within `simrunner`, `door` is the runners' listener as one attempt reaches it, by its address and the token that
//! opens its routes: the run's inputs read through it, each try given at most five minutes and asked again three times
//! about half a minute apart while the listener does not answer or fails on its side; and a delivery once the run is
//! over asked again for four minutes in all, its requests and their answers included; a refusal is final. What the
//! runner says over it besides is its children's: the attempt's control read every 250 ms (`control`), the heartbeat
//! every 5 s (`heartbeat`), and the recording and report once the run is over (`report`), each on limits of its own.

pub mod control;
pub mod heartbeat;
pub mod report;

use std::error::Error;
use std::iter::once;
use std::path::Path;
use std::time::Duration;

use reqwest::{Client as Http, Method, RequestBuilder};
use serde::de::DeserializeOwned;
use serde_json::Value;
use studio_protocol::DELIVERY_BUDGET;
use tokio::fs::File;
use tokio::time::{Instant, sleep};

use crate::attempt::Stop;

/// How long a source of the run's inputs has to answer, three tries apart (wall clock), before the attempt fails
/// naming it: about half a minute.
pub(crate) const RETRIES: [Duration; 3] = [Duration::from_secs(1), Duration::from_secs(5), Duration::from_secs(25)];

/// How long one try of an input may take, its answer read whole: the workload's stream is the longest of them.
const INPUT_TRY_LIMIT: Duration = Duration::from_secs(300);

/// How a delivery once the run is over, its recording or its report, is asked again while the listener does not
/// answer or fails on its side (wall clock): up to nine tries, the waits between them from 1 s doubling to a minute,
/// long enough to ride out a rolling restart of the API and well inside a Job's deadline. All of it, the waits, the
/// requests and their answers, falls within the protocol's `DELIVERY_BUDGET`, each try given at most
/// `DELIVERY_TRY_LIMIT` of what is left, in which the largest recording goes up. Past it the runner exits non-zero,
/// what it left still in its folder.
const DELIVERY_RETRIES: [Duration; 8] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
    Duration::from_secs(32),
    Duration::from_secs(60),
    Duration::from_secs(60),
];
const DELIVERY_TRY_LIMIT: Duration = Duration::from_secs(120);

/// How one request is asked again: the waits between its tries, how long one try may take, and how long all of them
/// may take together.
struct Tries {
    waits: &'static [Duration],
    each: Duration,
    within: Option<Duration>,
}

const INPUTS: Tries = Tries { waits: &RETRIES, each: INPUT_TRY_LIMIT, within: None };
const DELIVERIES: Tries = Tries { waits: &DELIVERY_RETRIES, each: DELIVERY_TRY_LIMIT, within: Some(DELIVERY_BUDGET) };

/// The runners' listener as one attempt reaches it: its address and the token that opens its routes.
#[derive(Clone)]
pub struct Door {
    http: Http,
    endpoint: String,
    token: String,
}

/// What a request to the door carries: nothing, JSON, or a file, read afresh for each try as it goes.
pub enum Carried<'a> {
    Nothing,
    Json(&'a [u8]),
    File(&'a Path),
}

impl Door {
    pub fn new(endpoint: &str, token: &str) -> Self {
        Self { http: Http::new(), endpoint: endpoint.trim_end_matches('/').to_owned(), token: token.to_owned() }
    }

    /// A request to `path` on the listener with the attempt's token, asked once, on whatever limit its caller sets.
    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.http.request(method, format!("{}{path}", self.endpoint)).bearer_auth(&self.token)
    }

    pub async fn json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<T, Stop> {
        let bytes = self.bytes(method, path, body).await?;
        read_answer(path, &bytes)
    }

    /// One request for the run's inputs, on the inputs' tries.
    pub async fn bytes(&self, method: Method, path: &str, body: Option<Vec<u8>>) -> Result<Vec<u8>, Stop> {
        let carried = body.as_deref().map_or(Carried::Nothing, Carried::Json);
        self.asked(method, path, &carried, &INPUTS).await
    }

    /// One delivery once the run is over, `PUT` on the deliveries' tries.
    pub async fn deliver<T: DeserializeOwned>(&self, path: &str, carried: Carried<'_>) -> Result<T, Stop> {
        let bytes = self.asked(Method::PUT, path, &carried, &DELIVERIES).await?;
        read_answer(path, &bytes)
    }

    /// One request, made afresh for each try and asked again on `tries` while the listener does not answer, fails on
    /// its side, or cuts its answer off; a refusal is final.
    async fn asked(&self, method: Method, path: &str, carried: &Carried<'_>, tries: &Tries) -> Result<Vec<u8>, Stop> {
        let started = Instant::now();
        let left = || tries.within.map_or(Duration::MAX, |within| within.saturating_sub(started.elapsed()));
        let (mut last, mut tried) = (String::new(), 0);
        for wait in once(&Duration::ZERO).chain(tries.waits) {
            if *wait >= left() {
                break;
            }
            sleep(*wait).await;
            tried += 1;
            let request = self.request(method.clone(), path).timeout(tries.each.min(left()));
            let request = match carried {
                Carried::Nothing => request,
                Carried::Json(body) => request.header("content-type", "application/json").body(body.to_vec()),
                Carried::File(path) => {
                    let file = File::open(path).await.map_err(|fault| {
                        Stop::Platform(format!("{} could not be read to send: {fault}", path.display()))
                    })?;
                    request.header("content-type", "application/octet-stream").body(file)
                }
            };
            match request.send().await {
                Ok(answer) if answer.status().is_success() => match answer.bytes().await {
                    Ok(bytes) => return Ok(bytes.to_vec()),
                    Err(fault) => last = format!("its answer was cut off: {}", chain(&fault)),
                },
                Ok(answer) if answer.status().is_server_error() => last = format!("it answered {}", answer.status()),
                Ok(answer) => {
                    let status = answer.status();
                    let text = answer.text().await.unwrap_or_default();
                    let detail = serde_json::from_str::<Value>(&text)
                        .ok()
                        .and_then(|problem| problem.get("detail").and_then(Value::as_str).map(str::to_owned))
                        .unwrap_or(text);
                    return Err(Stop::Platform(format!("Agent Studio refused {path} ({status}): {detail}")));
                }
                Err(fault) => last = chain(&fault),
            }
        }
        let within = tries.within.map_or_else(String::new, |within| format!(" within {} s", within.as_secs()));
        Err(Stop::Platform(format!(
            "Agent Studio's runners' listener did not answer {path} in {tried} tries{within}: {last}"
        )))
    }
}

fn read_answer<T: DeserializeOwned>(path: &str, bytes: &[u8]) -> Result<T, Stop> {
    serde_json::from_slice(bytes)
        .map_err(|fault| Stop::Platform(format!("{path} answered what does not read: {fault}")))
}

/// A fault with every cause behind it, since a client's top line ("error sending request") says nothing of why.
pub fn chain(fault: &dyn Error) -> String {
    let mut words = fault.to_string();
    let mut cause = fault.source();
    while let Some(next) = cause {
        words.push_str(": ");
        words.push_str(&next.to_string());
        cause = next.source();
    }
    words
}
