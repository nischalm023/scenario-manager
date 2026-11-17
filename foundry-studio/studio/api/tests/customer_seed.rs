//! The customer seed (`data/seed/customer/manifest.json`) in a store, as far as the mock of world-proto-server holds
//! it: seeded fresh, the Cencora site is ready and a plan of it compiles, its pick stations' sector naming each
//! station by the derived address its stamp expands to; seeded over a Cencora map from before the site stated its
//! flow, the seed writes nothing and names the map.

mod support;

use std::env;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};
use studio_api::scenarios::SiteReader;
use studio_protocol::{DeclaredId, ScopePath, SitePath};
use support::runs::{config, experiment, plan, scenario, send, stack_over};
use support::templates::{registered, repo};
use wps_client::seed::{SeedError, run as seed};
use wps_client::wire::RefAnnotation;
use wps_client::wps::Wps;
use wps_client::{BlockingCompute, Client, ClientConfig, MockWps};

const SITE: &str = "customer/cencora/site/ndc2";

fn read(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|fault| panic!("{}: {fault}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|fault| panic!("{}: {fault}", path.display()))
}

/// The development seed's mock, whose registry also defines the components the customer seed carries and the
/// development seed does not (core components, and three of Agent Studio's), each with the annotations
/// `data/seed/application` gives it, as the development WPS's own seed registers them.
async fn store() -> Client<MockWps> {
    let mock = registered();
    let application = repo().join("data/seed/application");
    let core = read(&application.join("core-components.json"));
    let mut definitions: Vec<(String, &str, Value)> =
        core.as_object().unwrap().iter().map(|(name, entry)| (name.clone(), "0.7.0", entry["refs"].clone())).collect();
    for name in ["operator", "fence", "light_curtain"] {
        let entry = read(&application.join(format!("components/{name}.json")));
        definitions.push((name.to_owned(), "0.1.0", entry["refs"].clone()));
    }
    for (name, version, refs) in definitions {
        if mock.component(&name).await.expect("the mock's registry").is_none() {
            let refs: Vec<RefAnnotation> = serde_json::from_value(refs).expect("annotations");
            mock.publish(&name, version, json!({}), refs);
        }
    }
    Client::start(mock, ClientConfig::default(), Arc::new(BlockingCompute)).await.expect("the mock's client")
}

fn below(scope: &str) -> ScopePath {
    format!("{SITE}/{scope}").parse().expect("a scope")
}

/// Seeded fresh from the customer manifest: the site's export names the pick stations' sector members as derived
/// addresses, Studio's readiness passes, and a plan of the site at its defaults compiles.
#[tokio::test]
async fn the_cencora_site_seeded_fresh_is_ready_and_plans() {
    let scratch = scratch_or_skip!();
    let world = store().await;
    seed(&world, &repo().join("data/seed/customer/manifest.json"), None, "tests").await.expect("the customer seed");
    let site: SitePath = SITE.parse().expect("a site");
    let export = SiteReader::export(&world, &SiteReader::resolve_cut(&world, &site).await.unwrap()).await.unwrap();
    let doc: Value = serde_json::from_slice(&export.bytes).expect("the export");
    let picks = doc["entities"].as_array().unwrap().iter().find(|entry| entry["name"] == "sector: pick stations");
    let members = picks.expect("the pick stations' sector")["components"]["sector"]["members"].as_array().unwrap();
    assert_eq!(members.len(), 10);
    assert!(
        members.iter().all(|member| member.as_str().unwrap().ends_with("/instance:0/local:station")),
        "{members:?}"
    );

    let stack = stack_over(&scratch.pool, config(), world, SITE.parse().unwrap()).await;
    let scenario = scenario(&stack, "Cencora at its defaults").await;
    let (_, readiness) = send(&stack.router, "GET", &format!("/api/scenarios/{scenario}/readiness"), None).await;
    assert_eq!(readiness["ready"], true, "{readiness}");
    let made = experiment(&stack, &scenario, 1, &[]).await;
    let (planned, _) = plan(&stack, &made.id).await;
    assert_eq!(planned.runs.len(), 1);
    scratch.close().await;
}

/// The Cencora map as it stood before the site stated its flow: without the sectors the flow's nodes name and without
/// its HTM class's drive kind.
fn map_before_the_flow(customer: &Path) -> Value {
    let flow = read(&customer.join("cencora/ndc2/flow.world.json"));
    let sectors: Vec<&Value> = flow["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["components"]["flow.node"]["sectors"].as_array())
        .flatten()
        .collect();
    let mut map = read(&customer.join("cencora/ndc2/map.world.json"));
    let entities = map["entities"].as_array_mut().unwrap();
    entities.retain(|entry| !sectors.contains(&&entry["id"]));
    for entry in entities {
        if let Some(drive) = entry.pointer_mut("/components/floor_drive").and_then(Value::as_object_mut) {
            drive.remove("kind");
        }
    }
    map
}

/// Seeded first with the map before the flow and the manifest without its flow entry, then from the customer
/// manifest: the seed stops before writing, naming the map, the flow it did not write and the reset.
#[tokio::test]
async fn the_customer_seed_writes_nothing_over_a_cencora_map_seeded_before_the_flow() {
    let world = store().await;
    let customer = repo().join("data/seed/customer");
    let earlier = env::temp_dir().join(format!("studio-customer-seed-{}", &DeclaredId::mint().to_string()[..8]));
    fs::create_dir_all(earlier.join("cencora/ndc2")).expect("a folder");
    fs::write(earlier.join("cencora/ndc2/map.world.json"), map_before_the_flow(&customer).to_string())
        .expect("written");
    fs::copy(customer.join("cencora/ndc2/root.world.json"), earlier.join("cencora/ndc2/root.world.json"))
        .expect("copied");
    let mut manifest = read(&customer.join("manifest.json"));
    manifest["entries"].as_array_mut().unwrap().retain(|entry| entry["scope"] != below("flow").to_string());
    fs::write(earlier.join("manifest.json"), manifest.to_string()).expect("written");
    seed(&world, &earlier.join("manifest.json"), None, "tests").await.expect("the seed before the flow");
    let _ = fs::remove_dir_all(&earlier);
    let before = world.tree().await.expect("the scope tree");

    let refused = seed(&world, &customer.join("manifest.json"), None, "tests").await;
    let (map, flow) = (below("map"), below("flow"));
    let Err(SeedError::UnmatchedHeldScope { held, unwritten, .. }) = &refused else { panic!("{refused:?}") };
    assert_eq!((held, unwritten), (&map, &flow));
    let message = refused.unwrap_err().to_string();
    let says =
        ["./dev.sh --stop --db", "deleting that stack's runs and the container's rows", "reset where it is kept"];
    assert!(says.iter().all(|what| message.contains(what)), "says what to do and what it deletes: {message}");
    let after = world.tree().await.expect("the scope tree");
    let root: ScopePath = SITE.parse().unwrap();
    for scope in [root, map] {
        assert_eq!(after.head(&scope), before.head(&scope), "{scope}: nothing written");
    }
    assert_eq!(after.head(&flow), None, "no flow scope made");
}
