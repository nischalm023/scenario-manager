//! Spec 08 section 8's statement 16 on the local invoker, a container of the runner image per attempt, and simrunner,
//! a run's life in its container: a real run whose manifest writes `running`, whose heartbeat and the platform's words
//! are on its row, and whose own report writes the ending, metrics and series, the recording it sent stored; a runner
//! that exits without reporting, settled from its folder; the token in the runner's environment and never on its
//! command line, and an exit with no ending; a file the runner left that does not read; two starts of one Job racing,
//! which start one container; a wait the engine could not answer, which is no exit; and what a runner printed, kept
//! once its container goes, its container kept until then. What the tests need is in `support::containers`; any of it
//! missing, they say they are skipped.

mod support;

use std::env;
use std::fs;
use std::process::Command as StdCommand;
use std::sync::Arc;
use std::time::Duration;

use run_invocation::Invoker;
use runstore::layout::{AttemptNumber as StoredAttempt, RunId, recording_key};
use runstore::{RunStorage, RunStore as RunObjects};
use serde_json::Value;
use sha2::{Digest, Sha256};
use studio_api::AppState;
use studio_api::config::Deployment;
use studio_api::db::mint_ulid;
use studio_api::runs::ContainerInvoker;
use tokio::time::sleep;
use wps_client::BlockingCompute;

use support::containers::{
    RemovedAtEnd, SIM_MINUTES, container_exists, engine, local, loops, needed, out_of, refusing_engine, request_alone,
    run_of, script, setup, state_is, submitted,
};
use support::results::{CANNED_METRICS, CANNED_RECORDING, CANNED_SERIES};
use support::runs::until;

/// The image's folders, the Dockerfile's: where a script finds its output folder as `$4`.
const OUT_IN_CONTAINER: &str = "/var/lib/simrunner/out";

/// Statement 16 and statement 8's first-cut half: the runner's manifest writes `running`; its own report writes the
/// ending, `finished` by the simulator with its stop reason, no stub sentence and no finding, the metrics and series
/// rows and the build it ran, which only a report names; the recording it sent stored, the very file it left in its
/// folder on the host; its heartbeat and the platform's words are on the row; the Job is then known stopped, its
/// container gone; and the site's conversation holds one `run.status` per transition.
#[tokio::test]
async fn a_run_writes_running_on_its_manifest_and_reports_its_results_at_its_end() {
    let Some(needed) = needed() else { return };
    let scratch = scratch_or_skip!();
    let local = local(&scratch.pool, &needed, &needed.image, &[], 5).await;
    let stack = &local.stack;
    let running = loops(stack).await;
    let runs = submitted(stack, "Peak hour", 1).await;
    let _removed = RemovedAtEnd::of(&needed.engine, &runs);
    let run = &runs[0];
    until("the run to end", || state_is(stack, run, "finished")).await;
    let read = run_of(stack, run).await;
    assert_eq!((read["ended_by"].as_str(), &read["stop_detail"]), (Some("simulator"), &Value::Null), "{read}");
    assert_eq!(read["findings"], serde_json::json!([]), "no plan-differs");
    assert_eq!(read["attempts"][0]["stub"], false, "the manifest says a simulation ran");
    assert_eq!(read["recording"]["stored"], true, "the runner sent its recording and the API stored it: {read}");
    let left = fs::read(out_of(stack, run).join("recording.worldview")).expect("the recording in its folder");
    let key = recording_key(&RunId::new(run.as_str()).unwrap(), StoredAttempt::new(1).unwrap());
    let stored = stack.gate.app.runstore.get(&key).await.expect("the stored recording");
    assert!(stored.as_ref() == left.as_slice(), "the stored recording is the one the runner wrote");
    let attempt = &read["attempts"][0];
    assert!(attempt["runner_commit"].is_string(), "the runner's own report, which alone names its build: {attempt}");
    assert!(attempt["heartbeat_at"].is_string(), "the runner beat while it ran: {attempt}");
    assert!(attempt["platform_status"].is_string(), "the reconciler kept the engine's words: {attempt}");
    assert_eq!(attempt["stalled"], false);
    assert!(read["progress"]["sim_ms"].as_u64().unwrap() >= SIM_MINUTES * 60_000, "{read}");
    let path: Vec<(Option<String>, String)> =
        sqlx::query_as("SELECT \"from\", \"to\" FROM attempt_transitions ORDER BY tx, id")
            .fetch_all(&scratch.pool)
            .await
            .expect("the transitions");
    let path: Vec<String> = path.into_iter().map(|(_, to)| to).collect();
    assert_eq!(path, ["queued", "running", "finished"], "the manifest wrote running; the report, the ending");
    let (metrics, series): (i64, i64) =
        sqlx::query_as("SELECT (SELECT count(*) FROM attempt_metrics), (SELECT count(*) FROM attempt_series)")
            .fetch_one(&scratch.pool)
            .await
            .expect("the rows");
    assert!(metrics > 0 && metrics == series, "{metrics} metrics, {series} series");
    until("the Job known stopped", || async {
        sqlx::query_scalar::<_, bool>("SELECT job_stopped_at IS NOT NULL FROM attempts")
            .fetch_one(&scratch.pool)
            .await
            .unwrap()
    })
    .await;
    let job = read["attempts"][0]["job"].as_str().unwrap().to_owned();
    until("the container removed", || async { !container_exists(&needed.engine, &job) }).await;
    until("every transition journaled", || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM journal WHERE kind = 'run.status'")
            .fetch_one(&scratch.pool)
            .await
            .unwrap()
            == 3
    })
    .await;
    let finished: Value = sqlx::query_scalar("SELECT body FROM journal WHERE dedup_key LIKE '%:finished'")
        .fetch_one(&scratch.pool)
        .await
        .expect("the finished entry");
    assert_eq!((finished["ended_by"].as_str(), &finished["stop_detail"]), (Some("simulator"), &Value::Null));
    drop(running);
    scratch.close().await;
}

/// The local fallback, for a runner that exits without reporting: what it left in its folder, a finished ending, the
/// metrics, the series and the recording, is settled as the attempt's ending once its container exited, the recording
/// taken as one the runner sent would be; the row names no runner build, which only a report does. The runner is a
/// script that waits for the test to leave the canned results in its folder and exits without a word.
#[tokio::test]
async fn a_runner_that_exits_without_reporting_is_settled_from_its_folder() {
    let Some(needed) = needed() else { return };
    let scratch = scratch_or_skip!();
    let ending =
        r#"{"state":"finished","ended_by":"simulator","stop_reason":"SimTimeLimit","stop_detail":null,"sim_ms":1}"#;
    let runner = script(
        &needed.engine,
        &format!(
            "out=\"$4\"; while [ ! -e \"$out/go\" ]; do sleep 0.1; done; printf '%s' '{ending}' > \"$out/ending.json\""
        ),
    );
    let local = local(&scratch.pool, &needed, &runner, &[], 5).await;
    let stack = &local.stack;
    let running = loops(stack).await;
    let runs = submitted(stack, "Peak hour", 1).await;
    let _removed = RemovedAtEnd::of(&needed.engine, &runs);
    let out = out_of(stack, &runs[0]);
    until("the runner's folder", || async { out.exists() }).await;
    sqlx::query("UPDATE attempts SET state = 'running', started_at = now()").execute(&scratch.pool).await.unwrap();
    fs::write(out.join("metrics.json"), CANNED_METRICS).expect("the metrics left");
    fs::write(out.join("series.json"), CANNED_SERIES).expect("the series left");
    fs::write(out.join("recording.worldview"), CANNED_RECORDING).expect("the recording left");
    fs::write(out.join("go"), "").expect("the runner told to go on");
    until("the attempt settled", || state_is(stack, &runs[0], "finished")).await;
    let read = run_of(stack, &runs[0]).await;
    assert_eq!(read["recording"]["stored"], true, "the folder's recording taken: {read}");
    assert_eq!(read["attempts"][0]["runner_commit"], Value::Null, "no report named a build");
    let (reported, metrics): (bool, i64) =
        sqlx::query_as("SELECT (SELECT reported_at IS NOT NULL FROM attempts), (SELECT count(*) FROM attempt_metrics)")
            .fetch_one(&scratch.pool)
            .await
            .unwrap();
    assert!(!reported && metrics > 0, "settled from the folder, its metrics stored: {reported} {metrics}");
    drop(running);
    scratch.close().await;
}

/// Statement 16: the runner's token rides in its environment and never on its command line, which names only its job
/// spec and its output folder; the environment carries no database credential, and the executor thread count the
/// invoker gives from the run's two cores; an exit with no `ending.json` ends the attempt `failed`, by the platform,
/// naming the exit status.
#[tokio::test]
async fn a_runner_gets_its_token_in_its_environment_and_an_exit_with_no_ending_fails() {
    let Some(needed) = needed() else { return };
    let scratch = scratch_or_skip!();
    let runner = script(
        &needed.engine,
        "out=\"$4\"; mkdir -p \"$out\"; printf '%s\\n' \"$@\" > \"$out/argv\"; env > \"$out/env\"; exit 3",
    );
    let local = local(&scratch.pool, &needed, &runner, &[], 5).await;
    let stack = &local.stack;
    let running = loops(stack).await;
    let runs = submitted(stack, "Peak hour", 1).await;
    let _removed = RemovedAtEnd::of(&needed.engine, &runs);
    until("the runner's exit to end the attempt", || state_is(stack, &runs[0], "failed")).await;
    let read = run_of(stack, &runs[0]).await;
    assert_eq!(read["ended_by"], "platform");
    assert_eq!(read["stop_reason"], "the runner exited with status 3 and left no ending");
    let out = out_of(stack, &runs[0]);
    let argv = fs::read_to_string(out.join("argv")).expect("the runner wrote its arguments");
    let environment = fs::read_to_string(out.join("env")).expect("the runner wrote its environment");
    let token = environment
        .lines()
        .find_map(|line| line.strip_prefix("STUDIO_RUNNER_TOKEN="))
        .expect("the token is in the environment");
    let stored: String =
        sqlx::query_scalar("SELECT token_sha256 FROM runner_tokens").fetch_one(&scratch.pool).await.unwrap();
    assert_eq!(stored, format!("{:x}", Sha256::digest(token.as_bytes())), "it is the attempt's token");
    assert!(!argv.contains(token), "no token on the command line: {argv}");
    assert_eq!(argv.lines().filter(|line| line.starts_with("--")).collect::<Vec<_>>(), ["--job", "--out"]);
    assert!(argv.lines().any(|line| line == OUT_IN_CONTAINER), "{argv}");
    assert!(!environment.contains("STUDIO_DATABASE_URL"), "the runner never reads Postgres");
    assert!(environment.lines().any(|line| line == "STUDIO_RUNNER_THREADS=2"), "two cores give two threads");
    drop(running);
    scratch.close().await;
}

/// Spec 08 section 3.4's creation, idempotent by name: two starts of one Job racing, as a creator's create and a late
/// second ask would, start one container. Neither the database nor world-proto-server is needed: the runner is a
/// script that counts its starts in its mounted folder.
#[tokio::test]
async fn two_starts_of_one_job_racing_start_one_container() {
    let Some(engine) = engine() else { return };
    let runner = script(&engine, "echo started >> \"$4/starts\"; sleep 1");
    let objects = support::runstore_folder();
    let out = env::temp_dir().join(format!("studio-runner-out-{}", mint_ulid()));
    let setup = setup(&engine, objects.clone(), out.clone(), &[]);
    let app = AppState::new(
        support::unconnected_pool(),
        support::gate_rules(),
        Deployment::Local,
        support::MODEL_LABEL,
        Arc::new(BlockingCompute),
        RunObjects::local_folder(&objects).expect("the run store"),
    );
    let invoker = ContainerInvoker::new(app, Arc::new(setup));
    let request = request_alone(&runner);
    let (first, second) = tokio::join!(invoker.start(request.clone()), invoker.start(request.clone()));
    first.expect("a start");
    second.expect("a second start answers as the first");
    let counter = out.join(request.name.as_str()).join("starts");
    until("the runner to have started", || async { counter.exists() }).await;
    sleep(Duration::from_millis(1500)).await;
    let starts = fs::read_to_string(&counter).expect("the runner counted its starts");
    assert_eq!(starts.lines().count(), 1, "one container: {starts:?}");
    let _ = StdCommand::new(&engine).args(["rm", "-f", request.name.as_str()]).output();
}

/// What a runner printed outlives its container, as it outlived the process invoker's runners: once the container
/// exited and went, both streams are kept in `{job}.log` beside its folder. An attempt no row holds settles as already
/// ended, so no run is needed.
#[tokio::test]
async fn a_removed_container_leaves_what_its_runner_printed() {
    let Some(engine) = engine() else { return };
    let scratch = scratch_or_skip!();
    let objects = support::runstore_folder();
    let out = env::temp_dir().join(format!("studio-runner-out-{}", mint_ulid()));
    let runner = script(&engine, "echo 'said on stdout'; echo 'said on stderr' >&2; exit 2");
    let setup = setup(&engine, objects.clone(), out.clone(), &[]);
    let app = AppState::new(
        scratch.pool.clone(),
        support::gate_rules(),
        Deployment::Local,
        support::MODEL_LABEL,
        Arc::new(BlockingCompute),
        RunObjects::local_folder(&objects).expect("the run store"),
    );
    let invoker = ContainerInvoker::new(app, Arc::new(setup));
    let request = request_alone(&runner);
    let name = request.name.as_str().to_owned();
    invoker.start(request).await.expect("a start");
    until("the container removed", || async { !container_exists(&engine, &name) }).await;
    let log = fs::read_to_string(out.join(format!("{name}.log"))).expect("the runner's log kept");
    assert!(log.contains("said on stdout") && log.contains("said on stderr"), "{log:?}");
    scratch.close().await;
}

/// A `docker wait` the engine could not answer is no exit: the engine here stands in front of the real one and refuses
/// the first wait as an unreachable daemon does, so the invoker must ask again, and settle and remove the container
/// only once it really exited. The run's rows are not needed: an attempt no row holds settles as already ended.
#[tokio::test]
async fn a_wait_the_engine_could_not_answer_is_no_exit() {
    let Some(engine) = engine() else { return };
    let scratch = scratch_or_skip!();
    let folder = env::temp_dir().join(format!("studio-wait-{}", mint_ulid()));
    let stand_in = refusing_engine(&engine, &folder, &["wait"]);
    let objects = support::runstore_folder();
    let out = env::temp_dir().join(format!("studio-runner-out-{}", mint_ulid()));
    let setup = setup(&stand_in, objects.clone(), out, &[]);
    let app = AppState::new(
        scratch.pool.clone(),
        support::gate_rules(),
        Deployment::Local,
        support::MODEL_LABEL,
        Arc::new(BlockingCompute),
        RunObjects::local_folder(&objects).expect("the run store"),
    );
    let invoker = ContainerInvoker::new(app, Arc::new(setup));
    let request = request_alone(&script(&engine, "sleep 4"));
    let name = request.name.as_str().to_owned();
    invoker.start(request).await.expect("a start");
    until("the first wait refused", || async { folder.join("wait").exists() }).await;
    sleep(Duration::from_millis(1500)).await;
    assert!(container_exists(&engine, &name), "a container that still runs is neither settled nor removed");
    until("the container removed once it exited", || async { !container_exists(&engine, &name) }).await;
    scratch.close().await;
}

/// What a runner printed is its container's only copy until it is kept: a `logs` the engine refused leaves the
/// container in place and is asked again, and so is a removal the engine refused, so the log is kept and the container
/// goes all the same. An attempt no row holds settles as already ended, so no run is needed.
#[tokio::test]
async fn a_log_and_a_removal_the_engine_refused_are_asked_again_the_container_kept_until_its_log_is() {
    let Some(engine) = engine() else { return };
    let scratch = scratch_or_skip!();
    let folder = env::temp_dir().join(format!("studio-clean-up-{}", mint_ulid()));
    let stand_in = refusing_engine(&engine, &folder, &["logs", "rm"]);
    let objects = support::runstore_folder();
    let out = env::temp_dir().join(format!("studio-runner-out-{}", mint_ulid()));
    let setup = setup(&stand_in, objects.clone(), out.clone(), &[]);
    let app = AppState::new(
        scratch.pool.clone(),
        support::gate_rules(),
        Deployment::Local,
        support::MODEL_LABEL,
        Arc::new(BlockingCompute),
        RunObjects::local_folder(&objects).expect("the run store"),
    );
    let invoker = ContainerInvoker::new(app, Arc::new(setup));
    let request = request_alone(&script(&engine, "echo 'said before it went'; exit 0"));
    let name = request.name.as_str().to_owned();
    invoker.start(request).await.expect("a start");
    until("the first logs refused", || async { folder.join("logs").exists() }).await;
    sleep(Duration::from_millis(300)).await;
    assert!(container_exists(&engine, &name), "the container stays until what its runner printed is kept");
    until("the container removed", || async { !container_exists(&engine, &name) }).await;
    assert!(folder.join("rm").exists(), "the first removal was refused, and asked again");
    let log = fs::read_to_string(out.join(format!("{name}.log"))).expect("the runner's log kept");
    assert!(log.contains("said before it went"), "{log:?}");
    scratch.close().await;
}

/// Spec 08 section 3.13: a runner that leaves a file that does not read ends its attempt `failed`, by the platform,
/// naming the file, never `partial` as a lost runner would. The runner is a script that waits for the test to stand
/// in for its manifest, then leaves a finished ending beside a `metrics.json` that is no JSON.
#[tokio::test]
async fn a_file_the_runner_left_that_does_not_read_fails_the_attempt_naming_it() {
    let Some(needed) = needed() else { return };
    let scratch = scratch_or_skip!();
    let ending =
        r#"{"state":"finished","ended_by":"simulator","stop_reason":"SimTimeLimit","stop_detail":null,"sim_ms":1}"#;
    let runner = script(
        &needed.engine,
        &format!(
            "out=\"$4\"; while [ ! -e \"$out/go\" ]; do sleep 0.1; done; printf '%s' '{ending}' > \"$out/ending.json\"; \
             printf 'not json' > \"$out/metrics.json\""
        ),
    );
    let local = local(&scratch.pool, &needed, &runner, &[], 5).await;
    let stack = &local.stack;
    let running = loops(stack).await;
    let runs = submitted(stack, "Peak hour", 1).await;
    let _removed = RemovedAtEnd::of(&needed.engine, &runs);
    until("the runner to start", || async {
        state_is(stack, &runs[0], "queued").await && !run_of(stack, &runs[0]).await["attempts"][0]["job"].is_null()
    })
    .await;
    sqlx::query("UPDATE attempts SET state = 'running', started_at = now()").execute(&scratch.pool).await.unwrap();
    let out = out_of(stack, &runs[0]);
    until("the runner's folder", || async { out.exists() }).await;
    fs::write(out.join("go"), "").expect("the runner told to go on");
    until("the attempt to end", || async { !state_is(stack, &runs[0], "running").await }).await;
    let read = run_of(stack, &runs[0]).await;
    assert_eq!((read["state"].as_str(), read["ended_by"].as_str()), (Some("failed"), Some("platform")), "{read}");
    let reason = read["stop_reason"].as_str().expect("its words");
    assert!(reason.starts_with("the runner's metrics.json does not read"), "{reason}");
    drop(running);
    scratch.close().await;
}
