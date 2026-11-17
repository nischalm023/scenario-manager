//! What every backend has to do to stand in for the other.
//!
//! Relative to the crate: `runstore` promises one set of semantics whichever
//! backend it holds, and these tests check that each backend a build can reach
//! delivers them. Whole objects appearing atomically, whether put as bytes or
//! streamed up from a file; no clobbering under `create`; listings that are
//! lexical and prefix-exact; exact ranged reads; removal; a probe that answers.
//! And, because the folder is where the two backends were furthest apart, the
//! places they were made to agree: keys outside the alphabet refused before a
//! backend sees them, a read past the end of an object told apart from storage
//! failing, a key that holds no object deletable, a key running through an
//! object missing, and staging files never named in a listing.
//!
//! Every test is written once, against a `Ground`, and run over each backend in
//! turn: the folder always, in a throwaway directory; a bucket when the `gcs`
//! feature is on and one is named — an emulator's through
//! `RUNSTORE_GCS_EMULATOR`, which `studio/runs/dev.sh gcs-emulator` starts, or
//! a real one through `RUNSTORE_GCS_BUCKET` and the host's own credentials. The
//! emulator leaves two parts of the service out, and the two tests of exactly
//! those parts say they were skipped on it rather than fail; a real bucket
//! runs everything. The few assertions only a disk can answer are asked of the
//! folder alone. Whatever a test wrote under its run is swept afterwards.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use runstore::layout::AttemptNumber;
use runstore::store::PART_BYTES;
use runstore::{RunStorage, StoreError};
use tempfile::TempDir;

mod support;
use support::{Gap, Ground, grounds};

/// Each contract test as one `#[tokio::test]` that runs it over every backend
/// this build can reach, naming the ground so a failure says which, and
/// sweeping the run it wrote under afterwards.
macro_rules! on_every_backend {
    ($($name:ident),* $(,)?) => {
        $(
            #[tokio::test]
            async fn $name() {
                for ground in grounds() {
                    eprintln!("{}: {ground}", stringify!($name));
                    contract::$name(&ground).await;
                    ground.sweep().await;
                }
            }
        )*
    };
}

on_every_backend! {
    put_then_get_returns_the_same_bytes,
    put_replaces_an_object_whole,
    a_listing_never_names_a_half_written_object,
    put_file_streams_a_file_up_whole_and_never_lists_it_half_way,
    put_file_takes_a_small_file_and_refuses_what_is_not_a_file,
    put_file_takes_an_empty_file_and_one_of_exactly_a_part,
    create_refuses_a_key_that_is_taken,
    a_listing_is_lexical_and_holds_only_the_prefix,
    an_empty_prefix_lists_everything,
    get_range_returns_the_exact_slice,
    a_read_past_the_end_says_how_far_the_object_goes,
    a_key_that_holds_no_object_is_deletable,
    a_key_running_through_an_object_holds_nothing,
    a_staging_file_is_never_listed,
    delete_leaves_nothing_behind,
    a_missing_object_is_reported_as_missing,
    a_key_outside_the_alphabet_never_reaches_the_backend,
    a_probe_answers_whether_or_not_anything_is_stored,
    a_store_is_held_behind_the_interface,
}

mod contract {
    use super::*;

    pub async fn put_then_get_returns_the_same_bytes(ground: &Ground) {
        let store = &ground.store;
        let key = ground.spec_key();
        let written = Bytes::from_static(b"{\"warehouse\":\"cencora_v3\"}");

        let token = store.put(&key, written.clone()).await.expect("the put succeeds");

        assert_eq!(store.get(&key).await.expect("the object is there"), written);
        let head = store.head(&key).await.expect("the object is there");
        assert_eq!(head.key, key);
        assert_eq!(head.size, written.len() as u64);
        assert!(store.exists(&key).await.expect("the store is readable"));
        // What the write handed back names the same object a head would. An
        // emulator that reports no generation on a write is held to the rest.
        if ground.lacks(Gap::GenerationOnWrite) {
            assert_eq!((&token.key, token.size, &token.etag), (&head.key, head.size, &head.etag));
            assert!(token.generation.is_none() && head.generation.is_some(), "the gap is the emulator's: {token:?}");
        } else {
            assert_eq!(token, head, "the write's token disagreed with storage");
        }
    }

    pub async fn put_replaces_an_object_whole(ground: &Ground) {
        let store = &ground.store;
        let key = ground.part(0);

        store.put(&key, Bytes::from_static(b"first")).await.expect("the first put succeeds");
        store.put(&key, Bytes::from_static(b"second")).await.expect("the second put succeeds");

        assert_eq!(store.get(&key).await.expect("the object is there"), Bytes::from_static(b"second"));
    }

    pub async fn a_listing_never_names_a_half_written_object(ground: &Ground) {
        let store = Arc::new(ground.store.clone());
        let prefix = ground.parts_prefix(AttemptNumber::FIRST);
        // Large enough that the write is still in flight while the listings
        // below run, so a staging file is on disk at the same time as them.
        let bytes = Bytes::from(vec![0x5a_u8; 32 * 1024 * 1024]);
        let size = bytes.len() as u64;
        let key = ground.part(0);

        let writer = tokio::spawn({
            let store = Arc::clone(&store);
            let key = key.clone();
            async move { store.put(&key, bytes).await }
        });
        listed_whole_or_not_at_all(&*store, &prefix, &key, size, || writer.is_finished()).await;
        writer.await.expect("the writing task finished").expect("the put succeeds");

        let listed = store.list(&prefix).await.expect("the store is readable");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].size, size);
        // Nothing of the write is left behind under its own name on disk.
        if let Some(folder) = ground.on_disk() {
            assert_eq!(names_on_disk(&folder.join(prefix.trim_end_matches('/'))), vec!["000000".to_string()]);
        }
    }

    pub async fn put_file_streams_a_file_up_whole_and_never_lists_it_half_way(ground: &Ground) {
        if ground.skips(Gap::Multipart) {
            return;
        }
        let store = Arc::new(ground.store.clone());
        let prefix = ground.parts_prefix(AttemptNumber::FIRST);
        let key = ground.part(0);
        // Two whole parts and a short last one, so every branch of the
        // streaming goes up, with a pattern a mis-ordered part would break.
        let written: Vec<u8> = (0..=255_u8).cycle().take((2 * PART_BYTES + 12_345) as usize).collect();
        let size = written.len() as u64;
        let (_folder, path) = on_disk_file(&written);

        let writer = tokio::spawn({
            let store = Arc::clone(&store);
            let key = key.clone();
            async move { store.put_file(&key, &path).await }
        });
        listed_whole_or_not_at_all(&*store, &prefix, &key, size, || writer.is_finished()).await;
        let token = writer.await.expect("the writing task finished").expect("the upload succeeds");

        assert_eq!(token.key, key);
        assert_eq!(token.size, size);
        let head = store.head(&key).await.expect("the object is there");
        assert_eq!(head.size, size);
        assert_eq!(store.get(&key).await.expect("the object is there"), Bytes::from(written));
        // The folder assembles parts in a staging file beside the object;
        // completing the upload leaves only the object.
        if let Some(folder) = ground.on_disk() {
            assert_eq!(names_on_disk(&folder.join(prefix.trim_end_matches('/'))), vec!["000000".to_string()]);
        }
    }

    pub async fn put_file_takes_an_empty_file_and_one_of_exactly_a_part(ground: &Ground) {
        let store = &ground.store;
        // The two edges of the one-request path: nothing at all, and the most
        // that still goes up as one request rather than in parts.
        for (sequence, size) in [(1_u32, 0_usize), (2, PART_BYTES as usize)] {
            let key = ground.part(sequence);
            let written: Vec<u8> = (0..=255_u8).cycle().take(size).collect();
            let (_folder, path) = on_disk_file(&written);

            let token = store.put_file(&key, &path).await.expect("the upload succeeds");

            assert_eq!(token.size, size as u64, "{size} bytes");
            assert_eq!(store.head(&key).await.expect("the object is there").size, size as u64, "{size} bytes");
            assert_eq!(store.get(&key).await.expect("the object is there"), Bytes::from(written), "{size} bytes");
        }
    }

    pub async fn put_file_takes_a_small_file_and_refuses_what_is_not_a_file(ground: &Ground) {
        let store = &ground.store;
        let key = ground.part(1);
        let (folder, path) = on_disk_file(b"a few bytes, well under one part");

        let token = store.put_file(&key, &path).await.expect("the upload succeeds");
        assert_eq!(token.size, "a few bytes, well under one part".len() as u64);
        assert_eq!(
            store.get(&key).await.expect("the object is there"),
            Bytes::from_static(b"a few bytes, well under one part")
        );

        // A folder, and a path that is nothing at all: neither is a file, and
        // neither leaves an object behind under the key it was offered.
        let elsewhere = ground.part(2);
        assert!(store.put_file(&elsewhere, folder.path()).await.is_err(), "a folder is not a file");
        assert!(store.put_file(&elsewhere, &folder.path().join("missing")).await.is_err(), "nor is nothing");
        assert!(!store.exists(&elsewhere).await.expect("the store is readable"));
    }

    pub async fn create_refuses_a_key_that_is_taken(ground: &Ground) {
        if ground.skips(Gap::CreatePrecondition) {
            return;
        }
        let store = &ground.store;
        let key = ground.part(7);

        let token = store.create(&key, Bytes::from_static(b"first writer")).await.expect("the key was free");
        assert_eq!(token.key, key);
        assert_eq!(token.size, "first writer".len() as u64);
        let refused = store.create(&key, Bytes::from_static(b"stale twin")).await;

        assert!(
            matches!(&refused, Err(StoreError::AlreadyExists { key: refused_key }) if refused_key == &key),
            "expected the second create to be refused, got {refused:?}"
        );
        assert_eq!(
            store.get(&key).await.expect("the object is there"),
            Bytes::from_static(b"first writer"),
            "the refused create must not have touched the object"
        );
    }

    pub async fn a_listing_is_lexical_and_holds_only_the_prefix(ground: &Ground) {
        let store = &ground.store;
        let attempt = AttemptNumber::FIRST;
        let other_attempt = AttemptNumber::new(2).expect("attempts count from 1");
        let payload = Bytes::from_static(b"part");

        for sequence in [10_u32, 2, 1_000, 0] {
            store.put(&ground.part(sequence), payload.clone()).await.expect("the put succeeds");
        }
        store.put(&ground.spec_key(), payload.clone()).await.expect("the put succeeds");
        let elsewhere = ground.part_of(other_attempt, 0);
        store.put(&elsewhere, payload.clone()).await.expect("the put succeeds");

        let keys: Vec<String> = store
            .list(&ground.parts_prefix(attempt))
            .await
            .expect("the store is readable")
            .into_iter()
            .map(|o| o.key)
            .collect();

        assert_eq!(
            keys,
            [0_u32, 2, 10, 1_000].into_iter().map(|sequence| ground.part(sequence)).collect::<Vec<_>>(),
            "the objects come back in key order, and nothing outside the prefix comes back"
        );

        // A prefix that stops inside a segment still means what it says.
        let partial = format!("{}0000", ground.parts_prefix(attempt));
        let partial_keys: Vec<String> =
            store.list(&partial).await.expect("the store is readable").into_iter().map(|o| o.key).collect();
        assert_eq!(partial_keys, [0_u32, 2, 10].into_iter().map(|sequence| ground.part(sequence)).collect::<Vec<_>>());
    }

    pub async fn an_empty_prefix_lists_everything(ground: &Ground) {
        let store = &ground.store;
        let key = ground.part(0);
        store.put(&key, Bytes::from_static(b"part")).await.expect("the put succeeds");

        let listed: Vec<String> =
            store.list("").await.expect("the store is readable").into_iter().map(|o| o.key).collect();

        // A fresh folder holds this one object and nothing else; a bucket is
        // shared with every other test and run, so it is only held to naming it.
        if ground.on_disk().is_some() {
            assert_eq!(listed, vec![key]);
        } else {
            assert!(listed.contains(&key), "the listing left out {key}");
        }
    }

    pub async fn get_range_returns_the_exact_slice(ground: &Ground) {
        let store = &ground.store;
        let key = ground.part(3);
        let written: Vec<u8> = (0..=255_u8).cycle().take(4096).collect();
        store.put(&key, Bytes::from(written.clone())).await.expect("the put succeeds");

        let slice = store.get_range(&key, 100..228).await.expect("the range is inside the object");
        assert_eq!(slice, Bytes::from(written[100..228].to_vec()));

        let tail = store.get_range(&key, 4095..4096).await.expect("the range is inside the object");
        assert_eq!(tail, Bytes::from(written[4095..4096].to_vec()));

        // A reader chasing a writer asks for more than is there and gets what
        // is there; only a start past the end is a failure.
        let over_the_end = store.get_range(&key, 4000..5000).await.expect("a range ending past the object reads short");
        assert_eq!(over_the_end, Bytes::from(written[4000..4096].to_vec()));
        let past = store.get_range(&key, 4096..4200).await;
        assert!(matches!(past, Err(StoreError::PastTheEnd { start: 4096, size: 4096, .. })), "got {past:?}");
    }

    pub async fn a_read_past_the_end_says_how_far_the_object_goes(ground: &Ground) {
        let store = &ground.store;
        let key = ground.part(0);
        store.put(&key, Bytes::from_static(b"0123456789")).await.expect("the put succeeds");

        // A reader chasing a writer asks past the end and learns it has caught
        // up, rather than getting a failure it would have to guess at.
        let caught_up = store.get_range(&key, 10..20).await;
        assert!(matches!(caught_up, Err(StoreError::PastTheEnd { start: 10, size: 10, .. })), "got {caught_up:?}");
        let well_past = store.get_range(&key, 99..100).await;
        assert!(matches!(well_past, Err(StoreError::PastTheEnd { start: 99, size: 10, .. })), "got {well_past:?}");

        // A range with nothing in it is not a read at all.
        assert_eq!(store.get_range(&key, 4..4).await.expect("an empty range reads nothing"), Bytes::new());
        // A range that runs backwards holds no bytes either, the way Rust reads one.
        let (start, end) = (400_u64, 4_u64);
        assert_eq!(store.get_range(&key, start..end).await.expect("an empty range reads nothing"), Bytes::new());

        // A start inside the object still reads what is there.
        assert_eq!(store.get_range(&key, 8..20).await.expect("a short read"), Bytes::from_static(b"89"));
    }

    pub async fn a_key_that_holds_no_object_is_deletable(ground: &Ground) {
        let store = &ground.store;
        let key = ground.part(0);
        store.put(&key, Bytes::from_static(b"part")).await.expect("the put succeeds");

        // A level of the layout is a folder on disk and nothing at all in a
        // bucket. Deleting it removes no object, and that is not a failure on
        // either.
        let level = ground.parts_prefix(AttemptNumber::FIRST);
        store.delete(level.trim_end_matches('/')).await.expect("a key that holds no object is deletable");
        store.delete(&ground.part(1)).await.expect("a key nothing was written to is deletable");

        assert!(store.exists(&key).await.expect("the store is readable"), "the object under it is untouched");
    }

    pub async fn a_key_running_through_an_object_holds_nothing(ground: &Ground) {
        let store = &ground.store;
        let key = ground.part(0);
        store.put(&key, Bytes::from_static(b"part")).await.expect("the put succeeds");

        // `<object>/inner` can hold nothing: on the folder the object is a file
        // rather than a directory, and a bucket simply has no such object.
        let below = format!("{key}/inner");
        assert!(matches!(store.get(&below).await, Err(StoreError::NotFound { .. })));
        assert!(matches!(store.head(&below).await, Err(StoreError::NotFound { .. })));
        assert!(!store.exists(&below).await.expect("the store is readable"));
        store.delete(&below).await.expect("a key that holds no object is deletable");
    }

    pub async fn a_staging_file_is_never_listed(ground: &Ground) {
        let store = &ground.store;
        let key = ground.part(0);
        store.put(&key, Bytes::from_static(b"part")).await.expect("the put succeeds");
        let prefix = ground.parts_prefix(AttemptNumber::FIRST);

        // The deterministic companion to the race above: the folder backend
        // writes every object through a file named `<key>#<digits>`, so one is
        // planted here rather than waited for. Only a disk can be planted on.
        if let Some(folder) = ground.on_disk() {
            let staging = folder.join(prefix.trim_end_matches('/')).join("000000#12345");
            fs::write(&staging, b"half an object").expect("the parts folder exists");
            let listed: Vec<String> =
                store.list(&prefix).await.expect("the store is readable").into_iter().map(|o| o.key).collect();
            assert_eq!(listed, vec![key.clone()], "a listing named a staging file");
        }

        // Nor can one be addressed, on either backend: `#` is not in the key
        // alphabet.
        assert!(matches!(store.get(&format!("{key}#12345")).await, Err(StoreError::InvalidKey { .. })));
    }

    pub async fn delete_leaves_nothing_behind(ground: &Ground) {
        let store = &ground.store;
        let key = ground.part(1);
        store.put(&key, Bytes::from_static(b"part")).await.expect("the put succeeds");
        assert!(store.exists(&key).await.expect("the store is readable"));

        store.delete(&key).await.expect("the delete succeeds");

        assert!(!store.exists(&key).await.expect("the store is readable"));
        assert!(store.list(&ground.parts_prefix(AttemptNumber::FIRST)).await.expect("readable").is_empty());
        assert!(matches!(store.get(&key).await, Err(StoreError::NotFound { .. })));
        store.delete(&key).await.expect("deleting what is already gone succeeds");
    }

    pub async fn a_missing_object_is_reported_as_missing(ground: &Ground) {
        let store = &ground.store;
        let key = ground.part(0);

        assert!(matches!(store.get(&key).await, Err(StoreError::NotFound { .. })));
        assert!(matches!(store.head(&key).await, Err(StoreError::NotFound { .. })));
        assert!(!store.exists(&key).await.expect("the store is readable"));
        assert!(store.list(&ground.parts_prefix(AttemptNumber::FIRST)).await.expect("readable").is_empty());
    }

    pub async fn a_key_outside_the_alphabet_never_reaches_the_backend(ground: &Ground) {
        let store = &ground.store;
        let payload = Bytes::from_static(b"part");
        let (_folder, file) = on_disk_file(b"part");

        // `#` is the folder backend's own staging namespace and an ordinary
        // object name in a bucket; a trailing or leading `/` is silently
        // rewritten by both.
        for key in ["", "runs/a#1", "/runs/a", "runs/a/", "runs//a", "runs/../a", "runs/%2E%2E/a"] {
            assert!(
                matches!(store.put(key, payload.clone()).await, Err(StoreError::InvalidKey { .. })),
                "put(`{key}`) should have been refused"
            );
            assert!(matches!(store.put_file(key, &file).await, Err(StoreError::InvalidKey { .. })));
            assert!(matches!(store.create(key, payload.clone()).await, Err(StoreError::InvalidKey { .. })));
            assert!(matches!(store.get(key).await, Err(StoreError::InvalidKey { .. })));
            assert!(matches!(store.head(key).await, Err(StoreError::InvalidKey { .. })));
            assert!(matches!(store.exists(key).await, Err(StoreError::InvalidKey { .. })));
            assert!(matches!(store.delete(key).await, Err(StoreError::InvalidKey { .. })));
        }

        // Nothing was written on the way to being refused. The bucket is shared
        // with every other test, so it is this run's prefix that has to be
        // empty; the folder is this test's alone, so all of it is.
        assert!(store.list(&ground.run_prefix()).await.expect("the store is readable").is_empty());
        if let Some(folder) = ground.on_disk() {
            assert!(fs::read_dir(folder).expect("the folder exists").next().is_none());
        }

        // A prefix no key could start with is refused rather than answered with
        // an empty listing, and `/` is not the way to spell "everything".
        assert!(matches!(store.list("/").await, Err(StoreError::InvalidKey { .. })));
        assert!(matches!(store.list("runs//").await, Err(StoreError::InvalidKey { .. })));
        assert!(matches!(store.list("runs/a#").await, Err(StoreError::InvalidKey { .. })));
    }

    pub async fn a_probe_answers_whether_or_not_anything_is_stored(ground: &Ground) {
        let store = &ground.store;
        // A fresh folder has no `runs/` at all; the emulator's bucket may hold
        // every other test's runs. Both answer, and so does a store that has
        // just been written to.
        store.probe().await.expect("a reachable store answers its probe");
        store.put(&ground.spec_key(), Bytes::from_static(b"spec")).await.expect("the put succeeds");
        store.probe().await.expect("and still does once it holds a run");
    }

    pub async fn a_store_is_held_behind_the_interface(ground: &Ground) {
        // The server holds one store across its handlers, so the trait has to
        // be usable as a trait object.
        let shared: Arc<dyn RunStorage> = Arc::new(ground.store.clone());
        let key = ground.spec_key();

        shared.put(&key, Bytes::from_static(b"spec")).await.expect("the put succeeds");
        assert_eq!(shared.get(&key).await.expect("the object is there"), Bytes::from_static(b"spec"));
    }

    /// Keep listing `prefix` until `done` says the write has finished, holding
    /// that every object it ever names is `key`, whole, and readable whole.
    /// Either the object is not there yet, or it is there entire: the staging
    /// a backend writes through is never listed, and a listed object never
    /// reads back short.
    async fn listed_whole_or_not_at_all(
        store: &dyn RunStorage,
        prefix: &str,
        key: &str,
        size: u64,
        done: impl Fn() -> bool,
    ) {
        let mut read_back = false;
        loop {
            for object in store.list(prefix).await.expect("the store is readable") {
                assert_eq!(object.key, key, "a listing named something no writer wrote");
                assert_eq!(object.size, size, "a listing named a partly written object");
                if !read_back {
                    let read = store.get(key).await.expect("a listed object is readable");
                    assert_eq!(read.len() as u64, size, "a listed object read back short");
                    read_back = true;
                }
            }
            if done() {
                break;
            }
            tokio::task::yield_now().await;
        }
    }

    /// A file holding `contents`, in a folder of its own that lives as long as
    /// the handle; what `put_file` is handed.
    fn on_disk_file(contents: &[u8]) -> (TempDir, PathBuf) {
        let folder = TempDir::new().expect("a temporary folder");
        let path = folder.path().join("recording.worldview");
        fs::write(&path, contents).expect("the file is written");
        (folder, path)
    }

    /// Every name in one folder of the store on disk, for the assertions that
    /// nothing but the object was left behind.
    fn names_on_disk(parts: &Path) -> Vec<String> {
        fs::read_dir(parts)
            .expect("the parts folder exists")
            .map(|entry| entry.expect("a readable entry").file_name().to_string_lossy().into_owned())
            .collect()
    }
}
