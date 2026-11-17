//! Spec 01 section 3.15, the compute pool: a computation runs on a thread of the pool's own, waits for one in a
//! first-come queue of `STUDIO_COMPUTE_QUEUE`, and one that finds the queue full is refused at once with a wall-clock
//! wait; the flagged export's check, the pool's first user, answers `ComputeBusy` from a full pool and exports once
//! a thread is free.

mod support;

use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use studio_api::compute::{ComputeBudget, ComputePool, RETRY_AFTER};
use studio_protocol::{BranchName, Pin};
use tokio::time::sleep;
use wps_client::compute::run;
use wps_client::{Client, ClientConfig, Compute, WorldStore, WpsError};

const PATIENCE: Duration = Duration::from_secs(5);

fn pool(workers: &str, queue: &str) -> ComputePool {
    let budget = ComputeBudget { workers: workers.parse().expect("workers"), queue: queue.parse().expect("a depth") };
    ComputePool::new(budget).expect("the pool's threads start")
}

type Job = Box<dyn FnOnce() + Send>;

/// Submit what `job` makes, again while the pool answers busy, as a caller told `compute-busy` does: with no room to
/// wait, a thread takes a computation only once it is back at the queue.
fn submit_patiently(pool: &ComputePool, job: impl Fn() -> Job) {
    let began = Instant::now();
    while let Err(busy) = pool.submit(job()) {
        assert!(began.elapsed() < PATIENCE, "the pool stayed busy: {busy:?}");
        thread::sleep(Duration::from_millis(5));
    }
}

/// Hand `pool` a computation that holds its thread until the answer is sent, once it is running.
fn occupy(pool: &ComputePool) -> mpsc::Sender<()> {
    let (release, held) = mpsc::channel::<()>();
    let (running, started) = mpsc::channel();
    let held = Arc::new(Mutex::new(held));
    submit_patiently(pool, || {
        let (running, held) = (running.clone(), held.clone());
        Box::new(move || {
            let _ = running.send(());
            let _ = held.lock().map(|held| held.recv());
        })
    });
    started.recv_timeout(PATIENCE).expect("the computation runs");
    release
}

#[tokio::test]
async fn a_computation_runs_on_a_thread_of_the_pools_own() {
    let pool = ComputePool::new(ComputeBudget::default()).expect("the pool's threads start");
    let name = run(&pool, || thread::current().name().map(str::to_owned)).await.expect("an answer");
    assert!(name.as_deref().is_some_and(|name| name.starts_with("studio-compute-")), "{name:?}");
}

#[test]
fn a_full_queue_refuses_at_once_and_a_waiting_computation_runs_after() {
    let pool = pool("1", "1");
    let release = occupy(&pool);
    let (ran, done) = mpsc::channel();
    pool.submit(Box::new(move || {
        let _ = ran.send(());
    }))
    .expect("the second waits in the queue");
    let refused = pool.submit(Box::new(|| {})).expect_err("the third finds the queue full");
    assert_eq!(refused.retry_after, RETRY_AFTER);
    release.send(()).expect("the first is released");
    done.recv_timeout(PATIENCE).expect("the waiting computation runs once a thread is free");
}

#[test]
fn a_computation_that_panics_leaves_its_thread_running() {
    let pool = pool("1", "0");
    submit_patiently(&pool, || Box::new(|| panic!("a computation's own fault")));
    let (ran, done) = mpsc::channel();
    submit_patiently(&pool, || {
        let ran = ran.clone();
        Box::new(move || {
            let _ = ran.send(());
        })
    });
    done.recv_timeout(PATIENCE).expect("the same thread runs the next computation");
}

/// The export is checked on the pool: with its one thread held and no room to wait, it is refused as `ComputeBusy`,
/// and exports once the thread is free; the refusal is not kept by the export cache.
#[tokio::test]
async fn the_export_answers_compute_busy_from_a_full_pool() {
    let pool = Arc::new(pool("1", "0"));
    let world = Client::start(support::mock(), ClientConfig::default(), pool.clone()).await.expect("the mock's client");
    let root = support::made_site(&world, "customer/demo/site/busy", "Busy").await;
    let cut = world.resolve_cut(&root, &Pin::Branch(BranchName::main())).await.expect("a cut at the heads");

    let release = occupy(&pool);
    let refused = world.export(&cut).await.expect_err("no thread is free and none may wait");
    assert!(matches!(refused, WpsError::ComputeBusy { retry_after } if retry_after == RETRY_AFTER), "{refused:?}");
    release.send(()).expect("the thread is released");

    let began = Instant::now();
    let document = loop {
        match world.export(&cut).await {
            Ok(document) => break document,
            Err(WpsError::ComputeBusy { .. }) if began.elapsed() < PATIENCE => sleep(Duration::from_millis(5)).await,
            Err(other) => panic!("the export failed: {other:?}"),
        }
    };
    assert_eq!(document.scopes.get(&root), cut.seqs.get(&root));
}
