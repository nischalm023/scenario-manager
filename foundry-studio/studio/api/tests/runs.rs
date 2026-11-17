//! Spec 08 section 8 against the database and the world-proto-server client's in-memory mock: statements 2 (a WPS
//! failure while checking the third world fails the plan and writes no run), 3 (one cut for every run; the plan's hash
//! changes with every pinned input), 4's run half (a head moved after the plan runs the plan's changesets), 5 (a
//! stale plan, wall-clock stops, a disabled watchdog and a window off its slices refused) and 14 (the three findings,
//! apart; a token opens only its own attempt's routes, refused once its Job is known stopped); spec 01 statement 15
//! (the runners' door); spec 07 statements 3's run half (a submission seals; a stale experiment creates no run), 9 (a
//! flow changeset between two submissions) and 15 (the runner's scenario version and workload stream). What a runner
//! delivers of its own run is `runs_delivery`'s, and the deployment's counts `runs_stats`'.

mod support;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use dessim::worldmap::load_from_bytes;
use desunits::{SimDuration, SimTime};
use http_body_util::BodyExt;
use run_invocation::{AttemptKey, Creator, RunStore, Seats, Token};
use scenario_spec::{COMPILER, apply_variant, compile, generate, site_facts, stated_defaults};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Connection;
use studio_api::runner_routes;
use studio_api::runs::{PgRunStore, plan_sha256, press, resume, run_refusals};
use studio_api::scenarios::{SiteReader, check};
use studio_protocol::{
    AppliedTo, ApprovalAnswer, ApprovalState, ApprovalVia, BranchName, Execution, Expectation, ExperimentKind,
    Manifest, Patch, Pin, Plan, PlanFormat, PlanRunner, PlanScenario, PlannedRun, QueueSnapshot, ReportWindow,
    Revision, RunCount, RunOverrides, RunPosition, RunnerChosen, ScopePath, Seed, Seq, SitePath, SubmitRequest,
    UnitCut, VersionNumber, WallClock,
};
use tokio::time::timeout;
use tower::ServiceExt;
use wps_client::{Client, CutExport, MockWps, Op, UnitCut as WpsCut, WpsError};

use support::runs::{
    RunStack, config, count, door, experiment, identity, own_connection, plan as ready, plan_with, reserved, scenario,
    send, stack, submit, until, waiting_on,
};
use support::sites::below;

/// One change a test makes to a value before it is used.
type Change<T> = Box<dyn Fn(&mut T)>;

/// The manifest a faithful runner of `plan`'s first run would hand in, before any change a test makes to it.
async fn manifest(stack: &RunStack, plan: &Plan, key: &AttemptKey, token: &Token) -> Manifest {
    let runners = runner_routes(stack.gate.app.clone());
    let (status, pins) = door(&runners, "GET", &format!("/runner/v1/attempts/{}/1", key.run), Some(token), None).await;
    assert_eq!(status, StatusCode::OK, "{pins}");
    let expected: Expectation = serde_json::from_value(pins["expected"].clone()).expect("the expectation");
    let reads: BTreeMap<ScopePath, Seq> = serde_json::from_value(pins["reads"].clone()).expect("reads");
    serde_json::from_value(json!({
        "v": 1, "run": key.run, "attempt": 1, "plan": plan.id, "experiment": plan.experiment,
        "scenario": pins["scenario"], "cut": pins["cut"], "reads": reads, "exports": [],
        "world": {
            "sha256": expected.world_sha256, "scopes": reads, "walked": "annotations", "exact": true,
            "exact_reason": null, "assembled_by": "server", "derived_by": "server", "primary_frame": null,
        },
        "variant": [],
        "workload": {
            "spec": { "v": 1, "generator": plan_generator(stack, plan).await, "duration_sim_ms": pins["duration_sim_ms"],
                      "slots": 0, "brings_stock_in": true },
            "seed": 0, "generator": "0.1.0", "sha256": expected.workload_sha256, "bytes": 1,
        },
        "compile": { "v": 1, "compiler": expected.compiler, "digest": expected.compile_sha256, "expected": expected,
                     "differs": [] },
        "seed": pins["seed"], "duration_sim_ms": pins["duration_sim_ms"], "report_window": pins["report_window"],
        "runner_image": pins["runner"]["image"], "fetched_at": WallClock::now(), "stub": true,
    }))
    .expect("a manifest")
}

async fn plan_generator(stack: &RunStack, plan: &Plan) -> Value {
    let (_, read) = send(&stack.router, "GET", &format!("/api/scenarios/{}", plan.scenario.id), None).await;
    read["version"]["content"]["workload"]["generator"].clone()
}

/// Statement 3: every run of a submission reads the one cut the plan resolved, and each run's expectation names the
/// export the check loaded, before any variant, and the workload it generated.
#[tokio::test]
async fn every_run_of_a_plan_reads_one_cut_and_records_the_checks_digests() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let scenario = scenario(&stack, "Peak hour").await;
    let made = experiment(&stack, &scenario, 1, &[200, 250, 300]).await;
    assert_eq!(made.kind, ExperimentKind::Sweep);
    let (plan, hash) = ready(&stack, &made.id).await;
    assert_eq!(plan.runs.len(), 3);
    assert_eq!(hash, plan_sha256(&plan), "the stored hash is the plan's");
    let cut = SiteReader::resolve_cut(&*stack.gate.world, &stack.site.as_str().parse::<SitePath>().expect("a site"))
        .await
        .expect("the cut");
    let export = SiteReader::export(&*stack.gate.world, &cut).await.expect("the export");
    assert_eq!(plan.reads, export.scopes, "the plan reads what the export holds");
    for run in &plan.runs {
        assert_eq!(run.expected.world_sha256, export.sha256, "the world before the variant");
        assert_eq!(run.expected.compiler, COMPILER);
        assert_eq!(run.expected.compile_sha256.as_ref().map(String::len), Some(71));
    }
    for (run, variant) in plan.runs.iter().zip(&made.variants) {
        let mut world: Value = serde_json::from_slice(&export.bytes).unwrap();
        let mut content = stated_defaults();
        apply_variant(variant, &mut world, &mut content).unwrap();
        let bytes = serde_json::to_vec(&world).unwrap();
        let checked = check(&bytes, &content, &RunOverrides::default(), &scenario_spec::this_build_accepts());
        let map = load_from_bytes(&bytes).unwrap();
        let workload = generate(&content.workload.generator, run.duration_sim_ms, site_facts(&map));
        let runner = compile(&map, &content, &workload).compiled.unwrap();
        assert_eq!(checked.compiled.as_ref(), Some(&runner), "readiness and the runner compile identical inputs");
        assert_eq!(run.expected.compile_sha256.as_ref(), Some(&runner.digest), "the plan pins the executed flow");
    }
    let digests: Vec<&str> = plan.runs.iter().map(|run| run.expected.workload_sha256.as_str()).collect();
    assert!(digests[0] != digests[1] && digests[1] != digests[2], "each rate generates its own workload");
    submit(&stack, &plan, &hash).await;
    let reads: Vec<Value> = sqlx::query_scalar("SELECT reads FROM runs ORDER BY position")
        .fetch_all(&scratch.pool)
        .await
        .expect("the runs' reads");
    assert_eq!(reads.len(), 3);
    assert!(reads.iter().all(|read| *read == reads[0]), "every run of the submission reads the same changesets");
    scratch.close().await;
}

/// A reader over the stack's client that fails its `failing`th export, as world-proto-server going away would.
struct Failing<'a> {
    inner: &'a Client<MockWps>,
    exports: AtomicUsize,
    failing: usize,
}

impl SiteReader for Failing<'_> {
    async fn resolve_cut(&self, site: &SitePath) -> Result<WpsCut, WpsError> {
        SiteReader::resolve_cut(self.inner, site).await
    }

    async fn export(&self, cut: &WpsCut) -> Result<CutExport, WpsError> {
        if self.exports.fetch_add(1, Ordering::SeqCst) + 1 == self.failing {
            return Err(WpsError::Unavailable { attempts: 3, last: "the test took it away".to_owned() });
        }
        SiteReader::export(self.inner, cut).await
    }
}

/// Statement 2: a WPS failure while checking the third world leaves a failed plan, no approval and no submission, run
/// or attempt.
#[tokio::test]
async fn a_wps_failure_while_checking_the_third_world_fails_the_plan_and_writes_no_run() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let scenario = scenario(&stack, "Peak hour").await;
    let made = experiment(&stack, &scenario, 1, &[200, 250, 300, 350]).await;
    // The plan's own export is the first; the three worlds' checks export again, the third of them fourth.
    let failing = Failing { inner: &stack.gate.world, exports: AtomicUsize::new(0), failing: 4 };
    let (id, ready) = plan_with(&stack, &failing, &made.id).await;
    assert!(ready.is_none(), "the plan is not ready");
    let (state, problem, progress): (String, Value, Value) =
        sqlx::query_as("SELECT state, problem, progress FROM plans WHERE id = $1")
            .bind(id.as_str())
            .fetch_one(&scratch.pool)
            .await
            .expect("the plan's row");
    assert_eq!((state.as_str(), problem["type"].as_str()), ("failed", Some("wps-unavailable")), "{problem}");
    assert_eq!(progress, json!({ "worlds_done": 2, "worlds": 4 }), "two worlds were checked before the third failed");
    for table in ["approvals", "submissions", "runs", "attempts"] {
        assert_eq!(count(&scratch.pool, table).await, 0, "no row in {table}");
    }
    scratch.close().await;
}

fn a_plan() -> Plan {
    let site: ScopePath = "customer/demo/site/lshape-6".parse().expect("a scope");
    let flow = below(&site, "flow");
    let window = ReportWindow { from_sim_ms: SimTime::ms(0), to_sim_ms: None, before_name: "warm-up".to_owned() };
    Plan {
        v: PlanFormat(1),
        id: support::ulid(),
        experiment: support::ulid(),
        kind: ExperimentKind::Sweep,
        scenario: PlanScenario {
            id: support::ulid(),
            version: VersionNumber(1),
            revision: Revision(1),
            name: "Peak hour".to_owned(),
        },
        cut: UnitCut {
            unit: site.clone(),
            requested: Pin::Branch(BranchName::main()),
            branch: BranchName::main(),
            seqs: [(site.clone(), Seq::new(1)), (flow.clone(), Seq::new(2))].into(),
            exact: true,
            exact_reason: None,
        },
        reads: [(site, Seq::new(1)), (flow, Seq::new(2))].into(),
        prepared_at: WallClock::now(),
        runner: PlanRunner {
            image: "local:simrunner@test".to_owned(),
            tag: "test".to_owned(),
            chosen: RunnerChosen::Default,
        },
        runs: vec![PlannedRun {
            run: support::ulid(),
            position: RunPosition(1),
            seed: Seed(0),
            duration_sim_ms: SimDuration::ms(10_800_000),
            report_window: window,
            variant: vec![Patch {
                target: "scenario:x".to_owned(),
                pointer: "/workload/generator/orders_per_hour".to_owned(),
                value: json!(200),
                label: "Orders per hour".to_owned(),
                applied_to: AppliedTo::Spec,
            }],
            expected: Expectation {
                compile_sha256: None,
                world_sha256: "a".repeat(64),
                compiler: "none".to_owned(),
                workload_sha256: "b".repeat(64),
            },
        }],
        queue: QueueSnapshot { limit: RunCount(5), going: RunCount(0), waiting: RunCount(0), as_of: WallClock::now() },
    }
}

/// Statement 3, the hash: changing any pin, variant value, seed, image digest or expectation changes `plan_sha256`;
/// the queue, which is information, does not.
#[test]
fn plan_sha256_changes_with_every_pinned_input_and_never_with_the_queue() {
    let plan = a_plan();
    let hash = plan_sha256(&plan);
    let mut queued = plan.clone();
    queued.queue.waiting = RunCount(7);
    assert_eq!(plan_sha256(&queued), hash, "the queue is no input");
    let changes: Vec<(&str, Change<Plan>)> = vec![
        ("a pin", Box::new(|plan| *plan.cut.seqs.values_mut().last().expect("a scope") = Seq::new(3))),
        ("a read", Box::new(|plan| *plan.reads.values_mut().last().expect("a scope") = Seq::new(3))),
        ("the variant", Box::new(|plan| plan.runs[0].variant[0].value = json!(250))),
        ("the seed", Box::new(|plan| plan.runs[0].seed = Seed(1))),
        ("the image", Box::new(|plan| plan.runner.image = "local:simrunner@other".to_owned())),
        ("the expectation", Box::new(|plan| plan.runs[0].expected.workload_sha256 = "c".repeat(64))),
        ("the scenario's revision", Box::new(|plan| plan.scenario.revision = Revision(2))),
    ];
    for (what, change) in changes {
        let mut changed = plan.clone();
        change(&mut changed);
        assert_ne!(plan_sha256(&changed), hash, "changing {what} changes the hash");
    }
}

/// Statement 5, the run settings and window: a run that stops by wall clock, one without its watchdog and a report
/// window from 7,205,000 ms of simtime are each refused naming the field.
#[test]
fn a_wall_clock_stop_a_disabled_watchdog_and_a_window_off_the_slices_are_refused() {
    let window = |from| ReportWindow { from_sim_ms: SimTime::ms(from), to_sim_ms: None, before_name: "warm-up".into() };
    let hours = SimDuration::ms(10_800_000);
    assert!(run_refusals(&json!({ "settings": {} }), &window(0), hours).is_empty(), "a run as the check writes it");
    let stops = run_refusals(&json!({ "settings": { "wall_timeout_secs": 600 } }), &window(0), hours);
    assert_eq!(
        stops.iter().map(|item| item.field.as_deref()).collect::<Vec<_>>(),
        [Some("/settings/wall_timeout_secs")]
    );
    let watchdog = run_refusals(&json!({ "settings": { "no_watchdog": true } }), &window(0), hours);
    assert_eq!(watchdog.iter().map(|item| item.field.as_deref()).collect::<Vec<_>>(), [Some("/settings/no_watchdog")]);
    let off = run_refusals(&json!({ "settings": {} }), &window(7_205_000), hours);
    assert_eq!(off.len(), 1);
    assert_eq!(
        (off[0].code.as_str(), off[0].field.as_deref()),
        ("window-not-recorded", Some("/report_window/from_sim_ms"))
    );
    assert!(off[0].text.contains("7200000 ms and 7210000 ms"), "{}", off[0].text);
}

/// Statement 5 and spec 07 statement 3's run half: a scenario edited after the plan makes its execution
/// `plan-stale` and creates no run; the hash a press names must be the plan's; a second plan of a submitted experiment
/// is `experiment-submitted`; a press repeated answers the first approval; and a submission seals the version, queues
/// one attempt per run at the approval's instant and records its execution `ok`.
#[tokio::test]
async fn execution_checks_the_hash_the_revision_and_the_experiment_and_seals_in_one_transaction() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let scenario = scenario(&stack, "Peak hour").await;
    let made = experiment(&stack, &scenario, 1, &[]).await;
    let (plan, hash) = ready(&stack, &made.id).await;
    let (status, refused) =
        send(&stack.router, "POST", "/api/runs", Some(json!({ "plan": plan.id, "plan_sha256": "0".repeat(64) }))).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("plan-stale")), "{refused}");
    let edit = json!({ "version": 1, "revision": 1, "set": [{ "pointer": "/run_controls/duration_sim_ms", "value": 7_200_000 }] });
    assert_eq!(send(&stack.router, "PATCH", &format!("/api/scenarios/{scenario}"), Some(edit)).await.0, StatusCode::OK);
    let body = json!({ "plan": plan.id, "plan_sha256": hash });
    let (status, stale) = send(&stack.router, "POST", "/api/runs", Some(body)).await;
    assert_eq!((status, stale["type"].as_str()), (StatusCode::CONFLICT, Some("plan-stale")), "{stale}");
    for table in ["submissions", "runs", "attempts"] {
        assert_eq!(count(&scratch.pool, table).await, 0, "no row in {table}");
    }
    // A fresh experiment at revision 2 submits, sealing the version.
    let made = experiment(&stack, &scenario, 2, &[200, 300]).await;
    let (plan, hash) = ready(&stack, &made.id).await;
    let body = json!({ "plan": plan.id, "plan_sha256": hash });
    let (status, first) = send(&stack.router, "POST", "/api/runs", Some(body.clone())).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{first}");
    let id = first["approval"]["id"].as_str().expect("the approval's record").to_owned();
    let (_, again) = send(&stack.router, "POST", "/api/runs", Some(body)).await;
    assert_eq!(again["approval"]["id"], id.as_str(), "a press repeated answers the same approval");
    let approval = executed(&stack, &id).await;
    assert_eq!(
        (approval["approval"]["via"].as_str(), approval["approval"]["execution"].as_str()),
        (Some("button"), Some("ok"))
    );
    assert_eq!(approval["approval"]["outcome"]["runs"].as_array().map(Vec::len), Some(2));
    let digest: String = sqlx::query_scalar("SELECT input_digest FROM approvals WHERE id = $1")
        .bind(&id)
        .fetch_one(&scratch.pool)
        .await
        .expect("the approval");
    assert_eq!(digest, hash, "the approval's input digest is the plan's hash");
    let sealed: String = sqlx::query_scalar("SELECT state FROM scenario_versions WHERE scenario = $1 AND version = 1")
        .bind(scenario.as_str())
        .fetch_one(&scratch.pool)
        .await
        .expect("the version");
    assert_eq!(sealed, "sealed");
    let queued: Vec<(String, i32)> = sqlx::query_as("SELECT state, n FROM attempts ORDER BY run_id")
        .fetch_all(&scratch.pool)
        .await
        .expect("the attempts");
    assert_eq!(queued, [("queued".to_owned(), 1), ("queued".to_owned(), 1)]);
    let (status, refused) = send(&stack.router, "POST", "/api/runs/plan", Some(json!({ "experiment": made.id }))).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("experiment-submitted")), "{refused}");
    scratch.close().await;
}

/// The approval `id` once its execution has ended, read as a page reads it.
async fn executed(stack: &RunStack, id: &str) -> Value {
    let read = || async { send(&stack.router, "GET", &format!("/api/approvals/{id}"), None).await.1 };
    until("the execution to end", || async {
        !["not_started", "running"].contains(&read().await["approval"]["execution"].as_str().unwrap_or(""))
    })
    .await;
    read().await
}

/// Spec 01 section 3.10's rule 6 and spec 08 section 3.7, "at once" (Astra's run-chain review, the sixth finding): a
/// press of Run is answered `202` with its approval's record, approved by the button and its execution not started,
/// while the scenario its execution must seal is locked, the approval not yet `ok` and no submission made; once the
/// lock is let go the execution follows on its own, `ok`, its runs queued at the approval's instant.
#[tokio::test]
async fn a_press_is_answered_before_its_execution_which_follows_on_its_approval() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let scenario = scenario(&stack, "Peak hour").await;
    let made = experiment(&stack, &scenario, 1, &[]).await;
    let (plan, hash) = ready(&stack, &made.id).await;
    let mut holder = own_connection(&scratch.pool).await;
    let mut held = holder.begin().await.expect("the test's transaction");
    sqlx::query("SELECT id FROM scenarios WHERE id = $1 FOR UPDATE")
        .bind(scenario.as_str())
        .execute(&mut *held)
        .await
        .unwrap();
    let body = json!({ "plan": plan.id, "plan_sha256": hash });
    let pressed = timeout(Duration::from_secs(5), send(&stack.router, "POST", "/api/runs", Some(body))).await;
    let (status, accepted) = pressed.expect("the press is answered while its execution waits on the scenario");
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let ApprovalAnswer { approval } = serde_json::from_value(accepted).expect("the 202 is the approval's record");
    assert_eq!(
        (approval.tool.as_str(), approval.via, approval.state, approval.execution),
        ("run.submit", ApprovalVia::Button, ApprovalState::Approved, Some(Execution::NotStarted))
    );
    let id = approval.id.to_string();
    let (_, read) = send(&stack.router, "GET", &format!("/api/approvals/{id}"), None).await;
    assert_ne!(read["approval"]["execution"], "ok", "nothing is executed while the scenario is locked: {read}");
    assert_eq!(count(&scratch.pool, "submissions").await, 0);
    held.commit().await.expect("the scenario let go");
    let approval = executed(&stack, &id).await;
    assert_eq!(approval["approval"]["execution"], "ok", "{approval}");
    let queued: bool = sqlx::query_scalar(
        "SELECT bool_and(t.requested_at = a.decided_at) FROM attempts t, approvals a WHERE a.id = $1",
    )
    .bind(&id)
    .fetch_one(&scratch.pool)
    .await
    .expect("the attempts");
    assert!(queued, "its runs are queued at the approval's instant");
    scratch.close().await;
}

/// LEAD-DECISIONS entry 54's recovery: a press a stopped process recorded but never executed is executed by the next
/// process's start.
#[tokio::test]
async fn a_press_a_stopped_process_left_unexecuted_is_executed_at_the_next_start() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let scenario = scenario(&stack, "Peak hour").await;
    let made = experiment(&stack, &scenario, 1, &[]).await;
    let (plan, hash) = ready(&stack, &made.id).await;
    let request = SubmitRequest { plan: plan.id.clone(), plan_sha256: hash };
    let approval = press(&stack.gate.app, &identity(), &request, "/api/runs").await.expect("the press recorded").id;
    assert_eq!(count(&scratch.pool, "submissions").await, 0, "the press alone submits nothing");
    resume(&stack.gate.app).await.expect("the start's resumption");
    let read = executed(&stack, approval.as_str()).await;
    assert_eq!(read["approval"]["execution"], "ok", "{read}");
    assert_eq!(count(&scratch.pool, "submissions").await, 1);
    scratch.close().await;
}

/// Spec 07 sections 3.6 and 3.8 (Astra's run-chain review, the fourteenth finding): an experiment reads with no
/// submission and no runs until it is submitted, then with its submission and that submission's runs in position
/// order, each as the run list shows it.
#[tokio::test]
async fn an_experiment_reads_with_its_submission_and_runs() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let scenario = scenario(&stack, "Peak hour").await;
    let made = experiment(&stack, &scenario, 1, &[200, 300]).await;
    let read = || async { send(&stack.router, "GET", &format!("/api/experiments/{}", made.id), None).await };
    let (status, before) = read().await;
    assert_eq!(status, StatusCode::OK, "{before}");
    assert_eq!((&before["experiment"]["submission"], &before["runs"]), (&Value::Null, &json!([])));
    let (plan, hash) = ready(&stack, &made.id).await;
    submit(&stack, &plan, &hash).await;
    let submission: String =
        sqlx::query_scalar("SELECT id FROM submissions").fetch_one(&scratch.pool).await.expect("the submission");
    let (_, after) = read().await;
    assert_eq!(after["experiment"]["submission"], submission.as_str(), "{after}");
    let runs: Vec<&str> =
        after["runs"].as_array().expect("its runs").iter().filter_map(|run| run["id"].as_str()).collect();
    let planned: Vec<String> = plan.runs.iter().map(|run| run.run.to_string()).collect();
    assert_eq!(runs, planned, "every run, in position order");
    assert_eq!(after["runs"][0]["state"], "queued");
    scratch.close().await;
}

/// Spec 08 section 3.7's list, a page at a time (Astra's run-chain review, the eleventh finding): pages of two, each
/// continuing `after` the last one's `next`, give every run of the site exactly once in the list's order, newest
/// submission first, the last page naming no `next`; an `after` naming no run is refused.
#[tokio::test]
async fn the_run_list_pages_by_a_cursor_through_every_run_once() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let scenario = scenario(&stack, "Peak hour").await;
    for values in [&[200, 300, 400][..], &[], &[250, 350]] {
        let made = experiment(&stack, &scenario, 1, values).await;
        let (plan, hash) = ready(&stack, &made.id).await;
        submit(&stack, &plan, &hash).await;
    }
    let site = urlencoding(&stack.site.to_string());
    let (_, whole) = send(&stack.router, "GET", &format!("/api/runs?site={site}"), None).await;
    let every: Vec<Value> = whole["items"].as_array().expect("the list").iter().map(|run| run["id"].clone()).collect();
    assert_eq!(every.len(), 6);
    let (mut paged, mut after, mut pages) = (Vec::new(), None::<String>, 0);
    loop {
        let from = after.as_ref().map(|id| format!("&after={id}")).unwrap_or_default();
        let (status, page) = send(&stack.router, "GET", &format!("/api/runs?site={site}&limit=2{from}"), None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        paged.extend(page["items"].as_array().expect("a page").iter().map(|run| run["id"].clone()));
        pages += 1;
        match page["next"].as_str() {
            Some(next) => after = Some(next.to_owned()),
            None => break,
        }
    }
    assert_eq!((paged, pages), (every, 3), "each run once, in the list's order, in three pages");
    let (status, refused) =
        send(&stack.router, "GET", &format!("/api/runs?site={site}&after={}", support::ulid()), None).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("validation-failed")));
    scratch.close().await;
}

/// A query value with its slashes escaped.
fn urlencoding(text: &str) -> String {
    text.replace('/', "%2F")
}

/// Spec 01 section 3.10's rule 6 (Astra's run-chain review, the fifth finding): two presses of Run on one plan, both
/// waiting on its row while another holds it, answer the one approval, never `experiment-submitted` for the second;
/// its execution makes one submission.
#[tokio::test]
async fn two_presses_racing_on_one_plan_answer_one_approval() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let scenario = scenario(&stack, "Peak hour").await;
    let made = experiment(&stack, &scenario, 1, &[]).await;
    let (plan, hash) = ready(&stack, &made.id).await;
    let mut holder = own_connection(&scratch.pool).await;
    let mut held = holder.begin().await.expect("the test's transaction");
    sqlx::query("SELECT id FROM plans WHERE id = $1 FOR UPDATE")
        .bind(plan.id.as_str())
        .execute(&mut *held)
        .await
        .unwrap();
    let body = json!({ "plan": plan.id, "plan_sha256": hash });
    let press = || {
        let (router, body) = (stack.router.clone(), body.clone());
        tokio::spawn(async move { send(&router, "POST", "/api/runs", Some(body)).await })
    };
    let (first, second) = (press(), press());
    waiting_on(&scratch.pool, "plans", 2).await;
    held.commit().await.expect("the plan let go");
    let (first, second) = (first.await.expect("a press"), second.await.expect("a press"));
    assert_eq!((first.0, second.0), (StatusCode::ACCEPTED, StatusCode::ACCEPTED), "{} {}", first.1, second.1);
    assert_eq!(first.1["approval"]["id"], second.1["approval"]["id"], "one approval for both presses");
    let read = executed(&stack, first.1["approval"]["id"].as_str().expect("its approval")).await;
    assert_eq!(read["approval"]["execution"], "ok", "{read}");
    assert_eq!(count(&scratch.pool, "submissions").await, 1);
    scratch.close().await;
}

/// Statement 4's run half and spec 07 statement 9: a flow changeset applied after a plan changes nothing the plan's
/// runs read, which the runner's pins name; the next submission reads the new changeset, and the scenario keeps its
/// version and revision.
#[tokio::test]
async fn runs_submitted_either_side_of_a_flow_changeset_each_record_the_one_they_read() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let flow_scope = below(&stack.site, "flow");
    let scenario = scenario(&stack, "Peak hour").await;
    let made = experiment(&stack, &scenario, 1, &[]).await;
    let (first, hash) = ready(&stack, &made.id).await;
    let before = first.reads[&flow_scope];
    let moved = support::stage(&stack.gate.world, &flow_scope, Vec::new(), None).await;
    assert!(moved > before, "the flow's head moved past the plan");
    submit(&stack, &first, &hash).await;
    let store = PgRunStore::new(stack.gate.app.clone());
    let reserved = store.reserve(Seats(5), &Creator::mint()).await.expect("a reservation").expect("the run waits");
    let runners = runner_routes(stack.gate.app.clone());
    let (status, pins) =
        door(&runners, "GET", &format!("/runner/v1/attempts/{}/1", reserved.key.run), Some(&reserved.token), None)
            .await;
    assert_eq!(status, StatusCode::OK, "{pins}");
    assert_eq!(pins["cut"]["seqs"][flow_scope.as_str()], json!(before.get()), "the pins name the plan's changeset");
    let made = experiment(&stack, &scenario, 1, &[]).await;
    let (second, hash) = ready(&stack, &made.id).await;
    submit(&stack, &second, &hash).await;
    let reads: Vec<Value> = sqlx::query_scalar(
        "SELECT r.reads FROM runs r JOIN submissions s ON s.id = r.submission ORDER BY s.submitted_at",
    )
    .fetch_all(&scratch.pool)
    .await
    .expect("the runs' reads");
    let flows: Vec<Value> = reads.iter().map(|read| read[flow_scope.as_str()].clone()).collect();
    assert_eq!(flows, [json!(before.get()), json!(moved.get())], "each submission records the changeset it read");
    let (version, revision): (i32, i32) = sqlx::query_as(
        "SELECT s.latest_version, v.revision FROM scenarios s JOIN scenario_versions v ON v.scenario = s.id \
                        AND v.version = s.latest_version WHERE s.id = $1",
    )
    .bind(scenario.as_str())
    .fetch_one(&scratch.pool)
    .await
    .expect("the scenario");
    assert_eq!((version, revision), (1, 1), "the scenario keeps its version and revision");
    scratch.close().await;
}

/// Spec 01 statement 15 and spec 08 statement 14's door: a token opens its own attempt's pins and manifest and no
/// other's, is stored only as its hash, and answers 401 once the Job is recorded stopped; the token a re-created Job
/// carries kills the old one, and a running attempt is never re-created; the gate-facing listener answers 404 on
/// `/runner/`; the runners' listener reads no identity header; a second manifest with other bytes is
/// `manifest-differs`, the same bytes the first answer.
#[tokio::test]
async fn a_token_opens_only_its_own_attempts_routes_until_its_job_is_known_stopped() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let (plan, key, token) = reserved(&stack, "Peak hour").await;
    let runners = runner_routes(stack.gate.app.clone());
    let own = format!("/runner/v1/attempts/{}/1", key.run);
    assert_eq!(door(&runners, "GET", &own, Some(&token), None).await.0, StatusCode::OK);
    let control = format!("{own}/control");
    assert_eq!(
        door(&runners, "GET", &control, Some(&token), None).await,
        (StatusCode::OK, json!({"cancel_requested": false}))
    );
    for (path, presented) in [
        (control.clone(), None),
        (format!("/runner/v1/attempts/{}/2/control", key.run), Some(&token)),
        (format!("/runner/v1/attempts/{}/1/control", support::ulid()), Some(&token)),
    ] {
        assert_eq!(door(&runners, "GET", &path, presented, None).await.0, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(send(&stack.router, "GET", &control, None).await.0, StatusCode::NOT_FOUND);
    let (status, refused) = door(&runners, "GET", &own, None, None).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::UNAUTHORIZED, Some("runner-token-invalid")));
    let gate_headers = support::from_gate("GET", &own, "u-1", "").body(Body::empty()).expect("a request");
    assert_eq!(
        support::call(&runners, gate_headers).await.0,
        StatusCode::UNAUTHORIZED,
        "identity headers open nothing"
    );
    assert_eq!(
        door(&runners, "GET", &format!("/runner/v1/attempts/{}/2", key.run), Some(&token), None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        door(&runners, "GET", &format!("/runner/v1/attempts/{}/1", support::ulid()), Some(&token), None).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, _) = send(&stack.router, "GET", &own, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the gate-facing listener serves no runner route");
    let stored: String =
        sqlx::query_scalar("SELECT token_sha256 FROM runner_tokens").fetch_one(&scratch.pool).await.expect("the hash");
    assert_eq!(stored, format!("{:x}", Sha256::digest(token.expose().as_bytes())), "only the token's hash is kept");
    let held: i64 = sqlx::query_scalar("SELECT count(*) FROM runner_tokens WHERE token_sha256 = $1")
        .bind(token.expose())
        .fetch_one(&scratch.pool)
        .await
        .expect("a count");
    assert_eq!(held, 0, "the token itself is stored nowhere");
    // The token a re-created Job carries kills the old one.
    let store = PgRunStore::new(stack.gate.app.clone());
    let reclaimed = store.reclaim(&key, &Creator::mint(), Duration::ZERO).await.expect("a re-creation's claim");
    let fresh = reclaimed.expect("a fresh token for a queued attempt");
    assert_eq!(
        door(&runners, "GET", &own, Some(&token), None).await.0,
        StatusCode::UNAUTHORIZED,
        "the old token is dead"
    );
    assert_eq!(door(&runners, "GET", &own, Some(&fresh), None).await.0, StatusCode::OK);
    assert_eq!(door(&runners, "GET", &control, Some(&token), None).await.0, StatusCode::UNAUTHORIZED);
    // A manifest handed in once; the same bytes again answer the first outcome, other bytes are refused.
    let handed = serde_json::to_vec(&manifest(&stack, &plan, &key, &fresh).await).expect("JSON");
    let path = format!("{own}/manifest");
    let (status, first) = door(&runners, "PUT", &path, Some(&fresh), Some(handed.clone())).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(door(&runners, "PUT", &path, Some(&fresh), Some(handed.clone())).await, (StatusCode::OK, first));
    let mut other: Value = serde_json::from_slice(&handed).expect("JSON");
    other["seed"] = json!(9);
    let (status, refused) = door(&runners, "PUT", &path, Some(&fresh), Some(serde_json::to_vec(&other).unwrap())).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("manifest-differs")));
    // A running attempt is never re-created, so no token is minted over its runner's; a Job known stopped kills
    // every token of its attempt.
    let again = store.reclaim(&key, &Creator::mint(), Duration::ZERO).await.expect("a re-creation's claim");
    assert!(again.is_none(), "a running attempt is not re-created");
    let (_, cancelled) = send(&stack.router, "POST", &format!("/api/runs/{}/cancel", key.run), None).await;
    assert_eq!(cancelled["outcome"], "stopping");
    assert_eq!(
        door(&runners, "GET", &control, Some(&fresh), None).await,
        (StatusCode::OK, json!({"cancel_requested": true}))
    );
    store.job_stopped(&key).await.expect("the Job stopped");
    assert_eq!(door(&runners, "GET", &own, Some(&fresh), None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(door(&runners, "GET", &control, Some(&fresh), None).await.0, StatusCode::UNAUTHORIZED);
    scratch.close().await;
}

/// Statement 14: a compile digest other than the expectation records `plan-differs` with both digests and compiler
/// versions, and the run goes on (`running`); a read turned inexact records `read-differs` alone, and another compiler
/// giving the same plan `compiler-differs` alone, "to the same plan"; each is journaled once in the site's
/// conversation.
#[tokio::test]
async fn the_three_findings_are_recorded_apart_and_only_another_plan_raises_plan_differs() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let cases: [(&str, Change<Manifest>, &[&str]); 3] = [
        (
            "another plan",
            Box::new(|manifest| manifest.compile.digest = Some("4c1e7a".repeat(10) + "ffff")),
            &["plan-differs"],
        ),
        (
            "a read below a moved head",
            Box::new(|manifest| {
                manifest.world.exact = false;
                manifest.world.exact_reason = Some("flow at changeset 1 is read below its head".to_owned());
            }),
            &["read-differs"],
        ),
        (
            "another compiler",
            Box::new(|manifest| manifest.compile.compiler = "0.3.0".to_owned()),
            &["compiler-differs"],
        ),
    ];
    for (what, change, kinds) in cases {
        let (plan, key, token) = reserved(&stack, what).await;
        let mut handed = manifest(&stack, &plan, &key, &token).await;
        change(&mut handed);
        let runners = runner_routes(stack.gate.app.clone());
        let path = format!("/runner/v1/attempts/{}/1/manifest", key.run);
        let (status, answer) =
            door(&runners, "PUT", &path, Some(&token), Some(serde_json::to_vec(&handed).unwrap())).await;
        assert_eq!(status, StatusCode::OK, "{what}: {answer}");
        let found: Vec<&str> =
            answer["findings"].as_array().unwrap().iter().map(|finding| finding["kind"].as_str().unwrap()).collect();
        assert_eq!(found, kinds, "{what}");
        let finding = &answer["findings"][0];
        match kinds[0] {
            "plan-differs" => {
                assert_eq!(finding["differs"], json!(["compile"]));
                assert_eq!(finding["expected"]["compile_sha256"], json!(plan.runs[0].expected.compile_sha256));
                assert!(finding["got"]["compile_sha256"].as_str().unwrap().starts_with("4c1e7a"));
                assert!(
                    finding["sentence"]
                        .as_str()
                        .unwrap()
                        .contains("compiled another flow plan than the check: its 4c1e7a…")
                );
            }
            "compiler-differs" => {
                assert!(
                    finding["sentence"]
                        .as_str()
                        .unwrap()
                        .ends_with("used compiler 0.3.0 where the check used 0.2.0 to the same plan.")
                );
            }
            _ => assert!(finding["sentence"].as_str().unwrap().contains("read below a moved head")),
        }
        let state: String = sqlx::query_scalar("SELECT state FROM attempts WHERE run_id = $1")
            .bind(key.run.as_str())
            .fetch_one(&scratch.pool)
            .await
            .expect("the attempt");
        assert_eq!(state, "running", "{what}: the run goes on");
        let journaled: i64 =
            sqlx::query_scalar("SELECT count(*) FROM journal WHERE kind = 'run.finding' AND body->>'run' = $1")
                .bind(key.run.as_str())
                .fetch_one(&scratch.pool)
                .await
                .expect("the entries");
        assert_eq!(journaled, 1, "{what}: one entry");
    }
    scratch.close().await;
}

/// Spec 07 statement 15: with its token a runner reads its run's sealed version and is refused `not-this-run` for
/// another; the stream it gets for its variant's spec comes in pieces as it is made and has the digest the check
/// recorded.
#[tokio::test]
async fn a_runner_reads_its_runs_sealed_version_and_the_workload_the_check_digested() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let (plan, key, token) = reserved(&stack, "Peak hour").await;
    let runners = runner_routes(stack.gate.app.clone());
    let own = format!("/runner/v1/scenarios/{}/versions/1", plan.scenario.id);
    let (status, sealed) = door(&runners, "GET", &own, Some(&token), None).await;
    assert_eq!(status, StatusCode::OK, "{sealed}");
    assert_eq!(sealed["version"]["state"], "sealed");
    let other = scenario(&stack, "Night shift").await;
    let (status, refused) =
        door(&runners, "GET", &format!("/runner/v1/scenarios/{other}/versions/1"), Some(&token), None).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::FORBIDDEN, Some("not-this-run")));
    let handed = manifest(&stack, &plan, &key, &token).await;
    let world = SiteReader::export(
        &*stack.gate.world,
        &SiteReader::resolve_cut(&*stack.gate.world, &stack.site.as_str().parse::<SitePath>().expect("a site"))
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    let map = dessim::worldmap::load_from_bytes(&world.bytes).expect("the world loads");
    let facts = scenario_spec::site_facts(&map);
    let mut request = handed.workload.spec.clone();
    request.slots = facts.slots;
    request.brings_stock_in = facts.brings_stock_in;
    let body = serde_json::to_vec(&request).unwrap();
    let mut call = Request::builder()
        .method("POST")
        .uri("/runner/v1/workloads/generate")
        .header("content-type", "application/json");
    call = call.header("authorization", format!("Bearer {}", token.expose()));
    let answer = runners.clone().oneshot(call.body(Body::from(body)).unwrap()).await.expect("an answer");
    assert_eq!(answer.status(), StatusCode::OK);
    assert_eq!(answer.headers()["content-type"], "application/x-ndjson");
    // The stream arrives in pieces as the generator makes it (Astra's run-chain review, the tenth finding).
    let (mut body, mut stream, mut pieces) = (answer.into_body(), Vec::new(), 0);
    while let Some(frame) = body.frame().await {
        stream.extend(frame.expect("a piece of the stream").into_data().expect("data"));
        pieces += 1;
    }
    assert!(pieces > 1, "{} bytes in {pieces} piece", stream.len());
    assert_eq!(format!("{:x}", Sha256::digest(&stream)), plan.runs[0].expected.workload_sha256, "the check's digest");
    scratch.close().await;
}

#[tokio::test]
async fn an_unsupported_or_absent_flow_refuses_readiness_and_planning_before_approval() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let scenario = scenario(&stack, "Configured flow required").await;
    let site = stack.site.as_str().parse::<SitePath>().unwrap();
    let cut = SiteReader::resolve_cut(&*stack.gate.world, &site).await.unwrap();
    let export = SiteReader::export(&*stack.gate.world, &cut).await.unwrap();
    let doc: Value = serde_json::from_slice(&export.bytes).unwrap();
    let entities = doc["entities"].as_array().unwrap();
    let pick = entities.iter().find(|entry| entry["name"] == "Pick stations").unwrap();
    let mut parts = pick["components"].clone();
    parts["flow.node"]["work"]["sequencing"] = json!("strict");
    let unsupported = vec![Op::Patch {
        name: None,
        id: pick["id"].as_str().unwrap().parse().unwrap(),
        components: support::sites::components(parts),
    }];
    let absent = entities
        .iter()
        .filter(|entry| entry["components"].as_object().unwrap().keys().any(|key| key.starts_with("flow.")))
        .map(|entry| Op::Delete { id: entry["id"].as_str().unwrap().parse().unwrap() })
        .collect();
    for (ops, code) in [(unsupported, "unsupported-value"), (absent, "flow-empty")] {
        support::stage(&stack.gate.world, &below(&stack.site, "flow"), ops, None).await;
        let (_, readiness) = send(&stack.router, "GET", &format!("/api/scenarios/{scenario}/readiness"), None).await;
        assert_eq!(readiness["ready"], false, "{readiness}");
        assert!(
            readiness["problems"]
                .as_array()
                .unwrap()
                .iter()
                .any(|problem| problem["code"] == code && problem["severity"] == "error"),
            "{readiness}"
        );
        let made = experiment(&stack, &scenario, 1, &[]).await;
        let (id, plan) = plan_with(&stack, &*stack.gate.world, &made.id).await;
        assert!(plan.is_none(), "a refused flow must not offer a plan to approve");
        let problem: Value = sqlx::query_scalar("SELECT problem FROM plans WHERE id = $1")
            .bind(id.as_str())
            .fetch_one(&scratch.pool)
            .await
            .unwrap();
        assert!(
            problem["problems"]
                .as_array()
                .unwrap()
                .iter()
                .any(|problem| problem["code"] == code && problem["severity"] == "error"),
            "{problem}"
        );
    }
    assert_eq!(count(&scratch.pool, "runs").await, 0);
    assert_eq!(count(&scratch.pool, "approvals").await, 0);
    scratch.close().await;
}
