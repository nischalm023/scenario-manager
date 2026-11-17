//! The floor compile over a made L-Shape (`studio/viewer/tests/fixtures/l_shape/world.json`, the shell
//! template at its defaults as the engine evaluated it), and the floor cache it is held in.

use std::fs::read;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::oneshot::{self, Sender};
use tokio::time::{sleep, timeout};
use wps_client::cache::floor_cache;
use wps_client::compute::{BlockingCompute, Compute, ComputeBusy};

use super::*;

fn made_l_shape() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../viewer/tests/fixtures/l_shape/world.json");
    read(path).expect("the viewer's fixture is checked in")
}

/// Runs each computation as tokio's blocking threads do, counting them.
#[derive(Default)]
struct Counted(AtomicUsize);

impl Compute for Counted {
    fn submit(&self, job: Box<dyn FnOnce() + Send>) -> Result<(), ComputeBusy> {
        self.0.fetch_add(1, Ordering::SeqCst);
        BlockingCompute.submit(job)
    }
}

/// Holds each computation on its blocking thread until the test opens the gate, counting them.
#[derive(Default)]
struct Gated {
    submitted: AtomicUsize,
    gates: Mutex<Vec<Sender<()>>>,
}

impl Compute for Gated {
    fn submit(&self, job: Box<dyn FnOnce() + Send>) -> Result<(), ComputeBusy> {
        self.submitted.fetch_add(1, Ordering::SeqCst);
        let (open, opened) = oneshot::channel();
        self.gates.lock().expect("the gates").push(open);
        BlockingCompute.submit(Box::new(move || {
            let _ = opened.blocking_recv();
            job();
        }))
    }
}

impl Gated {
    fn open(&self) {
        for gate in self.gates.lock().expect("the gates").drain(..) {
            let _ = gate.send(());
        }
    }

    fn submitted(&self) -> usize {
        self.submitted.load(Ordering::SeqCst)
    }
}

/// The gate opened a little after the requesters ahead of it in a `join!` have asked.
async fn open_soon(pool: &Gated) {
    sleep(Duration::from_millis(20)).await;
    pool.open();
}

/// Spec 03 section 8 statement 6 rests on this: one document's floor document is the same bytes every time it is
/// compiled, and reads back as the floor and roll the one compile made.
#[test]
fn a_document_compiles_to_the_same_floor_document_every_time() {
    let document = made_l_shape();
    let floor = floor_document(&document).expect("the made L-Shape has a floor");
    assert_eq!(floor, floor_document(&document).expect("again"), "byte for byte");
    let (drawn, roll) = FloorAndRoll::read(&floor).expect("a browser reads it");
    let compiled = compile(&load(&document).unwrap()).unwrap();
    assert_eq!(drawn.cells.len(), compiled.floor.cells.len());
    assert_eq!(roll, compiled.roll());
}

#[test]
fn a_document_that_will_not_load_says_so_in_words() {
    let refused = floor_document(b"{ \"entities\": [] }").expect_err("no prologue, no floor");
    assert!(matches!(refused, FloorError::Load(_)), "{refused:?}");
    assert!(refused.to_string().starts_with("the site's document will not load: "), "{refused}");
}

/// Two asks at once compile once on the pool; a third is answered from the cache.
#[tokio::test]
async fn a_sites_floor_is_compiled_once_however_many_ask() {
    let (floors, pool) = (floor_cache(1 << 30), Arc::new(Counted::default()));
    let document = Bytes::from(made_l_shape());
    let key = floor_key("ab12", "test", None);
    let (first, second) = tokio::join!(
        site_floor(&floors, pool.clone(), key.clone(), document.clone()),
        site_floor(&floors, pool.clone(), key.clone(), document.clone()),
    );
    let first = first.expect("compiles");
    assert_eq!(first, second.expect("the same floor"));
    let third = site_floor(&floors, pool.clone(), key.clone(), document).await.expect("held");
    assert_eq!((third, pool.0.load(Ordering::SeqCst)), (first, 1));
    assert!(floors.contains(&key));
}

/// A document with no floor fails once for every requester waiting on its compile, and is never held.
#[tokio::test]
async fn a_document_with_no_floor_is_refused_once_for_all_who_asked_and_never_held() {
    let (floors, pool) = (floor_cache(1 << 30), Arc::new(Gated::default()));
    let key = floor_key("cd34", "test", None);
    let ask = || site_floor(&floors, pool.clone(), key.clone(), Bytes::from_static(b"not json"));
    let (first, second, ()) = tokio::join!(ask(), ask(), open_soon(&pool));
    for failure in [first, second] {
        let refused = matches!(&failure, Err(FloorFailure::Floor(why)) if matches!(**why, FloorError::Load(_)));
        assert!(refused, "{failure:?}");
    }
    assert_eq!(pool.submitted(), 1, "one compile and its one failure, read by both");
    assert!(!floors.contains(&key));
}

/// The cache owns the compile (reviews-impl/astra-t12.md finding 5): a requester that stops waiting leaves it
/// running, and the next waits on that compile rather than submitting another.
#[tokio::test]
async fn a_requester_that_stops_waiting_leaves_the_compile_running_for_the_next() {
    let (floors, pool) = (floor_cache(1 << 30), Arc::new(Gated::default()));
    let (key, document) = (floor_key("ab12", "test", None), Bytes::from(made_l_shape()));
    let left = timeout(Duration::from_millis(50), site_floor(&floors, pool.clone(), key.clone(), document.clone()));
    assert!(left.await.is_err(), "the compile is held at the gate, so its first requester gives up waiting");
    let (floor, ()) = tokio::join!(site_floor(&floors, pool.clone(), key.clone(), document), open_soon(&pool));
    assert!(FloorAndRoll::read(&floor.expect("the one compile's floor")).is_ok());
    assert_eq!(pool.submitted(), 1, "one compile, the requester who left counted in");
    assert!(floors.contains(&key));
}

/// A compile nobody waits for any more still lands, held for the next to ask.
#[tokio::test]
async fn a_compile_nobody_waits_for_still_lands_in_the_cache() {
    let (floors, pool) = (floor_cache(1 << 30), Arc::new(Gated::default()));
    let (key, document) = (floor_key("ef56", "test", None), Bytes::from(made_l_shape()));
    let left = timeout(Duration::from_millis(50), site_floor(&floors, pool.clone(), key.clone(), document));
    assert!(left.await.is_err());
    pool.open();
    let landed = async {
        while !floors.contains(&key) {
            sleep(Duration::from_millis(10)).await;
        }
    };
    timeout(Duration::from_secs(120), landed).await.expect("the floor is held with nobody waiting");
    let held = site_floor(&floors, pool.clone(), key, Bytes::new()).await.expect("answered from the cache");
    assert!(FloorAndRoll::read(&held).is_ok());
    assert_eq!(pool.submitted(), 1);
}

#[tokio::test]
async fn a_full_pool_is_compute_busy() {
    struct Full;
    impl Compute for Full {
        fn submit(&self, _: Box<dyn FnOnce() + Send>) -> Result<(), ComputeBusy> {
            Err(ComputeBusy { retry_after: Duration::from_secs(2) })
        }
    }
    let failure =
        site_floor(&floor_cache(1 << 30), Arc::new(Full), floor_key("ef56", "test", None), Bytes::new()).await;
    assert!(matches!(failure, Err(FloorFailure::Compute(WpsError::ComputeBusy { .. }))), "{failure:?}");
}

#[test]
fn a_key_reads_back_from_its_text_and_nothing_else_reads_as_one() {
    let site = floor_key("9f86d081884c7d65", "60aa424", None);
    let drafted = floor_key("9f86d081884c7d65", "60aa424", Some("2c26b46b"));
    for key in [site, drafted] {
        assert_eq!(read_key(&key_text(&key)), Some(key));
    }
    assert_eq!(key_text(&floor_key("9f86", "60aa424", None)), format!("9f86.floor{FLOOR_VERSION}@60aa424"));
    for refused in ["", "9f86", "9f86.compiler", "zz.floor1@x", "9f86.floor1@x.d.e", "9f86.floor1@x.not-hex"] {
        assert_eq!(read_key(refused), None, "{refused}");
    }
}
