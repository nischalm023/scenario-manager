//! Within `studio-api`, `problem` is how the service answers a refusal: `studio-protocol`'s `Problem` sent as
//! `application/problem+json` under its own status (spec 01 section 3.14), and the answers every route shares:
//! `not-found` for a method and path no route serves, `validation-failed` for a body or query that does not read,
//! `internal` for a fault on the service's side, and a world-proto-server failure as spec 04 section 3.8 maps it.

use std::fmt::Display;
use std::time::Duration;

use axum::http::header::{CONTENT_TYPE, RETRY_AFTER};
use axum::http::{HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde_json::Value;
use studio_protocol::{Key, MovedScope, Problem, ProblemItem, ProblemType, Severity};
use wps_client::{Problem as WpsProblem, WpsError};

pub const PROBLEM_JSON: &str = "application/problem+json";

/// A problem as an HTTP answer, boxed since its extensions make it large beside the answers routes return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProblemAnswer(pub Box<Problem>);

impl From<Problem> for ProblemAnswer {
    fn from(problem: Problem) -> Self {
        Self(Box::new(problem))
    }
}

/// The body under its status, with `Retry-After` whenever the body says how long to wait.
impl IntoResponse for ProblemAnswer {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.0.status).expect("every problem type's status is an HTTP status");
        let body = serde_json::to_vec(&self.0).expect("a problem is strings and numbers, which always serialize");
        let mut answer = (status, [(CONTENT_TYPE, HeaderValue::from_static(PROBLEM_JSON))], body).into_response();
        if let Some(seconds) = self.0.retry_after_s {
            answer.headers_mut().insert(RETRY_AFTER, HeaderValue::from(seconds));
        }
        answer
    }
}

/// The fallback of both listeners: no route serves this method and path. A known path asked with another method is
/// answered the same way, since spec 01 section 3.14's vocabulary has no type of its own for it.
pub async fn not_found(method: Method, uri: Uri) -> ProblemAnswer {
    let detail = format!("Agent Studio serves no route {method} {}.", uri.path());
    Problem::new(ProblemType::NotFound, detail).at(uri.path()).into()
}

/// A request whose body or query fails a check, `why` saying which and how.
pub fn validation_failed(why: impl Display, instance: &str) -> ProblemAnswer {
    Problem::new(ProblemType::ValidationFailed, format!("The request does not read: {why}")).at(instance).into()
}

/// A JSON body read as `T`; an empty body reads as `{}`, so a POST that needs no fields needs no body.
pub fn json_body<T: DeserializeOwned>(body: &[u8], instance: &str) -> Result<T, ProblemAnswer> {
    let body = if body.iter().all(u8::is_ascii_whitespace) { b"{}".as_slice() } else { body };
    serde_json::from_slice(body).map_err(|fault| validation_failed(fault, instance))
}

/// A fault on the service's side while `doing` something for the request at `instance`: logged in full, and
/// answered as `internal` without its internals.
pub fn internal(doing: &str, fault: impl Display, instance: &str) -> ProblemAnswer {
    eprintln!("studio-api: failed to {doing} for {instance}: {fault}");
    let detail = format!("The service failed to {doing}. Try again; if it keeps failing, the fault is in its log.");
    Problem::new(ProblemType::Internal, detail).at(instance).into()
}

/// A world-proto-server failure, or the client's own finding, as the person meets it (spec 04 section 3.8's table
/// into spec 01 section 3.14): the client's problem type, status and detail, and every extension it fills, with the
/// wait on `compute-busy`. A fault in the client itself answers `internal`, logged in full.
pub fn wps_problem(error: &WpsError, instance: &str) -> ProblemAnswer {
    let answer = error.api_problem();
    let Ok(type_) = serde_json::from_value::<ProblemType>(Value::from(answer.type_)) else {
        return internal("read world-proto-server's failure", error, instance);
    };
    if type_ == ProblemType::Internal {
        eprintln!("studio-api: the world-proto-server client failed for {instance}: {error}");
    }
    let mut problem = Problem::new(type_, answer.detail).at(instance);
    problem.status = answer.status;
    problem.problems = answer.problems.iter().map(problem_item).collect();
    problem.moved = answer
        .moved
        .into_iter()
        .map(|moved| MovedScope { scope: moved.scope, base: moved.base, head: moved.head })
        .collect();
    problem.uncertain = answer.uncertain;
    if let WpsError::ComputeBusy { retry_after } = error {
        problem.retry_after_s = Some(whole_seconds(*retry_after));
    }
    problem.into()
}

/// A refusal of WPS's or a finding of the client's check: each stops the write it is found on, so each is an error.
fn problem_item(problem: &WpsProblem) -> ProblemItem {
    ProblemItem {
        code: problem.kind.to_string(),
        severity: Severity::Error,
        ref_: problem.entity.clone().map(Key::Entity),
        scope: problem.scope.clone(),
        field: problem.field(),
        knob: None,
        text: problem.text.clone(),
    }
}

/// A wall-clock wait as `Retry-After` writes it: whole seconds, rounded up, at least one.
fn whole_seconds(wait: Duration) -> u32 {
    let seconds = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
    u32::try_from(seconds.max(1)).unwrap_or(u32::MAX)
}
