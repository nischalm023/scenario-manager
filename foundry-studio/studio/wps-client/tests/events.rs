//! Spec 04 section 3.6 against the mock: a watched scope yields `Moved` for each changeset, shared by every consumer of
//! it; a consumer that fell behind reads its next event marked as a gap rather than hearing nothing of the miss; a
//! consumer stating the changeset it knows hears of every move past it, one landing before the watch was ready too;
//! and one changeset across several scopes moves each on its own feed, every move carrying the commit place they
//! share.

mod support;

use std::time::Duration;

use futures_util::StreamExt;
use serde_json::json;
use studio_protocol::{DeclaredId, Seq};
use support::{client, client_of, declare, object, path, read_site, site, write};
use tokio::time::timeout;
use wps_client::{Author, DraftBuilder, MockWps, ScopeEvent, WorldStore};

#[tokio::test]
async fn a_watched_scope_moves_and_a_consumer_behind_hears_of_its_gap() {
    let client = client().await;
    let root = site(&client, "events").await;
    let map = path(&format!("{root}/map"));
    let mut first = client.events(&map);
    let mut behind = client.events(&map);
    tokio::time::sleep(Duration::from_millis(100)).await;
    write(&client, &map, vec![declare(DeclaredId::mint(), json!({ "frame": {} }))]).await;
    let moved = timeout(Duration::from_secs(2), first.next()).await.expect("within 2 s").expect("an event");
    let ScopeEvent::Moved { seq, gap, .. } = moved else { panic!("{moved:?}") };
    assert_eq!((seq, gap), (Seq::new(1), false));
    for _ in 0..1100 {
        write(&client, &map, vec![declare(DeclaredId::mint(), json!({ "frame": {} }))]).await;
    }
    let (mut seen, mut told) = (Vec::new(), false);
    while seen.last() != Some(&Seq::new(1101)) {
        let next = timeout(Duration::from_secs(2), behind.next()).await.expect("never silent").expect("an event");
        match next {
            ScopeEvent::Moved { seq, gap, .. } => {
                told |= gap;
                seen.push(seq);
            }
            ScopeEvent::Resynced { head, .. } => {
                told = true;
                seen.push(head);
            }
        }
    }
    let unbroken = seen.windows(2).all(|pair| pair[1].get() == pair[0].get() + 1) && seen.len() == 1101;
    assert!(told || unbroken, "a consumer that missed changesets is told so");
    assert!(told, "1100 changesets overrun both the mock's 256 and the hub's 1024, so a gap is marked");
}

/// Spec 01 section 3.11's stale-draft watcher: a changeset landing at once, before the watch's first head read, still
/// reaches a consumer that stated its base.
#[tokio::test]
async fn a_move_before_the_watch_is_ready_reaches_a_stated_base() {
    let client = client().await;
    let root = site(&client, "ready").await;
    let map = path(&format!("{root}/map"));
    let mut watcher = client.events_since(&map, Seq::new(0));
    write(&client, &map, vec![declare(DeclaredId::mint(), json!({ "frame": {} }))]).await;
    let heard = timeout(Duration::from_secs(2), watcher.next()).await.expect("within 2 s").expect("an event");
    let seq = match heard {
        ScopeEvent::Moved { seq, .. } => seq,
        ScopeEvent::Resynced { head, .. } => head,
    };
    assert_eq!(seq, Seq::new(1));
}

/// Section 3.6: a watch opened after the scope moved past the stated changeset tells of it first, as `Resynced`.
#[tokio::test]
async fn a_watch_opened_after_a_move_tells_of_it_first() {
    let client = client().await;
    let root = site(&client, "late").await;
    let map = path(&format!("{root}/map"));
    write(&client, &map, vec![declare(DeclaredId::mint(), json!({ "frame": {} }))]).await;
    let mut watcher = client.events_since(&map, Seq::new(0));
    let heard = timeout(Duration::from_secs(2), watcher.next()).await.expect("within 2 s").expect("an event");
    assert!(matches!(heard, ScopeEvent::Resynced { head, .. } if head == Seq::new(1)), "{heard:?}");
}

/// Section 3.6 at scopes phase 3: one changeset writing a site's four scopes moves each scope's feed once, every
/// `Moved` carrying one commit place; the next changeset, writing one of them, carries another.
#[tokio::test]
async fn one_changeset_moves_each_scope_under_one_commit() {
    let client = client_of(MockWps::phase_3()).await;
    let root = site(&client, "commit").await;
    let scopes =
        [root.clone(), path(&format!("{root}/map")), path(&format!("{root}/flow")), path(&format!("{root}/fleet"))];
    let mut watches: Vec<_> = scopes.iter().map(|scope| client.events_since(scope, Seq::new(0))).collect();
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Each consumer starts from the head it reads when first polled, so each is polled, and is quiet, before the write.
    for watch in &mut watches {
        assert!(timeout(Duration::from_millis(20), watch.next()).await.is_err(), "nothing moved yet");
    }
    let base = read_site(&client, &root).await;
    let author = Author::new("approver").expect("an author");
    let mut builder = DraftBuilder::new(&base, root.clone(), author, "All four", "01JAZ0COMMIT");
    for scope in &scopes {
        builder.declare(scope, None, object(json!({ "external_ids": { "src": scope.to_string() } })));
    }
    client.apply(&builder.build()).await.expect("one changeset");
    let mut commits = Vec::new();
    for watch in &mut watches {
        let heard = timeout(Duration::from_secs(2), watch.next()).await.expect("within 2 s").expect("an event");
        let ScopeEvent::Moved { seq, commit, .. } = heard else { panic!("a move, not a resync: {heard:?}") };
        assert_eq!(seq, Seq::new(1));
        commits.push(commit.expect("a commit place"));
    }
    assert!(commits.windows(2).all(|pair| pair[0] == pair[1]), "one changeset, one commit place: {commits:?}");
    write(&client, &scopes[1], vec![declare(DeclaredId::mint(), json!({ "frame": {} }))]).await;
    let heard = timeout(Duration::from_secs(2), watches[1].next()).await.expect("within 2 s").expect("an event");
    let ScopeEvent::Moved { commit, .. } = heard else { panic!("{heard:?}") };
    assert!(commit.expect("a commit place") > commits[0], "a later changeset takes a later place");
}
