//! The figures a streaming server writes to `attempt_streams`, against the database: at the ending at once, the
//! simtime the run reached with the runner's newest counters by name; never overwritten by an older revision, though a
//! restarted server's first write does overwrite its predecessor's; and kept through a write that fails until one
//! lands, whatever the streams forgot meanwhile.

mod support;

use std::env;

use desunits::SimTime;
use sqlx::postgres::PgPoolOptions;
use studio_protocol::{AttemptNumber, ProgressValue, RunnerMessage, WallClock};
use studio_streaming::stats::{Closed, ClosedBy, Snapshot, Stats, write};
use studio_streaming::streams::{StreamKey, Streams};

use support::{DATABASE_VAR, GRACE, RUN, Runner, SERVER, ended, progress, run, server, started, stream_row};

#[tokio::test]
async fn the_figures_land_at_the_ending_with_the_runners_newest_counters() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut runner = Runner::welcomed(&server).await;
    for message in [
        started(),
        RunnerMessage::Progress(progress(10_000, 2.0)),
        RunnerMessage::Progress(progress(20_000, 6.0)),
        RunnerMessage::Ended(ended(20_000)),
    ] {
        runner.send(message).await;
    }
    let row = stream_row(&scratch.pool, |row| row.closed_by.as_deref() == Some("runner")).await;
    assert_eq!(row.sim_ms, 20_000, "the simtime the run reached");
    assert_eq!(row.progress_values.0, progress(20_000, 6.0).values, "the runner's newest counters by name");
    scratch.close().await;
}

fn snapshot(revision: u64, sim_ms: u64, closed: bool) -> Snapshot {
    let picks = ProgressValue { metric: "picks".to_owned(), label: "Picks".to_owned(), value: Some(sim_ms as f64) };
    Snapshot {
        key: StreamKey { run: run(), attempt: AttemptNumber(1) },
        revision,
        opened_at: WallClock::now(),
        sim_ms: SimTime::ms(sim_ms),
        progress: vec![picks],
        closed: closed.then(|| Closed { at: WallClock::now(), by: ClosedBy::Runner }),
    }
}

#[tokio::test]
async fn an_older_revision_never_overwrites_a_newer_one_but_a_restarted_servers_first_write_does() {
    let scratch = scratch_or_skip!();
    let server = SERVER.parse().unwrap();
    write(&scratch.pool, &server, &snapshot(2, 600_000, true)).await.expect("the final figures");
    write(&scratch.pool, &server, &snapshot(1, 300_000, false)).await.expect("an older snapshot, landing late");
    let row = stream_row(&scratch.pool, |_| true).await;
    assert_eq!((row.revision, row.sim_ms, row.closed_by.as_deref()), (2, 600_000, Some("runner")));
    let restarted = "01KZAVPQNES65HV4EZADPCSZZ2".parse().unwrap();
    write(&scratch.pool, &restarted, &snapshot(1, 610_000, false)).await.expect("a new process's first figures");
    let row = stream_row(&scratch.pool, |_| true).await;
    assert_eq!((row.revision, row.sim_ms), (1, 610_000));
    scratch.close().await;
}

#[tokio::test]
async fn figures_wait_through_a_failed_write_until_one_lands_though_the_stream_is_forgotten() {
    let scratch = scratch_or_skip!();
    let stats = Stats::default();
    let streams = Streams::new(1, GRACE, stats.clone(), support::frame_store());
    for n in [1, 2] {
        let key = StreamKey { run: run(), attempt: AttemptNumber(n) };
        let (stream, socket) = streams.take_over(key).expect("opened");
        streams.take(&stream, socket, RunnerMessage::Ended(ended(u64::from(n) * 1_000))).expect("its ending");
    }
    assert!(streams.get(&StreamKey { run: run(), attempt: AttemptNumber(1) }).is_none(), "the first is forgotten");
    let server = SERVER.parse().unwrap();
    let url = env::var(DATABASE_VAR).unwrap();
    let broken = PgPoolOptions::new().max_connections(1).connect(&url).await.unwrap();
    broken.close().await;
    stats.write_waiting(&broken, &server).await;
    let written: i64 =
        sqlx::query_scalar("SELECT count(*) FROM attempt_streams").fetch_one(&scratch.pool).await.unwrap();
    assert_eq!(written, 0, "nothing landed through the closed pool");
    stats.write_waiting(&scratch.pool, &server).await;
    let closed: Vec<(i32, Option<String>)> =
        sqlx::query_as("SELECT n, closed_by FROM attempt_streams WHERE run_id = $1 ORDER BY n")
            .bind(RUN)
            .fetch_all(&scratch.pool)
            .await
            .unwrap();
    assert_eq!(closed, [(1, Some("runner".to_owned())), (2, Some("runner".to_owned()))], "both waited and landed");
    scratch.close().await;
}
