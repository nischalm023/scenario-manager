//! How a template evaluation ends, through the API's routes, as Astra's review of the evaluation-latency chore asked
//! (design/reviews-impl/astra-eval.md, with the lead's dispositions of 2026-10-20): a floor that fails ends its
//! evaluation at once though the client check beside it waits on world-proto-server, and the check is given up; and
//! each evaluation, whichever way it ends, logs one timing line in wall-clock milliseconds saying the outcome its row
//! was committed with, never what it computed.

mod support;

use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use serde_json::{Value, json};
use tokio::time::timeout;
use tracing::Level;
use tracing::dispatcher::DefaultGuard;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::FmtSpan;
use wps_client::{Compute, ComputeBusy};

use support::templates::{draft, ready, ready_on, send, settled, until};

/// The timing line's words before its fields.
const TIMED: &str = "template evaluation timed in wall-clock milliseconds ";

/// Astra's review, its second finding (a failed floor waited for the speculative client check): with the check's
/// reads of world-proto-server held unanswered, a change whose site cannot be drawn ends failed at once, its line
/// naming no check, and the check is given up on the pool's thread rather than left waiting there.
#[tokio::test]
async fn a_floor_that_fails_ends_its_evaluation_without_waiting_for_the_check() {
    let scratch = scratch_or_skip!();
    let (written, _keeping) = Written::keep();
    let (gate, router) = ready(&scratch.pool).await;
    let (id, _) = draft(&router, "square", None).await;
    let held = gate.world.wps();
    let hold = held.hold_resolves();
    // The Square's aisles overrun a side this short, so the site these knobs make cannot be drawn.
    let (n, _) = set(&router, &id, 1, json!({ "aisles": 6, "side": 40800 })).await;
    let ended = timeout(Duration::from_secs(30), settled(&router, &id)).await;
    let read = ended.expect("the failed evaluation ended while its check's reads were held");
    assert_eq!((read["latest"]["n"].as_u64(), read["latest"]["state"].as_str()), (Some(n), Some("failed")), "{read}");
    let evaluation = support::templates::get(&router, &format!("/api/template-drafts/{id}/evaluations/{n}")).await;
    assert!(evaluation["problems"].as_array().unwrap().iter().any(|p| p["code"] == "floor-compile-failed"));
    until("the check beside it to give up its held resolution", || async {
        (held.resolves_held() > 0 && held.resolves_waiting() == 0).then_some(())
    })
    .await;
    let line = timed(&written, &id, n).await;
    assert_eq!(line.outcome, "failed");
    assert!(line.phases.iter().any(|phase| phase == "load"), "{:?}", line.phases);
    assert!(!line.phases.iter().any(|phase| phase == "check"), "{:?}", line.phases);
    drop(hold);
    assert_eq!(timed_lines(&written, &id, n).len(), 1);
    scratch.close().await;
}

/// Astra's review, its fourth finding and its closing note (the timing line missed the failure paths, and its `ready`
/// said what was computed, not what was recorded), with its clock finding: each evaluation logs one line, in
/// wall-clock milliseconds; a change a later change superseded while its check was held says `superseded` though it
/// drew a floor, the others `published`; and each phase's span carries its wall clock as `wall_clock_ms`.
#[tokio::test]
async fn each_evaluation_logs_the_outcome_its_row_was_committed_with_in_wall_clock_milliseconds() {
    let scratch = scratch_or_skip!();
    let (written, _keeping) = Written::keep();
    let (gate, router) = ready(&scratch.pool).await;
    let (id, _) = draft(&router, "square", None).await;
    let held = gate.world.wps();
    let hold = held.hold_resolves();
    let (second, revision) = set(&router, &id, 1, json!({ "aisles": 6 })).await;
    until("the second evaluation's check to reach its held resolution", || async {
        (held.resolves_held() > 0).then_some(())
    })
    .await;
    let (third, _) = set(&router, &id, revision, json!({ "aisles": 7 })).await;
    drop(hold);
    let read = settled(&router, &id).await;
    assert_eq!(
        (read["latest"]["n"].as_u64(), read["latest"]["state"].as_str()),
        (Some(third), Some("ready")),
        "{read}"
    );
    let mut outcomes = Vec::new();
    for n in [1, second, third] {
        let line = timed(&written, &id, n).await;
        outcomes.push((n, line.outcome.clone()));
        if n == second {
            assert!(line.phases.iter().any(|phase| phase == "compile"), "it drew its floor: {:?}", line.phases);
        }
    }
    let said = |outcome: &str| outcome.to_owned();
    assert_eq!(outcomes, [(1, said("published")), (second, said("superseded")), (third, said("published"))]);
    for n in [1, second, third] {
        assert_eq!(timed_lines(&written, &id, n).len(), 1, "evaluation {n} logs one line");
    }
    let lines = written.lines();
    let spans: Vec<&String> = lines.iter().filter(|line| line.contains("evaluation_phase{")).collect();
    assert!(!spans.is_empty(), "the phases' spans were written");
    for line in spans {
        assert!(line.contains(" wall_clock_ms="), "{line}");
    }
    for line in &lines {
        assert!(!line.contains(" ms=") && !line.contains("total_ms=") && !line.contains(" ready="), "{line}");
    }
    scratch.close().await;
}

/// Astra's review, its fourth finding on the compute pool's path: an evaluation the pool never answers is recorded
/// failed and still logs its line, with the phases it completed before the pool and the record's.
#[tokio::test]
async fn an_evaluation_the_pool_never_answers_is_recorded_failed_and_still_timed() {
    let scratch = scratch_or_skip!();
    let (written, _keeping) = Written::keep();
    let (_, router) = ready_on(&scratch.pool, Arc::new(Dropping)).await;
    let (id, read) = draft(&router, "square", None).await;
    assert_eq!(read["latest"]["state"], "failed", "{read}");
    let line = timed(&written, &id, 1).await;
    assert_eq!(
        (line.outcome.as_str(), line.phases),
        ("failed", ["row", "read", "ids", "finish"].map(String::from).to_vec())
    );
    assert_eq!(timed_lines(&written, &id, 1).len(), 1);
    scratch.close().await;
}

/// Astra's review, its fourth finding on the template read's path: a change whose template cannot be read at the
/// changesets its draft names (here past each scope's head, as no read the API kept answers) is recorded failed and
/// still logs its line, with the row's read, the template's and the record's.
#[tokio::test]
async fn an_evaluation_whose_template_cannot_be_read_is_recorded_failed_and_still_timed() {
    let scratch = scratch_or_skip!();
    let (written, _keeping) = Written::keep();
    let (_, router) = ready(&scratch.pool).await;
    let (id, _) = draft(&router, "square", None).await;
    sqlx::query(
        "UPDATE template_drafts SET template_seqs = \
         (SELECT jsonb_object_agg(key, 99999) FROM jsonb_each(template_seqs)) WHERE id = $1",
    )
    .bind(&id)
    .execute(&scratch.pool)
    .await
    .expect("the draft names changesets no scope holds");
    let (n, _) = set(&router, &id, 1, json!({ "aisles": 6 })).await;
    let read = settled(&router, &id).await;
    assert_eq!(read["latest"]["state"], "failed", "{read}");
    let line = timed(&written, &id, n).await;
    assert_eq!((line.outcome.as_str(), line.phases), ("failed", ["row", "read", "finish"].map(String::from).to_vec()));
    assert_eq!(timed_lines(&written, &id, n).len(), 1);
    scratch.close().await;
}

/// A compute pool whose thread died: it takes each computation and drops it unrun, so the computation ends at once
/// without an answer.
struct Dropping;

impl Compute for Dropping {
    fn submit(&self, _job: Box<dyn FnOnce() + Send>) -> Result<(), ComputeBusy> {
        Ok(())
    }
}

/// Set `knobs` on draft `id` at `revision`: the number of the evaluation the change started, and the draft's revision.
async fn set(router: &Router, id: &str, revision: u64, knobs: Value) -> (u64, u64) {
    let body = json!({ "revision": revision, "knobs": knobs });
    let (status, _, answer) = send(router, "PATCH", &format!("/api/template-drafts/{id}"), Some(body), None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    (
        answer["evaluation"]["n"].as_u64().expect("an evaluation number"),
        answer["revision"].as_u64().expect("a revision"),
    )
}

/// What the API's tracing writes on this test's thread, span closes included, kept for the test to read: a
/// `#[tokio::test]` runs every task it spawns on that thread, each evaluation's among them.
#[derive(Clone, Default)]
struct Written(Arc<Mutex<Vec<u8>>>);

impl Written {
    /// Keep this thread's lines from now until the guard drops.
    fn keep() -> (Self, DefaultGuard) {
        let written = Self::default();
        let subscriber = fmt()
            .with_writer(written.clone())
            .with_ansi(false)
            .with_max_level(Level::INFO)
            .with_span_events(FmtSpan::CLOSE)
            .finish();
        (written, set_default(subscriber))
    }

    fn lines(&self) -> Vec<String> {
        String::from_utf8_lossy(&self.0.lock().expect("the lines")).lines().map(str::to_owned).collect()
    }
}

impl Write for Written {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("the lines").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Written {
    type Writer = Written;

    fn make_writer(&'a self) -> Written {
        self.clone()
    }
}

/// One evaluation's timing line read back: the outcome it says, and its phases' names in event order.
struct Timed {
    outcome: String,
    phases: Vec<String>,
}

/// The timing lines evaluation `n` of draft `id` has written so far.
fn timed_lines(written: &Written, id: &str, n: u64) -> Vec<Timed> {
    let parse = |line: &String| {
        let (_, rest) = line.split_once(TIMED)?;
        let (fields, phases) = rest.split_once(" phases_wall_clock_ms=")?;
        let field = |name: &str| fields.split(' ').find_map(|pair| pair.strip_prefix(name)).map(str::to_owned);
        let names = phases.split(' ').filter_map(|pair| pair.split_once('=')).map(|(name, _)| name.to_owned());
        let ours = field("draft=")? == id && field("n=")? == n.to_string();
        ours.then(|| Timed { outcome: field("outcome=").unwrap_or_default(), phases: names.collect() })
    };
    written.lines().iter().filter_map(parse).collect()
}

/// Evaluation `n` of draft `id`'s timing line, once it is written.
async fn timed(written: &Written, id: &str, n: u64) -> Timed {
    until("the evaluation's timing line", || async { timed_lines(written, id, n).into_iter().next() }).await
}
