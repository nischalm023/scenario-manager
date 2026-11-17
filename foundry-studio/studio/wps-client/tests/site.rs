//! Spec 04 section 3.4 and spec 05 section 4.4 against the mock: Create site is one changeset that makes the missing
//! customer prefixes flagless, the root a boundary and its `map`, `flow` and `fleet` flagless, and writes all four with
//! their prologues from base 0, applied whole or refused whole; the plan it is built from checks clean against the
//! site's empty base; its body is built first and kept, and sent again, as after a restart, it is answered by WPS's
//! replay, or lands at that sending where the first never landed, while another plan cannot reuse it; a root that exists is `site-exists`,
//! and a prefix another writer made after the read asks for a body built again; the site list holds a root only once
//! written.

mod support;

use serde_json::json;
use studio_protocol::{DeclaredId, Seq};
use support::{client_of, declare, object, path, write};
use wps_client::site::{SitePlan, create_site, list_sites, site_changeset};
use wps_client::wire::WireChangeset;
use wps_client::{Applied, Author, Client, DocId, MockWps, NotLanded, ProblemKind, Prologue, WorldStore, WpsError};

/// The plan of `customer/{customer}/site/{name}`, named "Made {name}": a frame in its map, the primary one; a node in
/// its flow; a fleet entity; a card in its root.
fn plan(customer: &str, name: &str, operation: &str) -> SitePlan {
    let root = path(&format!("customer/{customer}/site/{name}"));
    let frame = DeclaredId::mint();
    let prologue = Prologue {
        doc: DocId::mint(),
        kind: Some("world".to_owned()),
        name: Some(format!("Made {name}")),
        primary_frame: Some(frame.into()),
        depends_on: Vec::new(),
    };
    let author = Author::new("approver").expect("an author");
    let mut plan = SitePlan::new(root.clone(), prologue, author, "Create site", operation).expect("a site's root");
    let in_root = |below: &str| path(&format!("{root}/{below}"));
    plan.declare(&in_root("map"), frame, Some("floor".to_owned()), object(json!({ "frame": {} }))).expect("the map");
    let node = json!({ "flow.node": { "kind": "storage", "sectors": [] } });
    plan.declare(&in_root("flow"), DeclaredId::mint(), None, object(node)).expect("the flow");
    plan.declare(&in_root("fleet"), DeclaredId::mint(), None, object(json!({ "external_ids": { "src": "fleet" } })))
        .expect("the fleet");
    let card = json!({ "template.description": { "title": "Made", "text": "made" } });
    plan.declare(&root, DeclaredId::mint(), None, object(card)).expect("the root");
    plan
}

async fn phase_3() -> Client<MockWps> {
    client_of(MockWps::phase_3()).await
}

/// Create site as the API runs it: the body built and kept, then sent.
async fn made(client: &Client<MockWps>, plan: &SitePlan) -> Result<(Applied, WireChangeset), WpsError> {
    let sent = site_changeset(client, plan).await?;
    Ok((create_site(client, plan, &sent).await.map_err(NotLanded::into_error)?, sent))
}

/// Create site on an empty server: one changeset creating every prefix, the root and its sub-scopes with their flags,
/// each of the four written at changeset 1 under the plan's operation, each sub-scope with a document of its own.
#[tokio::test]
async fn create_site_is_one_changeset() {
    let client = phase_3().await;
    let plan = plan("new", "s", "01JAZ0MADE");
    let draft = plan.draft(&client.subtree(&path("customer")).await.expect("a read")).expect("a draft");
    assert_eq!(draft.initiated_on, path("customer"), "initiated on the first scope it creates");
    assert_eq!(client.check(&draft, &plan.empty_base()).await.expect("check"), Vec::new());
    let before = client.wps().requests();
    let (applied, _) = made(&client, &plan).await.expect("made");
    assert_eq!(client.wps().requests() - before, 2, "one read, one changeset");
    let created: Vec<(String, bool)> =
        applied.created.iter().map(|scope| (scope.scope.to_string(), scope.boundary)).collect();
    let root = plan.root.to_string();
    let flags = [("customer", false), ("customer/new", false), ("customer/new/site", false)];
    let mut expected: Vec<(String, bool)> = flags.iter().map(|(scope, flag)| ((*scope).to_owned(), *flag)).collect();
    expected.push((root.clone(), true));
    expected.extend(["fleet", "flow", "map"].map(|sub| (format!("{root}/{sub}"), false)));
    assert_eq!(created, expected);
    for scope in plan.scopes() {
        assert_eq!(applied.seqs.get(&scope), Some(&Seq::new(1)), "{scope}");
        let read = client.read(&scope, Seq::new(1)).await.expect("the scope");
        assert_eq!(read.prologue.as_ref(), plan.prologues.get(&scope), "{scope} states its own prologue");
    }
    let sites = list_sites(&client, Some("new")).await.expect("the site list");
    assert_eq!(sites.iter().map(|site| site.name.clone()).collect::<Vec<_>>(), vec![Some("Made s".to_owned())]);
}

/// Create site under a customer whose scopes exist and hold entities: initiated on the root, it creates the root and
/// its sub-scopes only, the existing prefixes held to their flag alone.
#[tokio::test]
async fn create_site_below_a_customer_that_holds_entities() {
    let client = phase_3().await;
    for prefix in ["customer", "customer/held", "customer/held/site"] {
        client.ensure_scope(&path(prefix), false).await.expect("a prefix");
    }
    write(
        &client,
        &path("customer/held"),
        vec![declare(DeclaredId::mint(), json!({ "external_ids": { "src": "c" } }))],
    )
    .await;
    let plan = plan("held", "s", "01JAZ0HELD");
    let (applied, _) = made(&client, &plan).await.expect("made");
    assert_eq!(applied.created.len(), 4, "the root and its three sub-scopes");
    assert_eq!(applied.created[0].scope, plan.root);
}

/// A root that exists when the body is built is `site-exists`. A body built before another writer made the site, or a
/// prefix of it, is refused on the first scope it creates that exists, nothing stored under its identifier, and is
/// built again over a new read: `site-exists` for a made root, and the site made for a raced prefix.
#[tokio::test]
async fn an_existing_root_is_site_exists_and_a_raced_prefix_asks_for_a_new_body() {
    let client = phase_3().await;
    let rival = plan("c", "taken", "01JAZ0RIVAL");
    let early = site_changeset(&client, &rival).await.expect("a body built before the site stands");
    made(&client, &plan("c", "taken", "01JAZ0TAKEN")).await.expect("made");
    let refused = create_site(&client, &rival, &early).await.expect_err("its first created scope exists");
    assert_eq!(refused, NotLanded::Refused(WpsError::ScopeExists { scope: path("customer"), head: Seq::new(0) }));
    let taken = site_changeset(&client, &rival).await.expect_err("built again, the root stands");
    assert_eq!(taken, WpsError::ScopeExists { scope: path("customer/c/site/taken"), head: Seq::new(1) });
    assert_eq!(taken.api_problem().type_, "site-exists");

    let raced = plan("late", "s", "01JAZ0RACED");
    let first = site_changeset(&client, &raced).await.expect("a body");
    client.wps().make_scope_before_next_changeset(&path("customer/late"), false);
    let refused = create_site(&client, &raced, &first).await.expect_err("a prefix another writer made after the read");
    let late = WpsError::ScopeExists { scope: path("customer/late"), head: Seq::new(0) };
    assert_eq!(refused, NotLanded::Refused(late));
    let again = site_changeset(&client, &raced).await.expect("a body built again");
    let applied = create_site(&client, &raced, &again).await.expect("made over a new read");
    assert_eq!(applied.created.first().map(|scope| scope.scope.clone()), Some(path("customer/late/site")));
    assert!(raced.scopes().iter().all(|scope| applied.seqs.get(scope) == Some(&Seq::new(1))));
}

/// A Create site the server refuses (an id another site's map owns) leaves nothing behind: no prefix, no root,
/// nothing listed; the same name then makes the site.
#[tokio::test]
async fn a_refused_create_site_leaves_nothing() {
    let client = phase_3().await;
    let other = plan("other", "s", "01JAZ0OTHER");
    made(&client, &other).await.expect("made");
    let Some(owned) = other.ops.first().map(|scoped| scoped.op.entity().declared()) else { panic!("a declaration") };
    let mut refused = plan("gone", "s", "01JAZ0REFUSED");
    let map = path("customer/gone/site/s/map");
    refused.declare(&map, owned, None, object(json!({ "frame": {} }))).expect("the map");
    let WpsError::Refused { problem } = made(&client, &refused).await.expect_err("owned elsewhere") else {
        panic!("a refusal")
    };
    assert_eq!(problem.kind, ProblemKind::OwnedElsewhere);
    assert!(client.list_scopes(&path("customer/gone")).await.expect("a listing").is_empty(), "nothing created");
    assert!(list_sites(&client, Some("gone")).await.expect("the site list").is_empty());
    let (remade, _) = made(&client, &plan("gone", "s", "01JAZ0REMADE")).await.expect("the name is free");
    assert_eq!(remade.seqs.get(&map), Some(&Seq::new(1)));
}

/// Spec 05 section 9's retry: Create site called again with the plan and the body the API kept, as after a restart,
/// is answered by WPS's replay of the whole first answer, writing nothing; another plan cannot reuse the body, and
/// nothing is sent.
#[tokio::test]
async fn create_site_again_answers_the_site_it_made_and_refuses_a_changed_plan() {
    let client = phase_3().await;
    let plan = plan("c", "kept", "01JAZ0KEPT");
    let (first, sent) = made(&client, &plan).await.expect("made");
    let kept: SitePlan = serde_json::from_str(&serde_json::to_string(&plan).expect("JSON")).expect("a plan");
    assert_eq!(kept, plan, "the plan the API keeps reads back as it was");
    let body: WireChangeset = serde_json::from_str(&serde_json::to_string(&sent).expect("JSON")).expect("a body");
    let again = create_site(&client, &kept, &body).await.expect("the site it made");
    assert_eq!((again.seqs, again.created, again.replayed), (first.seqs, first.created, true));
    assert!(plan.scopes().iter().all(|scope| client.wps().head(scope) == Some(Seq::new(1))), "written once");

    let mut changed = kept.clone();
    changed.message = "Create another site".to_owned();
    let before = client.wps().requests();
    let refused = create_site(&client, &changed, &body).await.expect_err("another plan");
    let unsent =
        matches!(&refused, NotLanded::Unknown(WpsError::Invalid { text }) if text.contains("not the changeset"));
    assert!(unsent, "{refused:?}");
    assert_eq!(client.wps().requests(), before, "nothing was sent");
}

/// LEAD-DECISIONS entry 54: Create site sends the kept body as it is, reading no schema first, so a client whose cache
/// holds none of the plan's components (one just started) asks only for the scope tree the body is built over and
/// sends the changeset. Before, sending read `frame`'s schema first, a third request whose failure in a restarted API
/// passed for a refusal of a site WPS had made.
#[tokio::test]
async fn create_site_reads_no_schema_before_it_sends() {
    let client = phase_3().await;
    let plan = plan("cold", "s", "01JAZ0COLD");
    let before = client.wps().requests();
    let (applied, _) = made(&client, &plan).await.expect("made");
    assert_eq!(client.wps().requests() - before, 2, "the tree read and the changeset, no schema read");
    assert!(plan.scopes().iter().all(|scope| applied.seqs.get(scope) == Some(&Seq::new(1))));
}

/// Spec 05 section 9's retry, where the first sending never landed: both sendings lost before they applied leave the
/// outcome unknown, and the kept body sent again lands at that sending, as a first sending does.
#[tokio::test]
async fn a_create_site_that_never_landed_lands_when_sent_again() {
    let client = phase_3().await;
    let plan = plan("c", "late", "01JAZ0NEVER");
    let sent = site_changeset(&client, &plan).await.expect("a body");
    client.wps().drop_next_unapplied();
    client.wps().drop_next_unapplied();
    let lost = create_site(&client, &plan, &sent).await;
    assert!(matches!(lost, Err(NotLanded::Unknown(WpsError::OutcomeUnknown { .. }))), "{lost:?}");
    let landed = create_site(&client, &plan, &sent).await.expect("it lands at this sending");
    assert!(!landed.replayed && plan.scopes().iter().all(|scope| landed.seqs.get(scope) == Some(&Seq::new(1))));
}
