//! A member's view on a follower a panic has poisoned, which takes the hub's own lock and so stands beside `member`
//! rather than with the hubs' other tests.

use std::thread;

use tempfile::TempDir;

use super::Told;
use crate::frames::tests::key;
use crate::metric_streams::hub::{HubEnd, HubFault};
use crate::metric_streams::tests::{PICKS, canned, hubs, told, viewed};

/// A follower left poisoned by a panic as one socket's view opened fails its hub: the next view to open is refused
/// with the fault rather than panicking its socket's task, the hub's other member is told the hub failed, and the hub
/// is forgotten, so the next join starts afresh and is served.
#[tokio::test]
async fn a_follower_poisoned_by_a_panic_fails_its_hub_and_the_next_join_starts_afresh() {
    let dir = TempDir::new().unwrap();
    let (hubs, store) = hubs(&dir);
    let frames = store.open(&key(1));
    frames.push(canned()[0].1.clone()).await.expect("the scenario block");
    let mut opening = hubs.join(&key(1), PICKS, Ok(None)).member;
    let mut served = hubs.join(&key(1), PICKS, Ok(None)).member;
    viewed(&mut served).await.expect("served");
    let hub = opening.hub.clone();
    let panicked = thread::spawn(move || {
        let _held = hub.following.lock();
        panic!("a view panics as it opens");
    });
    panicked.join().expect_err("the view panicked holding the follower");
    let refused = opening.view().await;
    assert!(matches!(refused, Err(HubFault::Panicked(_))), "{refused:?}");
    // The notice that the scenario block is in reaches the served member unless its view opened before it went out.
    let failed = loop {
        match told(&mut served).await {
            Ok(Told::Described) => {}
            told => break told,
        }
    };
    assert!(matches!(failed, Ok(Told::Ended(HubEnd::Failed(HubFault::Panicked(_))))), "{failed:?}");
    assert_eq!(hubs.running(), 0, "forgotten while its members still hold it");
    let mut again = hubs.join(&key(1), PICKS, Ok(None)).member;
    viewed(&mut again).await.expect("a fresh hub's follower serves");
}
