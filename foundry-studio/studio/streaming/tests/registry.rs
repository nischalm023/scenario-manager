//! A streaming server's registration, against the database: registering deletes the rows of servers whose heartbeat
//! stopped past the expiry and keeps the ones merely stale; a beat writes the server's row again after someone
//! deleted it; and a server whose own heartbeat went stale past the expiry, a process asleep, keeps its row.

mod support;

use std::time::Duration;

use sqlx::PgPool;
use studio_streaming::registry::{Registration, beat, register};
use studio_streaming::stats::Stats;
use studio_streaming::streams::Streams;
use tokio::time::sleep;

use support::{GRACE, KEEP, SERVER};

fn registration() -> Registration {
    Registration {
        id: SERVER.parse().unwrap(),
        runner_url: "ws://runners.example:3201/runners".to_owned(),
        viewer_url: "wss://studio.example/streams".to_owned(),
    }
}

async fn registered(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT id FROM streaming_servers ORDER BY id").fetch_all(pool).await.unwrap()
}

#[tokio::test]
async fn dead_servers_rows_expire_a_deleted_row_comes_back_and_a_servers_own_never_goes() {
    let scratch = scratch_or_skip!();
    for (id, stopped) in [("01KZAVPQNES65HV4EZADPCSAA1", "11 minutes"), ("01KZAVPQNES65HV4EZADPCSAA2", "5 minutes")] {
        sqlx::query(
            "INSERT INTO streaming_servers (id, runner_url, viewer_url, heartbeat_at) \
             VALUES ($1, 'ws://gone.example:3201/runners', 'ws://gone.example:3202/streams', now() - $2::interval)",
        )
        .bind(id)
        .bind(stopped)
        .execute(&scratch.pool)
        .await
        .unwrap();
    }
    register(&scratch.pool, &registration()).await.expect("registered");
    assert_eq!(registered(&scratch.pool).await, ["01KZAVPQNES65HV4EZADPCSAA2", SERVER], "the expired row went");
    sqlx::query("DELETE FROM streaming_servers WHERE id = $1").bind(SERVER).execute(&scratch.pool).await.unwrap();
    let streams = Streams::new(KEEP, GRACE, Stats::default(), support::frame_store());
    let beating = beat(scratch.pool.clone(), registration(), streams, Duration::from_millis(50));
    let mut back = false;
    for _ in 0..100 {
        back = registered(&scratch.pool).await.contains(&SERVER.to_owned());
        if back {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(back, "the beat wrote the row again");
    beating.abort();
    sqlx::query("UPDATE streaming_servers SET heartbeat_at = now() - interval '1 hour' WHERE id = $1")
        .bind(SERVER)
        .execute(&scratch.pool)
        .await
        .unwrap();
    register(&scratch.pool, &registration()).await.expect("a beat after a long sleep");
    let fresh: bool =
        sqlx::query_scalar("SELECT heartbeat_at > now() - interval '1 minute' FROM streaming_servers WHERE id = $1")
            .bind(SERVER)
            .fetch_one(&scratch.pool)
            .await
            .expect("its own row is still there");
    assert!(fresh, "and alive again");
    scratch.close().await;
}
