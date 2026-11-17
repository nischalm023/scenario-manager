//! Spec 04 sections 3.3 and 3.12 against the mock: a configured revision and the server's listed features must agree
//! or the client stops naming both, and a server before scopes phase 3 or without structured refusals is refused at
//! start; a site read at one changeset of its anchor is refused until WPS can place its other scopes by commit order,
//! while a scope alone in its unit reads at any changeset it has; a cut reads its unit's heads by its subtree; and a
//! server without the export's flags is refused when a site is exported, naming the feature.

mod support;

use std::sync::Arc;

use serde_json::json;
use studio_protocol::{DeclaredId, Pin, Seq};
use support::{catalog, client, client_of, config, declare, path, register, site, write};
use wps_client::{BlockingCompute, Capability, Client, ClientConfig, MockWps, WorldStore, WpsError};

async fn start(mock: MockWps, revision: &str) -> Result<Client<MockWps>, WpsError> {
    register(&mock);
    let config = ClientConfig { revision: Some(revision.to_owned()), ..config() };
    Client::start(mock, config, Arc::new(BlockingCompute)).await
}

/// Section 3.12 and the lead's decision of 2026-10-19: cd4923516, configured against a server listing its features,
/// starts the client with scopes phase 3; configured against a server listing the first cut's, it stops naming both;
/// and a server without `scope_changesets`, listing the first cut's features or none, is refused at start, a
/// configured revision vouching for none.
#[tokio::test]
async fn a_server_before_scopes_phase_3_is_refused_at_start() {
    let started = start(MockWps::phase_3(), "cd4923516").await.expect("they agree");
    assert!(started.capabilities_now().has(Capability::MultiScopeChangesets));
    let refused = start(first_cut(), "cd4923516").await.err().expect("the revision and the features disagree");
    assert!(
        matches!(&refused, WpsError::Incompatible { text } if text.contains("MultiScopeChangesets")),
        "{refused:?}"
    );
    for mock in [first_cut(), MockWps::new()] {
        register(&mock);
        let refused = Client::start(mock, config(), Arc::new(BlockingCompute)).await.err().expect("refused");
        assert_eq!(refused, WpsError::Unsupported { capability: Capability::MultiScopeChangesets });
    }
    let refused = start(MockWps::new(), "cd4923516").await.err().expect("a server listing no features");
    assert_eq!(refused, WpsError::Unsupported { capability: Capability::MultiScopeChangesets });
}

/// The chore after world-proto b9d82a8 (LEAD-DECISIONS-IN-ABSENCE.md entry 49's rule for a server before 092d695): a
/// server whose `GET /protocol` lists scopes phase 3's features without `structured_refusals` answers refusals this
/// client cannot read by their codes, so it is refused at start, reads included.
#[tokio::test]
async fn a_server_without_structured_refusals_is_refused_at_start() {
    let mock = MockWps::new();
    mock.set_features(&[
        "generation",
        "scope_paths",
        "pin_rule",
        "registry_revision",
        "scope_listing_names",
        "exact_replace",
        "operation_id",
        "scope_changesets",
    ]);
    register(&mock);
    let refused = Client::start(mock, config(), Arc::new(BlockingCompute)).await.err().expect("refused");
    assert_eq!(refused, WpsError::Unsupported { capability: Capability::StructuredRefusals });
    assert!(refused.to_string().contains("refusal"), "{refused}");
}

/// A mock listing branch `server/first-cut`'s features, exact replace and the operation identifier without scopes
/// phase 3.
fn first_cut() -> MockWps {
    let mock = MockWps::new();
    mock.set_features(&["generation", "scope_paths", "pin_rule", "exact_replace", "operation_id"]);
    mock
}

/// Section 3.3 at scopes phase 3: a cut's heads come from one read of its unit's subtree, `GET /scopes?under=`: a
/// sub-scope anchors the cut of its root's unit, and a site beside it is in neither the cut nor the listing.
#[tokio::test]
async fn a_cut_reads_its_unit_by_its_subtree() {
    let client = client_of(MockWps::phase_3()).await;
    let root = site(&client, "under").await;
    let other = site(&client, "beside").await;
    write(&client, &other, vec![declare(DeclaredId::mint(), json!({ "external_ids": { "src": "b" } }))]).await;
    let cut = client.resolve_cut(&path(&format!("{root}/map")), &Pin::Branch("main".parse().expect("main"))).await;
    let cut = cut.expect("a cut");
    assert_eq!(cut.unit, root);
    let scopes: Vec<String> = cut.seqs.keys().map(ToString::to_string).collect();
    assert_eq!(scopes, ["", "/fleet", "/flow", "/map"].map(|below| format!("{root}{below}")));
    let listed: Vec<String> =
        client.list_scopes(&other).await.expect("a listing").iter().map(|s| s.path.to_string()).collect();
    assert_eq!(listed, ["", "/fleet", "/flow", "/map"].map(|below| format!("{other}{below}")));
}

/// Section 3.3: a site at one changeset of its anchor needs commit places, so it is refused naming the fixed cut
/// instead; a catalog item, alone in its unit, reads at its own changeset.
#[tokio::test]
async fn a_site_at_one_changeset_waits_for_commit_places() {
    let client = client().await;
    let root = site(&client, "numeric").await;
    write(&client, &root, vec![declare(DeclaredId::mint(), json!({ "external_ids": { "src": "r" } }))]).await;
    let refused = client.resolve_cut(&root, &Pin::Changeset(Seq::new(1))).await.expect_err("refused");
    assert_eq!(refused, WpsError::Unsupported { capability: Capability::ReadsAtVersion });
    assert!(refused.to_string().contains("fixed cut"), "{refused}");
    let item = catalog(&client, "numeric").await;
    write(&client, &item, vec![declare(DeclaredId::mint(), json!({ "robot_class": {} }))]).await;
    write(&client, &item, vec![declare(DeclaredId::mint(), json!({ "robot_class": {} }))]).await;
    let cut = client.resolve_cut(&item, &Pin::Changeset(Seq::new(1))).await.expect("a cut");
    assert_eq!(cut.seqs.into_iter().collect::<Vec<_>>(), vec![(item, Seq::new(1))]);
}

/// Section 3.12: a server that does not list `export_flags` has no route for a whole site, so the client refuses to
/// export one, naming the feature.
#[tokio::test]
async fn a_server_without_the_export_flags_is_refused_naming_them() {
    let mock = MockWps::phase_3();
    mock.set_features(&["generation", "scope_paths", "scope_changesets", "structured_refusals"]);
    let client = client_of(mock).await;
    let root = site(&client, "unsupported").await;
    let cut = client.resolve_cut(&root, &Pin::Branch("main".parse().expect("main"))).await.expect("a cut");
    let refused = client.export(&cut).await.expect_err("no flagged export to read");
    assert_eq!(refused, WpsError::Unsupported { capability: Capability::ExportFlags });
    assert!(refused.to_string().contains("export_flags"), "{refused}");
}
