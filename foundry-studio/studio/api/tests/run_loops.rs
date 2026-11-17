//! Spec 08 section 8's statements 1 and 6, and spec 01's 9 and 16's racing run entry, against the database: the run
//! loops of `run-invocation` over the API's store and journal with an invoker that only records what it is asked. At
//! most five seats through twelve runs, started first approved first started; the Job of an attempt its runner
//! reported is left to exit by itself, for as long as its runner may still be delivering, and stopped only past the
//! grace; two endings racing leave one; a watcher that repeats itself writes nothing twice; and a run's entry racing a
//! Dismiss lands in exactly one conversation. A Job's creation racing the rest is `run_loops_creation`'s.

mod support;

use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};

use run_invocation::{Cursor, Ended, JobName, JobRequest, JobStatus, LoopSettings, RunEvents, RunStore};
use serde_json::Value;
use sqlx::{Connection, PgConnection, PgPool};
use studio_api::runs::{JournalEvents, PgRunStore};
use studio_protocol::{DELIVERY_BUDGET, EndedBy, Ending, Endpoint, RunState, Ulid};
use tokio::time::sleep;

use support::loops::{ending, held, key, loops, loops_told, reported, short_waits, submitted};
use support::runs::{FakeInvoker, config, stack, until};

/// Statement 1 and spec 01 statement 9: with the limit at 5, twelve runs never hold more than five seats at any
/// observed instant and start in approval order, which within a submission is position order; each seat frees only
/// once its Job is known stopped.
#[tokio::test]
async fn twelve_runs_hold_at_most_five_seats_and_start_first_approved_first_started() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let runs = submitted(&stack, "Peak hour", 12).await;
    let invoker = Arc::new(FakeInvoker::default());
    let most = Arc::new(AtomicI64::new(0));
    let watching = {
        let (pool, most) = (scratch.pool.clone(), most.clone());
        tokio::spawn(async move {
            loop {
                most.fetch_max(held(&pool).await, Ordering::SeqCst);
                sleep(Duration::from_millis(5)).await;
            }
        })
    };
    let running = loops(&stack, invoker.clone());
    let started = |count: usize| {
        let invoker = invoker.clone();
        move || {
            let invoker = invoker.clone();
            async move { invoker.started().len() >= count }
        }
    };
    until("five starts", started(5)).await;
    sleep(Duration::from_millis(300)).await;
    assert_eq!(invoker.started().len(), 5, "a sixth waits for a seat");
    let store = PgRunStore::new(stack.gate.app.clone());
    // Each run that ends frees its seat only once its Job is known stopped; the next waiting one then starts.
    for (n, run) in runs.iter().enumerate() {
        store.end(&key(run), &ending(RunState::Finished)).await.expect("an ending");
        let name = JobName::of(&key(run));
        if n + 5 < runs.len() {
            sleep(Duration::from_millis(150)).await;
            assert_eq!(invoker.started().len(), n + 5, "run {} ended but its Job still runs: its seat is held", n + 1);
        }
        invoker.set(&name, JobStatus::Ended { reason: None });
        until("the seat to free and the next run to start", started((n + 6).min(runs.len()))).await;
    }
    watching.abort();
    assert!(most.load(Ordering::SeqCst) <= 5, "at most five seats held, {} seen", most.load(Ordering::SeqCst));
    let order: Vec<String> = invoker.started().iter().map(|request| request.spec.run.to_string()).collect();
    let expected: Vec<String> = runs.iter().map(ToString::to_string).collect();
    assert_eq!(order, expected, "first approved, first started, in position order");
    drop(running);
    scratch.close().await;
}

/// Ruling D174: with only stale streaming servers registered, an attempt starts unstreamed and its row says so; once a
/// server's heartbeat is fresh by the database's clock, the dispatcher points the next attempt at it, in the job spec
/// and on the attempt's row, and never at a stale one. Each phase waits for the start of its own run, since the second
/// phase's loops may start the first run's attempt again before it.
#[tokio::test]
async fn an_attempt_is_pointed_at_a_fresh_streaming_server_or_starts_unstreamed() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let register = |id: &'static str, host: &'static str, age: &'static str| {
        let pool = scratch.pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO streaming_servers (id, runner_url, viewer_url, heartbeat_at) \
                 VALUES ($1, 'ws://' || $2 || '/runners', 'ws://' || $2 || '/streams', now() - $3::interval)",
            )
            .bind(id)
            .bind(host)
            .bind(age)
            .execute(&pool)
            .await
            .expect("a streaming server registered");
        }
    };
    let recorded = |run: Ulid| {
        let pool = scratch.pool.clone();
        async move {
            sqlx::query_scalar::<_, Option<String>>("SELECT streaming_server FROM attempts WHERE run_id = $1")
                .bind(run.as_str())
                .fetch_one(&pool)
                .await
                .expect("the attempt's row")
        }
    };
    register("01KZAVPQNES65HV4EZADPCS7E9", "dead.example:3201", "1 hour").await;
    register("01KZAVPQNES65HV4EZADPCS7EA", "stale.example:3201", "11 seconds").await;
    let runs = submitted(&stack, "Peak hour", 1).await;
    let invoker = Arc::new(FakeInvoker::default());
    let running = loops(&stack, invoker.clone());
    let spec = start_of(&invoker, &runs[0]).await.spec;
    assert_eq!(spec.streaming, None, "no fresh server: the attempt runs unstreamed");
    assert_eq!(recorded(runs[0].clone()).await, None);
    drop(running);
    let more = submitted(&stack, "Off hour", 1).await;
    let fresh: Ulid = "01KZAVPQNES65HV4EZADPCS7E8".parse().unwrap();
    register("01KZAVPQNES65HV4EZADPCS7E8", "alive.example:3201", "0 seconds").await;
    let invoker = Arc::new(FakeInvoker::default());
    let running = loops(&stack, invoker.clone());
    let spec = start_of(&invoker, &more[0]).await.spec;
    let streaming = spec.streaming.expect("pointed at a streaming server");
    assert_eq!((&streaming.server, streaming.endpoint.as_str()), (&fresh, "ws://alive.example:3201/runners"));
    assert_eq!(recorded(more[0].clone()).await.as_deref(), Some(fresh.as_str()), "the pick is on the attempt's row");
    drop(running);
    scratch.close().await;
}

/// The first start `invoker` was asked for of `run`'s attempt, once it comes.
async fn start_of(invoker: &FakeInvoker, run: &Ulid) -> JobRequest {
    until("the run's start", || async { invoker.started().iter().any(|start| start.spec.run == *run) }).await;
    invoker.started().into_iter().find(|start| start.spec.run == *run).expect("the start just seen")
}

/// A runner that reported exits by itself, which leaves its Job, and the Job's log, to the platform's keeping for as long
/// as it keeps a finished Job: the reconciler does not stop the Job of an attempt its runner's report ended, and frees its
/// seat once the Job has ended; a Job still running a second after its runner reported is stopped.
#[tokio::test]
async fn the_job_of_an_attempt_its_runner_reported_is_left_to_exit_by_itself() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let runs = submitted(&stack, "Peak hour", 2).await;
    let invoker = Arc::new(FakeInvoker::default());
    let running = loops(&stack, invoker.clone());
    until("both started", || async { invoker.started().len() == 2 }).await;
    let store = PgRunStore::new(stack.gate.app.clone());
    let (first, second) = (JobName::of(&key(&runs[0])), JobName::of(&key(&runs[1])));
    store.take_results(&key(&runs[0]), reported(RunState::Finished)).await.expect("the first runner's report");
    sleep(Duration::from_millis(400)).await;
    assert!(!invoker.stopped().contains(&first), "a reported Job exits by itself");
    assert_eq!(held(&scratch.pool).await, 2, "its seat is held until it has");
    invoker.set(&first, JobStatus::Ended { reason: None });
    until("its seat freed once it ended", || async { held(&scratch.pool).await == 1 }).await;
    assert!(!invoker.stopped().contains(&first), "and it was never stopped");
    store.take_results(&key(&runs[1]), reported(RunState::Cancelled)).await.expect("the second runner's report");
    until("a reported Job still running past the grace stopped", || async { invoker.stopped().contains(&second) })
        .await;
    drop(running);
    scratch.close().await;
}

/// A runner whose report the API took, but whose answer never reached it, asks again until its delivery budget is
/// spent, and its Job and the Job's log outlast those tries: at the deployment's exit grace, the reconciler leaves the
/// Job running for the whole of what remains of that budget after the report was taken, and stops it only once the
/// budget and the minute to exit have passed.
#[tokio::test]
async fn a_report_taken_whose_answer_never_arrived_keeps_its_job_while_its_runner_may_ask_again() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let runs = submitted(&stack, "Peak hour", 1).await;
    let invoker = Arc::new(FakeInvoker::default());
    let mut settings = short_waits();
    let deployed = LoopSettings::new(Endpoint { endpoint: "http://127.0.0.1:3101".to_owned() });
    settings.exit_grace = deployed.exit_grace;
    let store = Arc::new(PgRunStore::new(stack.gate.app.clone()));
    let running = loops_told(&stack, store.clone(), invoker.clone(), settings);
    until("started", || async { invoker.started().len() == 1 }).await;
    let job = JobName::of(&key(&runs[0]));
    store
        .take_results(&key(&runs[0]), reported(RunState::Finished))
        .await
        .expect("the report taken, its answer then lost");
    let taken_ago = |ago: Duration| {
        let (pool, wakes) = (scratch.pool.clone(), stack.gate.app.runs.wakes.clone());
        async move {
            sqlx::query("UPDATE attempts SET reported_at = now() - make_interval(secs => $1)")
                .bind(ago.as_secs_f64())
                .execute(&pool)
                .await
                .expect("the report's instant moved back");
            wakes.reconcile();
            sleep(Duration::from_millis(300)).await;
        }
    };
    // Taken at the very start of the runner's first try, the report may still be asked again a second before the
    // runner's budget runs out.
    taken_ago(DELIVERY_BUDGET - Duration::from_secs(1)).await;
    assert!(!invoker.stopped().contains(&job), "a runner that may still ask again keeps its Job");
    taken_ago(deployed.exit_grace + Duration::from_secs(1)).await;
    until("the Job stopped once the budget and the minute to exit have passed", || async {
        invoker.stopped().contains(&job)
    })
    .await;
    drop(running);
    scratch.close().await;
}

/// Statement 6: two writers racing to end one attempt leave one ending and one transition into it.
#[tokio::test]
async fn two_endings_racing_leave_one_ending_and_one_transition() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let runs = submitted(&stack, "Peak hour", 1).await;
    let store = PgRunStore::new(stack.gate.app.clone());
    let (first, second) = (store.clone(), store.clone());
    let attempt = key(&runs[0]);
    let (finished, lost) = (
        ending(RunState::Finished),
        Ending { stop_reason: "lost".to_owned(), ended_by: EndedBy::Platform, ..ending(RunState::Partial) },
    );
    let (a, b) = tokio::join!(first.end(&attempt, &finished), second.end(&attempt, &lost));
    let mut outcomes = [a.expect("one ending"), b.expect("the other")];
    outcomes.sort_by_key(|outcome| *outcome == Ended::AlreadyEnded);
    assert_eq!(outcomes, [Ended::Wrote, Ended::AlreadyEnded]);
    let ends: i64 =
        sqlx::query_scalar("SELECT count(*) FROM attempt_transitions WHERE \"to\" IN ('finished', 'partial')")
            .fetch_one(&scratch.pool)
            .await
            .expect("the transitions");
    assert_eq!(ends, 1, "one transition into an ending");
    let written = store.end(&attempt, &ending(RunState::Cancelled)).await.expect("a late ending");
    assert_eq!(written, Ended::AlreadyEnded, "nobody writes over the first ending");
    scratch.close().await;
}

/// The site's journal: each entry's kind, dedup key and conversation, in order.
async fn entries(pool: &PgPool) -> Vec<(String, String, String)> {
    sqlx::query_as(
        "SELECT kind, COALESCE(dedup_key, ''), conversation FROM journal WHERE kind IN ('run.status', 'submission.ended') \
         ORDER BY conversation, seq",
    )
    .fetch_all(pool)
    .await
    .expect("the entries")
}

/// Statement 6 and spec 01 statement 9: a watcher killed between journaling transitions and advancing its cursor, then
/// restarted, hands every transition on again and writes nothing twice: one `run.status` per transition, one
/// `submission.ended` per round, no attempt's state changed and the journal's numbers gapless.
#[tokio::test]
async fn a_watcher_that_repeats_itself_writes_one_entry_per_transition_and_one_end_per_round() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let runs = submitted(&stack, "Peak hour", 2).await;
    let store = PgRunStore::new(stack.gate.app.clone());
    for run in &runs {
        store.end(&key(run), &ending(RunState::Finished)).await.expect("an ending");
    }
    let events = JournalEvents::new(stack.gate.app.clone());
    let recorded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM attempt_transitions").fetch_one(&scratch.pool).await.unwrap();
    // As the watcher does from a cursor: every transition handed on, its round's end told after its last ending. A
    // transition is handed on once no transaction older than its own still runs, so this reads until it has all.
    let hand_on = || async {
        let (mut cursor, mut handed) = (Cursor::default(), 0);
        let deadline = Instant::now() + Duration::from_secs(30);
        while handed < recorded {
            assert!(Instant::now() < deadline, "{handed} of {recorded} transitions were handed on");
            let transitions = store.transitions_after(cursor).await.expect("the transitions");
            for transition in transitions {
                events.status_changed(&transition).await.expect("told");
                if transition.to.ended()
                    && let Some(round) = store.round_ended(&transition).await.expect("the round")
                {
                    events.submission_ended(&round).await.expect("told");
                }
                cursor = transition.at;
                handed += 1;
            }
            sleep(Duration::from_millis(20)).await;
        }
    };
    hand_on().await;
    let first = entries(&scratch.pool).await;
    let states: Vec<&str> = first
        .iter()
        .map(|(kind, key, _)| if kind == "run.status" { key.rsplit(':').next().unwrap() } else { "ended" })
        .collect();
    assert_eq!(states.iter().filter(|state| **state == "queued").count(), 2);
    assert_eq!(states.iter().filter(|state| **state == "finished").count(), 2);
    assert_eq!(states.iter().filter(|state| **state == "ended").count(), 1, "one end for the round");
    // The cursor never advanced: the restarted watcher hands everything on again.
    hand_on().await;
    assert_eq!(entries(&scratch.pool).await, first, "a repeat writes nothing");
    let gaps: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (SELECT seq - lag(seq) OVER (PARTITION BY conversation ORDER BY seq) AS step FROM journal) \
         steps WHERE step <> 1",
    )
    .fetch_one(&scratch.pool)
    .await
    .expect("the steps");
    assert_eq!(gaps, 0, "the journal's numbers stay gapless");
    let states: Vec<String> =
        sqlx::query_scalar("SELECT state FROM attempts").fetch_all(&scratch.pool).await.expect("states");
    assert!(states.iter().all(|state| state == "finished"), "no attempt's state changed");
    let rounds: Vec<i32> =
        sqlx::query_scalar("SELECT unnest(ended_rounds) FROM submissions").fetch_all(&scratch.pool).await.unwrap();
    assert_eq!(rounds, [2], "the round is recorded once, at its two attempts");
    scratch.close().await;
}

/// Spec 08 sections 3.6 and 7 (Astra's run-chain review, the eighth finding): when every run of a submission has ended
/// before the watcher catches up, each run's own `run.status` ending is journaled before the round's
/// `submission.ended`, which counts them as they ended.
#[tokio::test]
async fn a_rounds_end_follows_its_last_runs_own_ending_when_the_watcher_catches_up() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let runs = submitted(&stack, "Peak hour", 2).await;
    let store = PgRunStore::new(stack.gate.app.clone());
    store.end(&key(&runs[0]), &ending(RunState::Finished)).await.expect("an ending");
    let failed = Ending { stop_reason: "Watchdog".to_owned(), ..ending(RunState::Failed) };
    store.end(&key(&runs[1]), &failed).await.expect("an ending");
    let running = loops(&stack, Arc::new(FakeInvoker::default()));
    until("the round's end", || async {
        entries(&scratch.pool).await.iter().any(|(kind, _, _)| kind == "submission.ended")
    })
    .await;
    let order: Vec<String> = sqlx::query_scalar(
        "SELECT CASE WHEN kind = 'submission.ended' THEN 'ended' ELSE dedup_key END FROM journal \
         WHERE kind IN ('run.status', 'submission.ended') ORDER BY seq",
    )
    .fetch_all(&scratch.pool)
    .await
    .expect("the journal");
    assert_eq!(order.last().map(String::as_str), Some("ended"), "the round's end comes last: {order:?}");
    assert_eq!(order.iter().filter(|key| key.ends_with(":finished") || key.ends_with(":failed")).count(), 2);
    let body: Value = sqlx::query_scalar("SELECT body FROM journal WHERE kind = 'submission.ended'")
        .fetch_one(&scratch.pool)
        .await
        .expect("the end");
    assert_eq!((body["counts"]["finished"].as_u64(), body["counts"]["failed"].as_u64()), (Some(1), Some(1)), "{body}");
    drop(running);
    scratch.close().await;
}

/// A connection of the test's own on the scratch schema, outside the stack's pool.
async fn own_connection(pool: &PgPool) -> PgConnection {
    let schema: String = sqlx::query_scalar("SELECT current_schema()").fetch_one(pool).await.expect("the schema");
    let url = env::var(support::DATABASE_VAR).expect("the test database");
    let mut conn = PgConnection::connect(&url).await.expect("a connection of the test's own");
    sqlx::query(&format!("SET search_path TO {schema}")).execute(&mut conn).await.expect("the scratch schema");
    conn
}

/// Statement 6 and spec 01 statement 16: a run's status entry racing a Dismiss lands in exactly one conversation, the
/// one live when it commits; its repeat after the Dismiss writes nothing; and a later transition lands in the next
/// conversation. The race is forced: the test holds the live conversation's row as Dismiss does, the writer waits on
/// it, and the test archives that conversation and opens the next before letting go.
#[tokio::test]
async fn a_run_entry_racing_a_dismiss_lands_in_the_conversation_live_when_it_commits() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let runs = submitted(&stack, "Peak hour", 1).await;
    let store = PgRunStore::new(stack.gate.app.clone());
    let mut queued = None;
    while queued.is_none() {
        let transitions = store.transitions_after(Cursor::default()).await.expect("the transitions");
        queued = transitions.into_iter().find(|transition| transition.to == RunState::Queued);
        sleep(Duration::from_millis(20)).await;
    }
    let queued = queued.expect("the queued transition");
    let live: String =
        sqlx::query_scalar("SELECT id FROM conversations WHERE state = 'live'").fetch_one(&scratch.pool).await.unwrap();
    let mut holder = own_connection(&scratch.pool).await;
    let mut held = holder.begin().await.expect("the test's transaction");
    sqlx::query("SELECT id FROM conversations WHERE id = $1 FOR UPDATE").bind(&live).execute(&mut *held).await.unwrap();
    let events = JournalEvents::new(stack.gate.app.clone());
    let writer = {
        let (events, queued) = (events.clone(), queued.clone());
        tokio::spawn(async move { events.status_changed(&queued).await })
    };
    sleep(Duration::from_millis(300)).await;
    sqlx::query("UPDATE conversations SET state = 'archived', ended_at = now() WHERE id = $1")
        .bind(&live)
        .execute(&mut *held)
        .await
        .unwrap();
    let next = support::ulid();
    sqlx::query(
        "INSERT INTO conversations (id, project, site, created_by, first_seq, last_seq) \
         SELECT $1, project, site, created_by, last_seq + 1, last_seq FROM conversations WHERE id = $2",
    )
    .bind(next.as_str())
    .bind(&live)
    .execute(&mut *held)
    .await
    .unwrap();
    held.commit().await.expect("the Dismiss commits");
    writer.await.expect("the writer").expect("the entry written");
    let landed: Vec<(String, String, String)> = entries(&scratch.pool).await;
    let queued_entries: Vec<&(String, String, String)> =
        landed.iter().filter(|(_, key, _)| key.ends_with(":queued")).collect();
    assert_eq!(queued_entries.len(), 1, "one entry");
    assert_eq!(queued_entries[0].2, next.to_string(), "in the conversation live when it committed");
    events.status_changed(&queued).await.expect("a repeat");
    assert_eq!(entries(&scratch.pool).await, landed, "its repeat after the Dismiss writes nothing");
    store.end(&key(&runs[0]), &ending(RunState::Finished)).await.expect("an ending");
    let mut handed = false;
    while !handed {
        for transition in store.transitions_after(queued.at).await.expect("the transitions") {
            events.status_changed(&transition).await.expect("told");
            handed = true;
        }
        sleep(Duration::from_millis(20)).await;
    }
    let finished: Vec<(String, String, String)> = entries(&scratch.pool).await;
    assert!(
        finished.iter().any(|(_, key, conversation)| key.ends_with(":finished") && *conversation == next.to_string())
    );
    scratch.close().await;
}
