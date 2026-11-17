//! The client's mock as the tests that draw a made site need it (brief I1.12; spec 03 sections 3.5 and 3.6, the marks
//! and the floor a made site's map answers): seeded by the development seed, its registry defining what the seed's
//! catalog carries, so an export of a site `plan_site` makes closes.

use std::sync::Arc;

use serde_json::json;

use wps_client::seed::run as seed;
use wps_client::{BlockingCompute, Client, ClientConfig, MockWps};

use super::templates::{registered, repo, seeded};

/// A mock seeded as `support::templates` seeds one, whose registry also defines `rack`, which the catalog's relay bay
/// carries and the engine's fixtures leave out: pure steel, referencing nothing, so an export of a made site closes.
pub async fn world() -> Client<MockWps> {
    seeded(rack_too()).await
}

/// The same, its client holding floors under `floor_budget` bytes, so a test can watch the floor cache let one go.
pub async fn world_holding(floor_budget: usize) -> Client<MockWps> {
    let config = ClientConfig { floor_budget, ..ClientConfig::default() };
    let client = Client::start(rack_too(), config, Arc::new(BlockingCompute)).await.expect("the client");
    let manifest = repo().join("data/seed/studio/manifest.json");
    seed(&client, &manifest, None, "tests").await.expect("the development seed writes into the mock");
    client
}

fn rack_too() -> MockWps {
    let mock = registered();
    mock.publish("rack", "0.7.0", json!({ "type": "object" }), Vec::new());
    mock
}
