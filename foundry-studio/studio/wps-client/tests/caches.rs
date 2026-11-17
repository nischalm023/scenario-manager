//! Spec 04 section 8's statements 7 and 8: the schema cache refreshes a component when a refusal names a version it
//! does not hold and never compiles a published version twice, and concurrent fetches never drop each other's
//! entries; the export cache exports a cut once however many ask, keeps exporting for a requester that stopped
//! waiting and charges the bytes an export keeps, the floor cache compiles once per export's bytes, a flight of
//! either retires itself alone, and each keeps within its byte budget.

mod support;

use std::future::Future;
use std::mem;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures_util::future::join_all;
use futures_util::poll;
use serde_json::json;
use studio_protocol::{BranchName, DeclaredId, Pin, ScopePath, Seq};
use support::{client, client_on, declare, path, site, write};
use tokio::task::spawn_blocking;
use tokio::time::{sleep, timeout};
use wps_client::cache::{ExportKey, FloorKey, export_bytes, export_cache, floor_cache};
use wps_client::{Client, Compute, ComputeBusy, CutExport, MockWps, UnitCut, WorldStore, WpsError};

/// Statement 7: a refusal naming a newer version refreshes that component; naming the held one does not; and no
/// published version is compiled twice, a revalidation included.
#[tokio::test]
async fn the_schema_cache_follows_a_newer_version_and_compiles_each_once() {
    let client = client().await;
    let held = client.registry().snapshot().components["flow.node"].latest.clone();
    assert_eq!(held.map(|version| version.to_string()).as_deref(), Some("0.1.0"));
    let compiled = client.registry().compilations();
    client.wps().publish("flow.node", "0.2.0", json!({ "type": "object", "required": ["kind"] }), Vec::new());
    assert!(!client.registry().note_refusal(client.wps(), "flow.node", "0.1.0").await.expect("the cache answers"));
    assert!(client.registry().note_refusal(client.wps(), "flow.node", "0.2.0").await.expect("the cache answers"));
    let latest = client.registry().snapshot().components["flow.node"].latest.clone();
    assert_eq!(latest.map(|version| version.to_string()).as_deref(), Some("0.2.0"));
    assert_eq!(client.registry().compilations(), compiled + 1, "only the new version is compiled");
    client.registry().revalidate(client.wps()).await.expect("a revalidation pass");
    assert_eq!(client.registry().compilations(), compiled + 1, "a revalidation compiles nothing it holds");
    assert!(
        client.registry().snapshot().validate("flow.node", &json!({})).is_err(),
        "the newer schema is the one checked"
    );
}

/// A site whose export closes, at `name`: a frame in its map.
async fn exportable(client: &Client<MockWps>, name: &str) -> (ScopePath, UnitCut) {
    let root = site(client, name).await;
    write(client, &path(&format!("{root}/map")), vec![declare(DeclaredId::mint(), json!({ "frame": {} }))]).await;
    let cut = client.resolve_cut(&root, &Pin::Branch(BranchName::main())).await.expect("a cut");
    (root, cut)
}

/// Statement 8: two concurrent exports of one cut ask WPS once; a byte budget is kept.
#[tokio::test]
async fn the_export_cache_exports_a_cut_once() {
    let client = client().await;
    let (root, cut) = exportable(&client, "cached").await;
    let before = client.wps().exports();
    let (first, second) = tokio::join!(client.export(&cut), client.export(&cut));
    let (first, second) = (first.expect("an export"), second.expect("an export"));
    assert_eq!(first.sha256, second.sha256);
    assert_eq!(client.wps().exports() - before, 1, "one flagged export for both");
    assert_eq!(first.holders.len(), 1, "the map's frame, the one declared entity");
    let map = path(&format!("{root}/map"));
    let kept = first.bytes.len() + mem::size_of::<(DeclaredId, ScopePath)>() + map.as_str().len();
    assert_eq!(export_bytes(&first), kept);
    assert_eq!(client.exports().bytes(), kept, "the export's bytes and holders kept");

    let small = export_cache(kept + 1);
    let export = Arc::new(first);
    let mut other = cut.seqs.clone();
    other.insert(root.clone(), Seq::new(9));
    for seqs in [cut.seqs.clone(), other] {
        let held = export.clone();
        let computed = small.get_or_spawn(ExportKey { seqs }, move || async move { Ok::<_, WpsError>(held) });
        computed.await.expect("held");
    }
    assert!(small.bytes() <= kept + 1, "past its budget the cache evicts");
}

/// A pool that holds every computation until it is opened, then runs each on tokio's blocking threads.
#[derive(Default)]
struct Gated {
    held: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
    open: AtomicBool,
}

impl Compute for Gated {
    fn submit(&self, job: Box<dyn FnOnce() + Send>) -> Result<(), ComputeBusy> {
        let mut held = self.held.lock().expect("the gate's lock");
        match self.open.load(Ordering::SeqCst) {
            true => drop(spawn_blocking(job)),
            false => held.push(job),
        }
        Ok(())
    }
}

impl Gated {
    fn open(&self) {
        let mut held = self.held.lock().expect("the gate's lock");
        self.open.store(true, Ordering::SeqCst);
        for job in mem::take(&mut *held) {
            drop(spawn_blocking(job));
        }
    }

    /// Drop every computation held, unanswered, as a pool that lost them would.
    fn lose(&self) {
        drop(mem::take(&mut *self.held.lock().expect("the gate's lock")));
    }

    fn waiting(&self) -> usize {
        self.held.lock().expect("the gate's lock").len()
    }
}

/// Drive `export` until its export holds a computation on `pool`, shut, and return without waiting for it more.
async fn held_on_the_pool(pool: &Gated, export: &mut (impl Future<Output = Result<CutExport, WpsError>> + Unpin)) {
    while pool.waiting() == 0 {
        tokio::select! {
            ended = &mut *export => panic!("an export ended with the pool shut: {ended:?}"),
            () = sleep(Duration::from_millis(1)) => {}
        }
    }
}

/// Statement 8 and the chore after world-proto b9d82a8 (kiQJVO): an export whose requester stops waiting is neither
/// restarted nor lost, the cache running it on, and the next requester reads that one export, asked of WPS once.
/// Before, the export was the requester's own work, which its leaving handed to the next one to start again.
#[tokio::test]
async fn a_requester_that_stops_waiting_leaves_its_export_running() {
    let pool = Arc::new(Gated::default());
    let client = client_on(pool.clone()).await;
    let (_, cut) = exportable(&client, "left").await;
    let before = client.wps().exports();
    {
        let mut left = pin!(client.export(&cut));
        held_on_the_pool(&pool, &mut left).await;
    }
    let next = client.export(&cut);
    pool.open();
    let export = timeout(Duration::from_secs(10), next).await.expect("the export ends").expect("an export");
    assert_eq!(client.wps().exports() - before, 1, "one flagged export for both");
    assert_eq!(export.scopes, cut.seqs);
}

/// The chore after world-proto b9d82a8 (kiQJVO): an export's flight that ends retires itself alone. The first flight
/// for a cut fails, its computation lost on the pool, and both its requesters read that one failure; a second flight
/// keeps a requester that arrives while it runs, so WPS is asked twice, once for each flight. Before, the first
/// flight's other requester exported the cut again on its own, and its ending retired the second flight, beside which
/// a third began.
#[tokio::test]
async fn an_export_flight_retires_itself_alone() {
    let pool = Arc::new(Gated::default());
    let client = client_on(pool.clone()).await;
    let (_, cut) = exportable(&client, "flights").await;
    let before = client.wps().exports();
    let patient = Duration::from_secs(5);
    let (mut a, mut b) = (pin!(client.export(&cut)), pin!(client.export(&cut)));
    held_on_the_pool(&pool, &mut a).await;
    assert!(poll!(b.as_mut()).is_pending(), "the second requester waits on the first flight");
    pool.lose();
    let (a, b) = (timeout(patient, a).await.expect("a ends"), timeout(patient, b).await.expect("b ends"));
    assert!(a.is_err() && b.is_err(), "both requesters read the first flight's failure: {a:?} {b:?}");
    let (mut c, mut d) = (pin!(client.export(&cut)), pin!(client.export(&cut)));
    held_on_the_pool(&pool, &mut c).await;
    assert!(poll!(d.as_mut()).is_pending(), "the fourth requester waits on the second flight");
    pool.open();
    let (c, d) = (timeout(patient, c).await.expect("c ends"), timeout(patient, d).await.expect("d ends"));
    assert_eq!(c.expect("an export").sha256, d.expect("an export").sha256);
    let exported = client.wps().exports() - before;
    assert_eq!(exported, 2, "once before the loss, then the cut once");
}

/// Statement 8: the floor cache compiles once per export's sha256, again for other bytes, within its budget.
#[tokio::test]
async fn the_floor_cache_compiles_once_per_document() {
    let floors = floor_cache(64);
    let compiles = Arc::new(AtomicUsize::new(0));
    let key =
        |document: &str| FloorKey { document: document.to_owned(), compiler: "floor-1/abc".to_owned(), draft: None };
    let compile = || {
        let compiles = compiles.clone();
        move || async move {
            compiles.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            Ok::<_, WpsError>(Bytes::from(vec![0u8; 40]))
        }
    };
    let (a, b) =
        tokio::join!(floors.get_or_spawn(key("sha-a"), compile()), floors.get_or_spawn(key("sha-a"), compile()));
    assert_eq!((a.expect("a floor"), b.expect("a floor")), (Bytes::from(vec![0u8; 40]), Bytes::from(vec![0u8; 40])));
    assert_eq!(compiles.load(Ordering::SeqCst), 1, "two asks, one compile");
    floors.get_or_spawn(key("sha-b"), compile()).await.expect("another floor");
    assert_eq!(compiles.load(Ordering::SeqCst), 2, "other bytes at the same cut compile again");
    assert!(floors.bytes() <= 64, "two 40-byte floors do not both fit 64 bytes");
}

/// Statement 7, under concurrency: fetches of many components at once each land in the cache, none dropping another's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_fetches_keep_every_entry() {
    let client = client().await;
    let names: Vec<String> = (0..64).map(|index| format!("parallel.c{index}")).collect();
    for name in &names {
        client.wps().publish(name, "0.1.0", json!({ "type": "object" }), Vec::new());
    }
    let client = Arc::new(client);
    let fetches = names.iter().cloned().map(|name| {
        let client = client.clone();
        tokio::spawn(async move { client.registry().ensure(client.wps(), &name).await.map(|entry| entry.is_some()) })
    });
    let fetched = join_all(fetches).await;
    assert!(fetched.iter().all(|entry| matches!(entry, Ok(Ok(true)))));
    let snapshot = client.registry().snapshot();
    let missing = snapshot.missing(names.iter().map(String::as_str));
    assert!(missing.is_empty(), "every fetched component is held: {missing:?}");
}
