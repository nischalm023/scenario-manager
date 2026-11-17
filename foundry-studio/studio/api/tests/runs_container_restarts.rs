//! Spec 08 section 8's statement 16 across an API restart on the local invoker, the containers outliving the API that
//! started them: an API process killed while its runner runs, the runner finishing in the outage and delivering to the
//! next process, which adopts its container; attempts a stopped process reserved, started again or ended as their
//! exited containers say; an adopted container held to the deadline it started with; a container keeping no
//! deadline, whose attempt ends while the rest are adopted; and a container whose log a stopped process had not yet
//! kept, which the next keeps before removing the container. What the tests need is in `support::containers`; any of it
//! missing, they say they are skipped.

mod support;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command as StdCommand, Stdio};
use std::slice::from_ref;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use chrono::Utc;
use run_invocation::{CpuMillis, Creator, Invoker, JobRequest, Reserved, Resources, RunStore, Seats};
use runstore::RunStore as RunObjects;
use rustc_hash::FxHashMap;
use serde_json::Value;
use studio_api::AppState;
use studio_api::config::{Deployment, RunConfig};
use studio_api::db::mint_ulid;
use studio_api::runs::{ContainerInvoker, PgRunStore, Runs};
use studio_protocol::{Endpoint, FORMAT, HOLD_VAR, JobSpec};
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::time::sleep;
use wps_client::{BlockingCompute, HttpWps};

use support::containers::{
    RemovedAtEnd, cancel, container_exists, engine, left_container, local, local_unserved, loops, loops_of, needed,
    out_of, refusing_engine, run_of, runner_image, script, serve_runners, setup, state_is, submitted,
};
use support::runs::{RunStack, experiment, plan, scenario, stack, submit, until};

/// The half minute a runner asks again for its run's inputs, 1 s, 5 s and 25 s apart: an outage longer than that is
/// ridden out only by a delivery's own four minutes of tries.
const INPUTS_RETRY_WINDOW: Duration = Duration::from_secs(31);

/// Statement 16's restart: the API process that started a runner dies, its runners' listener with it, while the runner
/// holds after its manifest; the runner finishes in the outage with nothing of its ending delivered, and its container
/// goes on. The outage outlasts the half minute the runner asks again for its inputs, so only a delivery's own tries
/// carry the ending over it. The next API process serves the same address and adopts the container, and the runner's
/// next try delivers its recording and its report: the attempt was never ended as lost, and ends as its runner says,
/// `finished`, its recording stored and its build named, which only its own report does. The first API process is a
/// real one, a child of this test running `first_api_process`, killed with SIGKILL.
#[tokio::test]
async fn an_api_process_that_dies_leaves_its_runner_to_the_next_which_takes_its_results() {
    let Some(needed) = needed() else { return };
    let scratch = scratch_or_skip!();
    // Long enough for the first process to be killed while the runner holds, short enough that it finishes after.
    let hold = (HOLD_VAR, "5000");
    let (stack, listener) = local_unserved(&scratch.pool, &needed, &needed.image, &[hold], 5).await;
    let address = listener.local_addr().expect("the runners' address");
    drop(listener);
    let runs = submitted(&stack, "Peak hour", 1).await;
    let _removed = RemovedAtEnd::of(&needed.engine, &runs);
    let app = &stack.gate.app;
    let setup = app.runs.container_setup().expect("a container setup");
    let schema: String =
        sqlx::query_scalar("SELECT current_schema()").fetch_one(&scratch.pool).await.expect("the schema");
    let mut first = StdCommand::new(env::current_exe().expect("this test's binary"))
        .args(["--ignored", "--exact", "first_api_process", "--nocapture"])
        .env("STUDIO_TEST_FIRST_API_SCHEMA", &schema)
        .env("STUDIO_TEST_FIRST_API_BIND", address.to_string())
        .env("STUDIO_TEST_FIRST_API_RUNNERS", &app.runs.config.runner_url)
        .env("STUDIO_TEST_FIRST_API_WPS", &*app.runs.wps)
        .env("STUDIO_TEST_FIRST_API_OBJECTS", &setup.objects)
        .env("STUDIO_TEST_FIRST_API_OUT", &setup.out)
        .env("STUDIO_TEST_FIRST_API_ENGINE", &setup.engine)
        .env("STUDIO_TEST_FIRST_API_IMAGE", &needed.image)
        .env(hold.0, hold.1)
        .stdout(Stdio::null())
        .spawn()
        .expect("the first API process");
    until("its runner to hand its manifest to the first process", || state_is(&stack, &runs[0], "running")).await;
    first.kill().expect("the first API process killed");
    first.wait().expect("the first API process reaped");
    let ending = out_of(&stack, &runs[0]).join("ending.json");
    until("the runner to finish while no API answers it", || async { ending.exists() }).await;
    sleep(INPUTS_RETRY_WINDOW + Duration::from_secs(10)).await;
    assert_eq!(run_of(&stack, &runs[0]).await["state"], "running", "nothing of its ending arrived");
    let second = TcpListener::bind(address).await.expect("the same address, served again");
    let _runners = serve_runners(app, second);
    let running = loops(&stack).await;
    until("the runner's own ending, delivered on its next try", || state_is(&stack, &runs[0], "finished")).await;
    let read = run_of(&stack, &runs[0]).await;
    assert_eq!((read["ended_by"].as_str(), &read["stop_detail"]), (Some("simulator"), &Value::Null), "{read}");
    assert_eq!(read["recording"]["stored"], true, "its recording, sent after the outage: {read}");
    assert!(read["attempts"][0]["runner_commit"].is_string(), "its own report, which alone names its build: {read}");
    let path: Vec<String> = sqlx::query_scalar("SELECT \"to\" FROM attempt_transitions ORDER BY tx, id")
        .fetch_all(&scratch.pool)
        .await
        .expect("the transitions");
    assert_eq!(path, ["queued", "running", "finished"], "never ended as lost");
    until("its Job known stopped", || async {
        sqlx::query_scalar::<_, bool>("SELECT job_stopped_at IS NOT NULL FROM attempts")
            .fetch_one(&scratch.pool)
            .await
            .unwrap()
    })
    .await;
    drop(running);
    scratch.close().await;
}

/// Not a test of its own: the first API process of `an_api_process_that_dies_leaves_its_runner_to_the_next_which_
/// takes_its_results`, which runs it as a child process and kills it. It serves the runners' listener at the address
/// its environment names and runs the loops, with a container invoker over the schema, folders, engine and image its
/// environment names, until it is killed.
#[tokio::test]
#[ignore = "the restart test's first API process, which that test runs and kills"]
async fn first_api_process() {
    let named = |name: &str| env::var(name).unwrap_or_else(|_| panic!("{name} is set by the restart test"));
    let pool = support::schema_pool(&named("STUDIO_TEST_FIRST_API_SCHEMA")).await;
    let objects = PathBuf::from(named("STUDIO_TEST_FIRST_API_OBJECTS"));
    let engine = PathBuf::from(named("STUDIO_TEST_FIRST_API_ENGINE"));
    let image = named("STUDIO_TEST_FIRST_API_IMAGE");
    let hold = env::var(HOLD_VAR).unwrap_or_default();
    let out = PathBuf::from(named("STUDIO_TEST_FIRST_API_OUT"));
    let setup = setup(&engine, objects.clone(), out, &[(HOLD_VAR, hold.as_str())]);
    let mut config = RunConfig::local(&named("STUDIO_TEST_FIRST_API_RUNNERS"), &image);
    config.runner_wps_url = Some(named("STUDIO_TEST_FIRST_API_WPS"));
    let runs = Runs::new(config, "http://127.0.0.1:9").container(runner_image(&image), setup);
    let app = AppState::new(
        pool,
        support::gate_rules(),
        Deployment::Local,
        support::MODEL_LABEL,
        Arc::new(BlockingCompute),
        RunObjects::local_folder(&objects).expect("the run store"),
    )
    .with_runs(runs);
    let listener = TcpListener::bind(named("STUDIO_TEST_FIRST_API_BIND")).await.expect("the runners' address");
    let _runners = serve_runners(&app, listener);
    let _running = loops_of(&app).await;
    sleep(Duration::from_secs(600)).await;
}

/// An API restart does not give a run its deadline again: the next process holds an adopted container to the deadline
/// its labels keep, so one whose deadline passed while no process watched it is stopped at once and its attempt failed
/// for its deadline. The site is the mock world-proto-server's, and the container, as a stopped process left it an
/// hour after starting it with an hour's allowance, is the test's own.
#[tokio::test]
async fn an_adopted_container_keeps_the_deadline_it_started_with() {
    let Some(engine) = engine() else { return };
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, RunConfig::local("http://127.0.0.1:9", "studio-simrunner:dev")).await;
    let scenario = scenario(&stack, "Peak hour").await;
    let made = experiment(&stack, &scenario, 1, &[]).await;
    let (plan, hash) = plan(&stack, &made.id).await;
    submit(&stack, &plan, &hash).await;
    let store = PgRunStore::new(stack.gate.app.clone());
    let held = store.reserve(Seats(5), &Creator::mint()).await.expect("a reservation").expect("a waiting attempt");
    let job = held.job.as_str();
    left_container(&engine, job, Some((Utc::now().timestamp() - 1, 3_600)), "sleep 60");
    let out = env::temp_dir().join(format!("studio-runner-out-{}", mint_ulid()));
    let setup = setup(&engine, support::runstore_folder(), out, &[]);
    let invoker = ContainerInvoker::new(stack.gate.app.clone(), Arc::new(setup));
    let adopted = invoker.adopt().await.expect("the container adopted");
    assert_eq!(adopted, from_ref(&held.job));
    until("the passed deadline to end the attempt", || async {
        store.attempt(&held.key).await.expect("the attempt").is_some_and(|row| row.state.ended())
    })
    .await;
    let reason: String = sqlx::query_scalar("SELECT stop_reason FROM attempts WHERE run_id = $1")
        .bind(held.key.run.as_str())
        .fetch_one(&scratch.pool)
        .await
        .expect("the ending");
    assert_eq!(reason, "ran past its wall-clock deadline of 1 h", "never a fresh allowance");
    until("the container removed", || async { !container_exists(&engine, job) }).await;
    scratch.close().await;
}

/// The request a dispatcher makes of `reserved`'s Job, running `image`, its runner reaching the listener at `studio`.
fn request_of(reserved: &Reserved, image: &str, studio: Endpoint) -> JobRequest {
    JobRequest {
        name: reserved.job.clone(),
        image: image.to_owned(),
        spec: JobSpec { v: FORMAT, run: reserved.key.run.clone(), attempt: reserved.key.n, studio, streaming: None },
        token: reserved.token.clone(),
        deadline: Duration::from_secs(21_600),
        resources: Resources { cpu_millis: CpuMillis(1_000), memory_mib: None },
        labels: FxHashMap::default(),
    }
}

/// The container a stopped API process started for `reserved` through the production path, labels and all, running
/// `image`, and then left: started by an invoker on a runtime of its own, which goes, the container's watch with it, as
/// soon as the start returns.
fn left_by_a_stopped_process(stack: &RunStack<HttpWps>, reserved: &Reserved, image: &str) {
    let setup = stack.gate.app.runs.container_setup().expect("a container setup").clone();
    let invoker = ContainerInvoker::new(stack.gate.app.clone(), setup);
    let request = request_of(reserved, image, stack.gate.app.runs.loop_settings().studio);
    thread::spawn(move || {
        let runtime = Builder::new_current_thread().enable_all().build().expect("the stopped process's runtime");
        runtime.block_on(invoker.start(request)).expect("its container started");
        runtime.shutdown_background();
    })
    .join()
    .expect("the stopped process");
}

/// Statements 1 and 16's restart: attempts a stopped API process reserved keep their seats across the restart, and
/// the next process's reconciler, past the creation grace, creates the Job of one whose container never existed again
/// under a fresh token, and ends one whose container, started through the production path, already exited before it
/// read its inputs, as the engine's exited container says.
#[tokio::test]
async fn after_a_restart_a_reserved_attempt_starts_again_or_ends_as_its_exited_container_says() {
    let Some(needed) = needed() else { return };
    let scratch = scratch_or_skip!();
    let runner = &needed.image;
    let local = local(&scratch.pool, &needed, runner, &[(HOLD_VAR, "20000")], 5).await;
    let stack = &local.stack;
    let runs = submitted(stack, "Peak hour", 2).await;
    let _removed = RemovedAtEnd::of(&needed.engine, &runs);
    let store = PgRunStore::new(stack.gate.app.clone());
    let stopped = Creator::mint();
    let never = store.reserve(Seats(5), &stopped).await.expect("a reservation").expect("the first run waits");
    let died = store.reserve(Seats(5), &stopped).await.expect("a reservation").expect("the second run waits");
    // The second's container ran and exited before the stopped process could watch it.
    left_by_a_stopped_process(stack, &died, &script(&needed.engine, "exit 0"));
    until("its container exited", || async {
        let state =
            StdCommand::new(&needed.engine).args(["inspect", "-f", "{{.State.Status}}", died.job.as_str()]).output();
        state.is_ok_and(|state| String::from_utf8_lossy(&state.stdout).trim() == "exited")
    })
    .await;
    let running = loops(stack).await;
    until("the never-made Job to run", || state_is(stack, &never.key.run, "running")).await;
    let fresh: String = sqlx::query_scalar("SELECT token_sha256 FROM runner_tokens WHERE run = $1")
        .bind(never.key.run.as_str())
        .fetch_one(&scratch.pool)
        .await
        .unwrap();
    assert_ne!(fresh, never.token.sha256(), "the never-made Job runs under a fresh token");
    until("the exited container's attempt to end", || state_is(stack, &died.key.run, "failed")).await;
    let read = run_of(stack, &died.key.run).await;
    assert_eq!(read["ended_by"], "platform", "{read}");
    assert_eq!([never.key.run.clone(), died.key.run.clone()].len(), runs.len());
    for run in &runs {
        cancel(stack, run).await;
    }
    sleep(Duration::from_millis(500)).await;
    drop(running);
    scratch.close().await;
}

/// A container that keeps no deadline in its labels, as only a build older than those labels starts one, cannot be held
/// to one, and does not stop the next API process from starting: adoption ends its attempt in words that say why, keeps
/// what its runner printed and removes the container, and adopts the rest. The site is the mock world-proto-server's
/// and the containers, as a stopped process left them, are the test's own.
#[tokio::test]
async fn a_container_kept_without_its_deadline_ends_its_attempt_and_the_rest_are_adopted() {
    let Some(engine) = engine() else { return };
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, RunConfig::local("http://127.0.0.1:9", "studio-simrunner:dev")).await;
    let made = experiment(&stack, &scenario(&stack, "Peak hour").await, 1, &[100, 150]).await;
    let (plan, hash) = plan(&stack, &made.id).await;
    submit(&stack, &plan, &hash).await;
    let store = PgRunStore::new(stack.gate.app.clone());
    let unlabelled = store.reserve(Seats(5), &Creator::mint()).await.unwrap().expect("the first run waits");
    let labelled = store.reserve(Seats(5), &Creator::mint()).await.unwrap().expect("the second run waits");
    left_container(&engine, unlabelled.job.as_str(), None, "echo 'an older build ran here'; sleep 600");
    left_container(&engine, labelled.job.as_str(), Some((Utc::now().timestamp() + 3_600, 3_600)), "sleep 600");
    let out = env::temp_dir().join(format!("studio-runner-out-{}", mint_ulid()));
    fs::create_dir_all(&out).expect("the runners' output folder");
    let setup = setup(&engine, support::runstore_folder(), out.clone(), &[]);
    let invoker = ContainerInvoker::new(stack.gate.app.clone(), Arc::new(setup));
    let adopted = invoker.adopt().await.expect("adoption goes on past a container that keeps no deadline");
    assert_eq!(adopted, from_ref(&labelled.job), "the rest are adopted");
    let ending: (String, String, String) =
        sqlx::query_as("SELECT state, ended_by, stop_reason FROM attempts WHERE run_id = $1")
            .bind(unlabelled.key.run.as_str())
            .fetch_one(&scratch.pool)
            .await
            .expect("its ending");
    assert_eq!(
        (ending.0.as_str(), ending.1.as_str(), ending.2.as_str()),
        (
            "failed",
            "platform",
            "its container was found without its deadline, so it was removed rather than left to run \
         unbounded"
        )
    );
    until("its container removed", || async { !container_exists(&engine, unlabelled.job.as_str()) }).await;
    let log = fs::read_to_string(out.join(format!("{}.log", unlabelled.job))).expect("what its runner printed, kept");
    assert!(log.contains("an older build ran here"), "{log:?}");
    assert!(container_exists(&engine, labelled.job.as_str()), "the adopted one runs on");
    let _ = StdCommand::new(&engine).args(["rm", "-f", labelled.job.as_str()]).output();
    scratch.close().await;
}

/// What a runner printed outlives an API restart in the middle of keeping it: an API process whose engine refused the
/// exited container's `logs` stopped before asking again, the runner's exit settled and the attempt's seat freed.
/// The next process adopts nothing, since no attempt holds a seat, and keeps what the runner printed and removes the
/// container all the same. The first process runs on a runtime of its own, over a pool of its own, which go with it.
#[tokio::test]
async fn a_clean_up_a_stopped_api_process_left_unfinished_is_finished_by_the_next() {
    let Some(engine) = engine() else { return };
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, RunConfig::local("http://127.0.0.1:9", "studio-simrunner:dev")).await;
    let made = experiment(&stack, &scenario(&stack, "Peak hour").await, 1, &[]).await;
    let (plan, hash) = plan(&stack, &made.id).await;
    submit(&stack, &plan, &hash).await;
    let store = PgRunStore::new(stack.gate.app.clone());
    let reserved = store.reserve(Seats(5), &Creator::mint()).await.unwrap().expect("a waiting attempt");
    let _removed = RemovedAtEnd::of(&engine, [&reserved.key.run]);
    let job = reserved.job.as_str().to_owned();
    let refused = env::temp_dir().join(format!("studio-restart-clean-up-{}", mint_ulid()));
    let (objects, out) =
        (support::runstore_folder(), env::temp_dir().join(format!("studio-runner-out-{}", mint_ulid())));
    let image = script(&engine, "echo 'said before its API process stopped'; exit 0");
    let schema: String = sqlx::query_scalar("SELECT current_schema()").fetch_one(&scratch.pool).await.unwrap();
    let first = setup(&refusing_engine(&engine, &refused, &["logs"]), objects.clone(), out.clone(), &[]);
    let request = request_of(&reserved, &image, Endpoint { endpoint: "http://127.0.0.1:9".to_owned() });
    thread::spawn(move || {
        let runtime = Builder::new_multi_thread().enable_all().build().expect("the first process's runtime");
        runtime.block_on(async {
            let objects = RunObjects::local_folder(&objects).expect("the run store");
            let pool = support::schema_pool(&schema).await;
            let rules = support::gate_rules();
            let app =
                AppState::new(pool, rules, Deployment::Local, support::MODEL_LABEL, Arc::new(BlockingCompute), objects);
            ContainerInvoker::new(app, Arc::new(first)).start(request).await.expect("its container started");
            until("its engine to refuse the exited container's log", || async { refused.join("logs").exists() }).await;
        });
        runtime.shutdown_background();
    })
    .join()
    .expect("the first process");
    assert!(store.attempt(&reserved.key).await.unwrap().is_some_and(|row| row.state.ended()), "its exit settled");
    // Its reconciler freed the seat once the exit was settled, which it does without waiting for the log.
    store.job_stopped(&reserved.key).await.expect("the seat freed");
    assert!(container_exists(&engine, &job), "the stopped process left its container");
    let next = ContainerInvoker::new(
        stack.gate.app.clone(),
        Arc::new(setup(&engine, support::runstore_folder(), out.clone(), &[])),
    );
    assert_eq!(next.adopt().await.expect("the next process adopts"), [], "no attempt holds a seat");
    until("its container removed", || async { !container_exists(&engine, &job) }).await;
    let log = fs::read_to_string(out.join(format!("{job}.log"))).expect("what its runner printed, kept");
    assert!(log.contains("said before its API process stopped"), "{log:?}");
    scratch.close().await;
}
