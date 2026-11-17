//! What the registry promises, against a real PostgreSQL.
//!
//! Each test gets a schema of its own inside one database — created, migrated
//! and dropped here — so the whole file runs in parallel against the container
//! `dev.sh` starts, and no test sees another's rows. The database is named by
//! `DATABASE_URL`; with none set, these tests report why they are skipped
//! rather than failing, because the workspace has to build and test without a
//! database in sight.
//!
//! What is tested is what a restart and a queue depend on: an attempt is
//! claimed once however many workers reach for it, an attempt left running
//! becomes `partial` and nothing more, a queued run cancels outright, and an
//! ending offered to an attempt that is no longer running is refused.

use crucible::protocol::{AttemptState, Progress, RecordingExtent};
use crucible::registry::{Registry, RegistryError};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Executor, PgPool};

/// A registry in a schema of its own, and the pool to drop it with.
struct Scratch {
    registry: Registry,
    pool: PgPool,
    schema: String,
}

impl Scratch {
    /// Open one, or say that no database was named.
    ///
    /// Only an absent `DATABASE_URL` is a skip. A database that is named and
    /// will not connect, or will not take the schema, FAILS: a gate that
    /// reports a cheerful skip when the database is unreachable is a gate that
    /// passes for the wrong reason.
    async fn open() -> Option<Self> {
        let url = std::env::var("DATABASE_URL").ok()?;
        let schema = format!("crucible_test_{}", unique_name());
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap_or_else(|failure| panic!("DATABASE_URL names a database this test cannot reach: {failure}"));
        admin.execute(format!("CREATE SCHEMA {schema}").as_str()).await.expect("a schema of our own");
        admin.close().await;

        // Every pooled connection sees only this schema, so `migrate!` builds
        // the tables inside it and every query below finds those.
        let options: PgConnectOptions = url.parse().expect("DATABASE_URL is a connection string");
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .after_connect({
                let schema = schema.clone();
                move |connection, _| {
                    let schema = schema.clone();
                    Box::pin(async move {
                        connection.execute(format!("SET search_path TO {schema}").as_str()).await?;
                        Ok(())
                    })
                }
            })
            .connect_with(options)
            .await
            .unwrap_or_else(|failure| panic!("the test's own schema would not open: {failure}"));
        let registry = Registry::from_pool(pool.clone()).await.expect("the schema migrates");
        Some(Self { registry, pool, schema })
    }

    /// Drop the schema. Explicit rather than a `Drop`, because dropping needs
    /// to await.
    async fn close(self) {
        let schema = self.schema;
        self.pool.execute(format!("DROP SCHEMA {schema} CASCADE").as_str()).await.expect("the schema drops");
        self.pool.close().await;
    }
}

/// A name no other test will pick.
///
/// The counter, not the clock, is what makes it unique: several of these tests
/// start inside one tick of this machine's nanosecond clock, and two that
/// picked the same name both tried to create the same schema.
fn unique_name() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("after 1970").as_nanos();
    format!("{now:x}_{:x}_{:x}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
}

/// Record a run and queue its attempt, which is what a submit does once its
/// client has been told the run exists.
async fn submit_queued(registry: &Registry, run: &str, name: &str, spec_key: &str) {
    registry.submit(run, name, spec_key).await.expect("the run is recorded");
    registry.queue_first_attempt(run).await.expect("the attempt is queued");
}

/// Report a skip the way a reader can act on.
macro_rules! registry_or_skip {
    () => {
        match Scratch::open().await {
            Some(scratch) => scratch,
            None => {
                eprintln!("skipped: set DATABASE_URL to a PostgreSQL the tests may create schemas in");
                return;
            }
        }
    };
}

#[tokio::test]
async fn a_queued_attempt_is_claimed_once_however_many_workers_reach_for_it() {
    let scratch = registry_or_skip!();
    let registry = scratch.registry.clone();
    submit_queued(&registry, "01AAAAAAAAAAAAAAAAAAAAAAAA", "one", "runs/one/spec.json").await;

    let claims = futures::future::join_all((0..4).map(|_| registry.claim_queued())).await;
    let taken: Vec<_> = claims.into_iter().map(|claim| claim.expect("the registry answers")).flatten().collect();
    assert_eq!(taken.len(), 1, "one attempt, one worker");
    assert_eq!(taken[0].run, "01AAAAAAAAAAAAAAAAAAAAAAAA");
    assert_eq!(taken[0].n, 1);
    assert_eq!(taken[0].name, "one");
    assert_eq!(taken[0].spec_key, "runs/one/spec.json");

    scratch.close().await;
}

#[tokio::test]
async fn an_attempt_left_running_becomes_partial_and_nothing_is_invented_for_it() {
    let scratch = registry_or_skip!();
    let registry = scratch.registry.clone();
    submit_queued(&registry, "01BBBBBBBBBBBBBBBBBBBBBBBB", "two", "runs/two/spec.json").await;
    registry.claim_queued().await.expect("claims").expect("there is one");
    registry
        .record_progress("01BBBBBBBBBBBBBBBBBBBBBBBB", 1, Progress { sim_ms: 61_000, picks: 40 })
        .await
        .expect("progress");

    assert_eq!(registry.sweep_interrupted().await.expect("sweeps"), 1);

    let summary = registry.summary("01BBBBBBBBBBBBBBBBBBBBBBBB").await.expect("reads").expect("the run exists");
    let attempt = &summary.attempts[0];
    assert_eq!(attempt.state, AttemptState::Partial);
    assert_eq!(attempt.object_key, None, "a prefix nobody can vouch for is not stored");
    // The progress it had reached stands; a partial attempt is not rewound to
    // zero, and nothing is filled in beyond where it got.
    assert_eq!(attempt.progress.sim_ms, 61_000);
    assert_eq!(attempt.progress.picks, 40);
    assert!(attempt.ended_at.is_some(), "the attempt is over");

    // A second sweep finds nothing: `partial` is not `running`.
    assert_eq!(registry.sweep_interrupted().await.expect("sweeps"), 0);
    scratch.close().await;
}

#[tokio::test]
async fn a_run_nothing_has_started_cancels_outright_and_leaves_the_queue() {
    let scratch = registry_or_skip!();
    let registry = scratch.registry.clone();
    submit_queued(&registry, "01CCCCCCCCCCCCCCCCCCCCCCCC", "three", "runs/three/spec.json").await;

    assert!(registry.cancel_queued("01CCCCCCCCCCCCCCCCCCCCCCCC").await.expect("cancels"));
    assert!(registry.claim_queued().await.expect("claims").is_none(), "a cancelled attempt is not queued");
    // Cancelling twice is not a second cancellation.
    assert!(!registry.cancel_queued("01CCCCCCCCCCCCCCCCCCCCCCCC").await.expect("cancels"));

    let summary = registry.summary("01CCCCCCCCCCCCCCCCCCCCCCCC").await.expect("reads").expect("the run exists");
    assert_eq!(summary.attempts[0].state, AttemptState::Cancelled);
    scratch.close().await;
}

#[tokio::test]
async fn an_ending_is_written_once_and_a_second_closer_loses() {
    let scratch = registry_or_skip!();
    let registry = scratch.registry.clone();
    submit_queued(&registry, "01FFFFFFFFFFFFFFFFFFFFFFFF", "twice", "runs/twice/spec.json").await;
    let claimed = registry.claim_queued().await.expect("claims").expect("there is one");

    // A restart sweeping a live attempt to `partial` is the case that matters:
    // the server still running it must not write `finished` back over the sweep.
    assert_eq!(registry.sweep_interrupted().await.expect("sweeps"), 1);
    let late = registry
        .attempt_ended(
            &claimed.run,
            claimed.n,
            AttemptState::Finished,
            Some("all orders completed"),
            Some("k"),
            None,
            None,
        )
        .await;
    assert!(
        matches!(late, Err(RegistryError::NotRunning { ref run, attempt: 1 }) if run == "01FFFFFFFFFFFFFFFFFFFFFFFF"),
        "an attempt that is no longer running is not closed a second time: {late:?}"
    );

    let summary = registry.summary("01FFFFFFFFFFFFFFFFFFFFFFFF").await.expect("reads").expect("the run exists");
    assert_eq!(summary.attempts[0].state, AttemptState::Partial);
    assert_eq!(summary.attempts[0].object_key, None, "the late ending wrote nothing");
    scratch.close().await;
}

#[tokio::test]
async fn an_ending_names_the_object_and_a_listing_is_newest_first() {
    let scratch = registry_or_skip!();
    let registry = scratch.registry.clone();
    for (run, name) in [("01DDDDDDDDDDDDDDDDDDDDDDDD", "older"), ("01EEEEEEEEEEEEEEEEEEEEEEEE", "newer")] {
        submit_queued(&registry, run, name, &format!("runs/{name}/spec.json")).await;
    }
    let claimed = registry.claim_queued().await.expect("claims").expect("there is one");
    registry
        .attempt_ended(
            &claimed.run,
            claimed.n,
            AttemptState::Finished,
            Some("sim duration limit reached"),
            Some("runs/older/attempts/1/recording.worldview"),
            None,
            Some(RecordingExtent { live_edge_ms: 610_000, frames: 66, size_bytes: 785_367 }),
        )
        .await
        .expect("the ending is written");

    let runs = registry.list().await.expect("lists");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].name, "newer", "newest first");
    let older = runs.iter().find(|run| run.name == "older").expect("the older run is listed");
    assert_eq!(older.attempts[0].state, AttemptState::Finished);
    assert_eq!(older.attempts[0].object_key.as_deref(), Some("runs/older/attempts/1/recording.worldview"));
    assert_eq!(older.attempts[0].stop_reason.as_deref(), Some("sim duration limit reached"));
    // `list` carries the extent, so a browser choosing where to attach asks
    // once instead of fetching an object per run to learn how long each is.
    let held = older.attempts[0].recording.expect("the ending wrote down what the recording holds");
    assert_eq!((held.live_edge_ms, held.frames, held.size_bytes), (610_000, 66, 785_367));
    let newer = runs.iter().find(|run| run.name == "newer").expect("the newer run is listed");
    assert!(newer.attempts[0].recording.is_none(), "an attempt that has not ended has no recording to describe");
    scratch.close().await;
}
