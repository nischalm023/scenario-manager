//! The template read's walk of its catalog: every scope the template's pins reach, through the catalog's own pins,
//! until no pin is new, and refused past the bound WPS holds a flagged export's scopes to.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::json;
use studio_protocol::{DeclaredId, ScopePath, Seq};
use wps_client::draft::Op;
use wps_client::wire::{RefAnnotation, WireChangeset};
use wps_client::wps::Wps;
use wps_client::{BlockingCompute, Client, ClientConfig, MockWps, WorldStore};

use super::{CLOSURE_LIMIT, Kept, ReadFault, catalog_of, read};

fn path(text: &str) -> ScopePath {
    text.parse().expect("a scope path")
}

/// A client of a mock whose `sector` names its members by reference, each where WPS's annotations read it.
async fn client() -> Client<MockWps> {
    let mock = MockWps::phase_3();
    let members = vec![RefAnnotation { path: "/members/*".to_owned(), expects: None }];
    mock.publish("sector", "0.7.0", json!({ "type": "object" }), members);
    let client = Client::start(mock, ClientConfig::default(), Arc::new(BlockingCompute)).await.expect("a client");
    for (prefix, boundary) in
        [("catalog", false), ("catalog/agent", false), ("template", false), ("template/site", false)]
    {
        client.ensure_scope(&path(prefix), boundary).await.expect("a prefix");
    }
    client
}

/// A scope of its own unit, `scope`, holding one sector `id` whose members are `members`, written at changeset 1.
async fn sector_scope(client: &Client<MockWps>, scope: &ScopePath, id: DeclaredId, members: Vec<String>) {
    client.ensure_scope(scope, true).await.expect("the scope");
    let sector = json!({ "sector": { "members": members } });
    let declare = Op::Declare { id, name: None, components: sector.as_object().cloned().expect("an object") };
    let changeset = WireChangeset::of_one_scope(scope.clone(), Seq::new(0), "a test", "a sector", vec![declare]);
    client.wps().post_changeset(&changeset).await.expect("the sector lands");
}

/// A pin to `id` at changeset 1.
fn pin(id: DeclaredId) -> String {
    format!("{id}@1")
}

/// A chain of six catalog items, each pinning the next, is followed to its last item, whose sector pins nothing: the
/// walk stops when no pin is new, not after a set number of hops.
#[tokio::test]
async fn the_catalog_is_followed_until_no_pin_is_new() {
    let client = client().await;
    let ids: Vec<DeclaredId> = (0..6).map(|_| DeclaredId::mint()).collect();
    for (index, id) in ids.iter().enumerate() {
        let members = ids.get(index + 1).map(|next| vec![pin(*next)]).unwrap_or_default();
        sector_scope(&client, &path(&format!("catalog/agent/item{index}")), *id, members).await;
    }
    let template = path("template/site/t");
    sector_scope(&client, &template, DeclaredId::mint(), vec![pin(ids[0])]).await;
    let own = vec![client.read(&template, Seq::new(1)).await.expect("the template")];
    let (catalog, notes) = catalog_of(&client, &own).await.expect("the catalog closes");
    let read: Vec<&str> = catalog.iter().map(|scope| scope.scope.as_str()).collect();
    assert_eq!(read, (0..6).map(|index| format!("catalog/agent/item{index}")).collect::<Vec<_>>());
    assert!(notes.is_empty(), "each item at its head: {notes:?}");
}

/// A template whose pins reach more scopes than a flagged export of WPS's holds is refused, naming the bound.
#[tokio::test]
async fn a_catalog_past_the_closure_limit_is_refused() {
    let client = client().await;
    let mut members = Vec::new();
    for index in 0..CLOSURE_LIMIT {
        let id = DeclaredId::mint();
        sector_scope(&client, &path(&format!("catalog/agent/item{index}")), id, Vec::new()).await;
        members.push(pin(id));
    }
    let template = path("template/site/t");
    sector_scope(&client, &template, DeclaredId::mint(), members).await;
    let own = vec![client.read(&template, Seq::new(1)).await.expect("the template")];
    match catalog_of(&client, &own).await {
        Err(ReadFault::TooLarge(limit)) => assert_eq!(limit, CLOSURE_LIMIT),
        other => panic!("{:?}", other.map(|(catalog, _)| catalog.len())),
    }
}

/// A template whose own scopes are more than a flagged export of WPS's holds is refused, naming the bound, though it
/// pins no catalog scope, and before any of its scopes is read: none of these was written, so reading one would fail.
#[tokio::test]
async fn a_template_of_more_scopes_than_the_bound_is_refused_though_it_pins_nothing() {
    let client = client().await;
    let template = path("template/site/t");
    let mut seqs = BTreeMap::from([(template.clone(), Seq::new(1))]);
    for index in 0..CLOSURE_LIMIT {
        seqs.insert(path(&format!("template/site/t/part{index}")), Seq::new(0));
    }
    match read(&Kept::default(), &client, &template, &seqs).await {
        Err(ReadFault::TooLarge(limit)) => assert_eq!(limit, CLOSURE_LIMIT),
        other => panic!("{:?}", other.map(|read| read.template.scopes.len())),
    }
}
