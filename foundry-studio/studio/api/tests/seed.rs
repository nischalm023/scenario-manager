//! Spec 04 statement 18, the development seed (section 3.13), as far as mocks hold it: two fresh servers seeded from
//! `data/seed/studio/manifest.json` hold the same entities, ids included; run again, the seed skips every entry; a component
//! of ours the registry lacks stops it, naming the component. The run against dev.sh's own server is dev.sh's
//! seed unit, and a second run of `studio-seed` there.

mod support;

use std::sync::Arc;

use studio_protocol::{ScopePath, Seq};
use support::templates::{registered, repo, seeded};
use wps_client::seed::run as seed;
use wps_client::{BlockingCompute, Client, ClientConfig, MockWps, WorldStore};

/// Statement 18: the same world in both servers, every entry skipped again, and a missing component named.
#[tokio::test]
async fn the_seed_writes_one_world_skips_it_again_and_stops_on_a_missing_component() {
    let (first, second) = (seeded(registered()).await, seeded(registered()).await);
    let manifest = repo().join("data/seed/studio/manifest.json");
    let again = seed(&first, &manifest, None, "tests").await.expect("a second run");
    assert!(again.written.is_empty());
    assert_eq!(again.skipped.len(), 25, "every entry");
    for scope in ["template/site/l-shape/map", "catalog/artifact/relay-bay", "template/site/rectangle"] {
        let scope: ScopePath = scope.parse().unwrap();
        let (a, b) = (first.read(&scope, Seq::new(1)).await.unwrap(), second.read(&scope, Seq::new(1)).await.unwrap());
        assert_eq!((a.entities, a.prologue), (b.entities, b.prologue), "{scope}");
    }
    let bare = MockWps::phase_3();
    let client = Client::start(bare, ClientConfig::default(), Arc::new(BlockingCompute)).await.unwrap();
    let stopped = seed(&client, &manifest, None, "tests").await.expect_err("an unregistered component stops it");
    assert!(stopped.to_string().contains("template.knob"), "{stopped}");
}
