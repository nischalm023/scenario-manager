//! Within `door`, `control` is the attempt row's cancellation as the runner reads it, every 250 ms of wall clock, which
//! it passes into the runner's stop (`stop`). A lost listener is retried; a refused control route is the platform's
//! stop, as SIGTERM is (`attempt`), so neither a drain nor a termination can become a person's cancellation.

use std::sync::Arc;
use std::time::Duration;

use reqwest::{Method, StatusCode};
use studio_protocol::{AttemptControl, JobSpec};
use tokio::task::JoinHandle;
use tokio::time::sleep;

use super::{Door, chain};
use crate::stop::RunnerStop;

const POLL: Duration = Duration::from_millis(250);
const REQUEST_LIMIT: Duration = Duration::from_secs(2);

/// One attempt's control reader. Dropping it aborts the reader on every exit from preparation or execution.
pub(crate) struct Control(JoinHandle<()>);

impl Control {
    /// Observe the first bounded response before returning, so even a short run sees an immediate stop or refusal.
    pub(crate) async fn start(door: &Door, spec: &JobSpec, stop: Arc<RunnerStop>) -> Self {
        let (door, path) = (door.clone(), format!("/runner/v1/attempts/{}/{}/control", spec.run, spec.attempt.0));
        let mut unavailable = false;
        let mut again = poll(&door, &path, &stop, &mut unavailable).await;
        Self(tokio::spawn(async move {
            while again {
                sleep(POLL).await;
                again = poll(&door, &path, &stop, &mut unavailable).await;
            }
        }))
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        self.0.abort();
    }
}

enum Fault {
    Unavailable(String),
    Refused(String),
}

async fn poll(door: &Door, path: &str, stop: &RunnerStop, unavailable: &mut bool) -> bool {
    match read(door, path).await {
        Ok(control) => {
            *unavailable = false;
            if control.cancel_requested {
                stop.cancel();
                // Said only once the stop holds the cancel, so a SIGTERM after these words cannot be the first stop.
                eprintln!("simrunner: the attempt's control says a person cancelled it");
                return false;
            }
        }
        Err(Fault::Refused(words)) => {
            stop.platform_failure(words);
            return false;
        }
        Err(Fault::Unavailable(words)) => {
            if !*unavailable {
                eprintln!("simrunner: {words}; retrying attempt control");
                *unavailable = true;
            }
        }
    }
    true
}

async fn read(door: &Door, path: &str) -> Result<AttemptControl, Fault> {
    let answer = door.request(Method::GET, path).timeout(REQUEST_LIMIT).send().await.map_err(|fault| {
        Fault::Unavailable(format!("the attempt control listener did not answer: {}", chain(&fault)))
    })?;
    let status = answer.status();
    if status.is_server_error() || matches!(status, StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS) {
        return Err(Fault::Unavailable(format!("the attempt control listener answered {status}")));
    }
    if !status.is_success() {
        return Err(Fault::Refused(format!("Agent Studio refused the attempt's control route ({status})")));
    }
    let bytes = answer
        .bytes()
        .await
        .map_err(|fault| Fault::Unavailable(format!("the attempt control response ended: {fault}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|fault| Fault::Refused(format!("the attempt control response does not read: {fault}")))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering};

    use axum::Router;
    use axum::extract::State;
    use axum::http::{HeaderMap, Uri};
    use axum::routing::get;
    use dessim::harness::{RunOverrides, prepare};
    use dessim::sim_types::Counters;
    use desunits::SimTime;
    use studio_protocol::{AttemptNumber, EndedBy, Ending, Endpoint, FORMAT, RunState};
    use tempfile::tempdir;
    use tokio::net::TcpListener;
    use tokio::sync::Notify;
    use tokio::time::timeout;

    use super::*;
    use crate::simulate::simulate;

    #[derive(Default)]
    struct Replies {
        status: AtomicU16,
        cancel: AtomicBool,
        reads: AtomicUsize,
        seen: Notify,
    }

    async fn response(State(replies): State<Arc<Replies>>, headers: HeaderMap, uri: Uri) -> (StatusCode, String) {
        assert_eq!(headers["authorization"], "Bearer control-test-token");
        assert!(uri.path().ends_with("/1/control"));
        let status = StatusCode::from_u16(replies.status.load(Ordering::SeqCst)).unwrap();
        let body = format!("{{\"cancel_requested\":{}}}", replies.cancel.load(Ordering::SeqCst));
        replies.reads.fetch_add(1, Ordering::SeqCst);
        replies.seen.notify_one();
        (status, body)
    }

    /// The door a test's runner reaches the listener `spec` names through, with the tests' token.
    fn door_of(spec: &JobSpec) -> Door {
        Door::new(&spec.studio.endpoint, "control-test-token")
    }

    async fn served() -> (JobSpec, Arc<Replies>, JoinHandle<()>) {
        let replies = Arc::new(Replies::default());
        replies.status.store(200, Ordering::SeqCst);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let spec = JobSpec {
            v: FORMAT,
            run: "01KZAVPQNES65HV4EZADPCS7E8".parse().unwrap(),
            attempt: AttemptNumber(1),
            studio: Endpoint { endpoint: format!("http://{}", listener.local_addr().unwrap()) },
            streaming: None,
        };
        let router =
            Router::new().route("/runner/v1/attempts/{run}/{n}/control", get(response)).with_state(replies.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (spec, replies, server)
    }

    async fn reads(replies: &Replies, n: usize) {
        timeout(Duration::from_secs(5), async {
            while replies.reads.load(Ordering::SeqCst) < n {
                replies.seen.notified().await;
            }
        })
        .await
        .expect("control keeps being read");
    }

    fn run_stopped(spec: &JobSpec, stop: Arc<RunnerStop>) -> Ending {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/canned/spec.json");
        let overrides = RunOverrides { num_threads: Some(1), ..RunOverrides::default() };
        let prepared = prepare(&fixture, &overrides).unwrap();
        let out = tempdir().unwrap();
        simulate(prepared, out.path(), &spec.run, spec.attempt, stop, Counters::new(), None).unwrap().ending
    }

    #[tokio::test]
    async fn an_unavailable_listener_recovers_and_only_a_row_request_cancels() {
        let (spec, replies, server) = served().await;
        replies.status.store(503, Ordering::SeqCst);
        let stop = RunnerStop::new();
        let _control = Control::start(&door_of(&spec), &spec, stop.clone()).await;
        reads(&replies, 2).await;
        assert!(!stop.signal.is_stopped(), "a listener outage is not a stop");
        replies.status.store(200, Ordering::SeqCst);
        reads(&replies, 4).await;
        assert!(!stop.signal.is_stopped(), "a false row request is not a stop");
        replies.cancel.store(true, Ordering::SeqCst);
        timeout(Duration::from_secs(5), stop.requested()).await.unwrap();
        let ending = run_stopped(&spec, stop);
        assert_eq!((ending.state, ending.ended_by), (RunState::Cancelled, EndedBy::Person));
        server.abort();
    }

    #[tokio::test]
    async fn lost_authority_or_a_missing_control_route_ends_by_the_platform() {
        for status in [401, 403, 404] {
            let (spec, replies, server) = served().await;
            let stop = RunnerStop::new();
            let _control = Control::start(&door_of(&spec), &spec, stop.clone()).await;
            reads(&replies, 2).await;
            assert!(!stop.signal.is_stopped());
            replies.status.store(status, Ordering::SeqCst);
            timeout(Duration::from_secs(5), stop.requested()).await.unwrap();
            assert!(!replies.cancel.load(Ordering::SeqCst));
            let ending = run_stopped(&spec, stop);
            assert_eq!((ending.state, ending.ended_by), (RunState::Failed, EndedBy::Platform), "{ending:?}");
            assert!(ending.stop_reason.contains(&status.to_string()), "{ending:?}");
            server.abort();
        }
    }

    #[tokio::test]
    async fn the_first_response_stops_a_short_run_before_polling_can_race_it() {
        for (status, cancel, state, ended_by) in
            [(404, false, RunState::Failed, EndedBy::Platform), (200, true, RunState::Cancelled, EndedBy::Person)]
        {
            let (spec, replies, server) = served().await;
            replies.status.store(status, Ordering::SeqCst);
            replies.cancel.store(cancel, Ordering::SeqCst);
            let stop = RunnerStop::new();
            let _control = Control::start(&door_of(&spec), &spec, stop.clone()).await;
            // No yield to the poller before execution: the first read must have latched the stop already.
            assert_eq!(replies.reads.load(Ordering::SeqCst), 1);
            assert!(stop.signal.is_stopped());
            let ending = run_stopped(&spec, stop);
            assert_eq!((ending.state, ending.ended_by), (state, ended_by), "{ending:?}");
            assert_eq!(ending.sim_ms, SimTime::ZERO);
            server.abort();
        }
    }
}
