//! Spec 01 section 3.2, identity: who the gate's headers name, who is a developer, and the identity layer every
//! request to the gate-facing listener but `GET /healthz` passes; and `GET /api/me` (spec 01 section 3.6).

mod support;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode};
use serde_json::{Value, json};
use studio_api::gate_routes;
use studio_api::identity::{DevIdentity, MissingIdentity};
use studio_protocol::Subject;

/// A route of the runners' listener (spec 01 section 3.16), which the gate-facing listener never serves.
const RUNNER_PATH: &str = "/runner/v1/attempts/01KN0TJZANP5N0PGJD8F7A3FDB/1";

fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.append(*name, HeaderValue::from_str(value).expect("a header value"));
    }
    headers
}

fn problem_type(body: &[u8]) -> String {
    let problem: Value = serde_json::from_slice(body).expect("a problem body");
    problem["type"].as_str().expect("a problem names its type").to_owned()
}

fn person(subject: &str, roles: &str) -> HeaderMap {
    let (email, name) = (format!("{subject}@example.com"), format!("Person {subject}"));
    headers(&[
        ("x-auth-request-user", subject),
        ("x-auth-request-email", &email),
        ("x-auth-request-preferred-username", &name),
        ("x-auth-request-groups", roles),
    ])
}

/// Spec 01 section 3.2: the gate's headers name the person, and `developer` is true when the roles header names
/// `STUDIO_DEVELOPER_ROLE` itself.
#[test]
fn the_gates_headers_name_the_person_and_the_developer_role_makes_a_developer() {
    let rules = support::gate_rules();
    let developer = rules.identify(&person("u-1", "readers, studio-developer")).expect("the headers name someone");
    assert_eq!(developer.subject.as_str(), "u-1");
    assert_eq!((developer.email.as_str(), developer.name.as_str()), ("u-1@example.com", "Person u-1"));
    assert!(developer.developer);

    let reader = rules.identify(&person("u-2", "readers,studio-developers")).expect("the headers name someone");
    assert!(!reader.developer, "only the role itself makes a developer");
    assert!(!rules.identify(&person("u-3", " ")).expect("the roles header may be blank").developer);
    let mut no_roles = person("u-3", "");
    no_roles.remove("x-auth-request-groups");
    assert!(!rules.identify(&no_roles).expect("the roles header may be absent").developer);
}

/// Spec 01 section 3.2: a request without the subject header names nobody; nor, here, without the email or the name,
/// or with any of them twice, so a client's copy never stands beside the gate's.
#[test]
fn a_request_without_its_subject_email_or_name_names_nobody() {
    let rules = support::gate_rules();
    for header in ["x-auth-request-user", "x-auth-request-email", "x-auth-request-preferred-username"] {
        let mut lacking = person("u-1", "");
        lacking.remove(header);
        let MissingIdentity { header: missing, why } = rules.identify(&lacking).expect_err("it names nobody");
        assert_eq!((missing.as_str(), why), (header, "is absent"));
    }
    let blank = rules.identify(&headers(&[("x-auth-request-user", " ")])).expect_err("a blank subject names nobody");
    assert_eq!(blank.why, "is blank");
    let mut twice = person("u-1", "");
    twice.append("x-auth-request-user", HeaderValue::from_static("u-2"));
    let twice = rules.identify(&twice).expect_err("a subject given twice names nobody");
    assert_eq!(twice.why, "appears more than once");
}

/// Spec 01 section 3.2: with `local`, a subject `STUDIO_DEVELOPERS` lists is a developer, and the development
/// identity stands in for the gate.
#[test]
fn locally_a_developer_list_or_a_development_identity_stands_in_for_the_gate() {
    let mut rules = support::gate_rules();
    rules.developers.insert("u-4".parse::<Subject>().expect("a subject"));
    assert!(rules.identify(&person("u-4", "")).expect("the headers name someone").developer);

    rules.dev_identity = Some("dev-1, dev@example.com, Dev One".parse::<DevIdentity>().expect("subject,email,name"));
    let dev = rules.identify(&HeaderMap::new()).expect("the development identity needs no headers");
    assert_eq!((dev.subject.as_str(), dev.email.as_str(), dev.name.as_str()), ("dev-1", "dev@example.com", "Dev One"));
    assert!(!dev.developer, "the development identity is a developer only when the list names it");
}

/// Spec 01 section 8, statement 10's second half: a request without the identity headers gets 401
/// `identity-missing`, before the database is touched.
#[tokio::test]
async fn me_without_identity_is_refused_before_the_database_is_touched() {
    let router = gate_routes(support::gate(support::state(support::unconnected_pool(), support::gate_rules())).await);
    let request = Request::get("/api/me").body(Body::empty()).expect("a request");
    let (status, headers, body) = support::call(&router, request).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(headers["content-type"], "application/problem+json");
    let problem: Value = serde_json::from_slice(&body).expect("a problem body");
    assert_eq!((problem["type"].as_str(), problem["status"].as_u64()), (Some("identity-missing"), Some(401)));
    assert_eq!(problem["instance"], "/api/me");
    assert!(problem["detail"].as_str().is_some_and(|detail| detail.contains("x-auth-request-user")), "{problem}");
}

/// Spec 01 section 3.6: `GET /api/me` answers who is asking; section 3.2: the person's row is upserted, the gate's
/// latest word on the name winning.
#[tokio::test]
async fn me_answers_from_the_gates_headers_and_records_the_person() {
    let scratch = scratch_or_skip!();
    let router = gate_routes(support::gate(support::state(scratch.pool.clone(), support::gate_rules())).await);

    let request =
        support::from_gate("GET", "/api/me", "u-1", "studio-developer").body(Body::empty()).expect("a request");
    let (status, _, body) = support::call(&router, request).await;
    assert_eq!(status, StatusCode::OK);
    let me: Value = serde_json::from_slice(&body).expect("a JSON answer");
    let expected = json!({ "subject": "u-1", "email": "u-1@example.com", "name": "Person u-1", "developer": true,
                           "runner_image": null });
    assert_eq!(me, expected);

    let mut renamed = support::from_gate("GET", "/api/me", "u-1", "").body(Body::empty()).expect("a request");
    renamed.headers_mut().insert("x-auth-request-preferred-username", HeaderValue::from_static("Renamed"));
    let (status, _, _) = support::call(&router, renamed).await;
    assert_eq!(status, StatusCode::OK);
    let (name, first_seen_first): (String, bool) =
        sqlx::query_as("SELECT name, first_seen_at <= last_seen_at FROM people WHERE subject = 'u-1'")
            .fetch_one(&scratch.pool)
            .await
            .expect("the person was recorded");
    assert_eq!(name, "Renamed", "the gate's latest word on the name wins");
    assert!(first_seen_first);
    scratch.close().await;
}

/// Spec 01 section 3.2: the gate-facing listener reads identity on every request but `GET /healthz`, so a path no
/// route serves, a known path asked with another method, `POST /healthz` and a `/runner/` path are each refused
/// without it, and `GET /healthz` alone answers.
#[tokio::test]
async fn every_request_but_the_health_probe_is_refused_without_identity() {
    let router = gate_routes(support::gate(support::state(support::unconnected_pool(), support::gate_rules())).await);
    let refused = [("GET", "/api/nowhere"), ("POST", "/api/me"), ("POST", "/healthz"), ("GET", RUNNER_PATH)];
    for (method, path) in refused {
        let request = Request::builder().method(method).uri(path).body(Body::empty()).expect("a request");
        let (status, _, body) = support::call(&router, request).await;
        let refusal = (status, problem_type(&body));
        assert_eq!(refusal, (StatusCode::UNAUTHORIZED, "identity-missing".to_owned()), "{method} {path}");
    }
    let (status, _, body) =
        support::call(&router, Request::get("/healthz").body(Body::empty()).expect("a request")).await;
    assert_eq!((status, body.as_slice()), (StatusCode::OK, b"ok".as_slice()));
}

/// Spec 01 section 3.2: every request upserts its person, one no route serves included; and section 8, statement 15:
/// the gate-facing listener answers 404 on a `/runner/` route.
#[tokio::test]
async fn a_request_no_route_serves_records_its_person_and_answers_not_found() {
    let scratch = scratch_or_skip!();
    let router = gate_routes(support::gate(support::state(scratch.pool.clone(), support::gate_rules())).await);
    for (method, path, subject) in
        [("GET", "/api/nowhere", "u-1"), ("POST", "/api/me", "u-2"), ("GET", RUNNER_PATH, "u-3")]
    {
        let request = support::from_gate(method, path, subject, "").body(Body::empty()).expect("a request");
        let (status, headers, body) = support::call(&router, request).await;
        assert_eq!((status, problem_type(&body)), (StatusCode::NOT_FOUND, "not-found".to_owned()), "{method} {path}");
        assert_eq!(headers["content-type"], "application/problem+json");
    }
    let recorded: Vec<String> = sqlx::query_scalar("SELECT subject FROM people ORDER BY subject")
        .fetch_all(&scratch.pool)
        .await
        .expect("the people read");
    assert_eq!(recorded, ["u-1", "u-2", "u-3"]);
    scratch.close().await;
}
