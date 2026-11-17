//! Spec 04 sections 3.4, 3.7, 3.8 and 3.9 against the mock: a draft is one changeset across its scopes, applied whole
//! or refused whole; every write names its operation identifier and needs scopes phase 3, or nothing is sent; applying
//! a draft again, or losing its answer once, is answered by WPS's replay of the whole first answer, and after a second
//! loss the body its caller kept is sent again (`resend`), which a changed draft cannot reuse; a read base may name a
//! registry family; scopes phase 3's refusals read as their kinds; and respellings are reported as WPS owes them.

mod support;

use std::collections::BTreeMap;

use semver::Version;
use serde_json::json;
use studio_protocol::EntityId;
use studio_protocol::{DeclaredId, ScopePath, Seq};
use support::{client_of, declare, object, path, read_site, site, write};
use wps_client::apply::{NotLanded, OPERATION_CHARS, changeset_of};
use wps_client::draft::{NewScope, Op, ScopedOp};
use wps_client::wire::Respelled;
use wps_client::wps::Wps;
use wps_client::{
    Author, ChangesetDraft, DocId, DraftBuilder, GeneratorSet, MockWps, ProblemKind, Prologue, Reference, UnitSnapshot,
    WorldStore, WpsError,
};

fn sub(root: &ScopePath, below: &str) -> ScopePath {
    path(&format!("{root}/{below}"))
}

async fn draft_of(
    base: &UnitSnapshot,
    root: &ScopePath,
    operation: &str,
    build: impl FnOnce(&mut DraftBuilder<'_>),
) -> ChangesetDraft {
    let author = Author::new("approver").expect("an author");
    let mut builder = DraftBuilder::new(base, root.clone(), author, "A site edit", operation);
    build(&mut builder);
    builder.build()
}

fn prologue(name: &str) -> Prologue {
    Prologue {
        doc: DocId::mint(),
        kind: Some("world".to_owned()),
        name: Some(name.to_owned()),
        primary_frame: None,
        depends_on: Vec::new(),
    }
}

/// Section 3.4: a draft writing a site's four scopes is one changeset: each scope at its first changeset, each log
/// entry carrying the draft's operation identifier, and one answer naming every part.
#[tokio::test]
async fn a_draft_across_a_site_is_one_changeset() {
    let client = client_of(MockWps::phase_3()).await;
    let root = site(&client, "whole").await;
    let base = read_site(&client, &root).await;
    let draft = draft_of(&base, &root, "01JAZ0WHOLE", |builder| {
        for below in ["map", "flow", "fleet"] {
            builder.declare(&sub(&root, below), None, object(json!({ "external_ids": { "src": below } })));
        }
        builder.declare(&root, None, object(json!({ "template.description": { "title": "W", "text": "w" } })));
    })
    .await;
    // The schema cache loads `external_ids` on first use; that read is the cache's, never the write's.
    client.registry().ensure(client.wps(), "external_ids").await.expect("its schema");
    let before = client.wps().requests();
    let applied = client.apply(&draft).await.expect("applied");
    assert_eq!(client.wps().requests() - before, 1, "one changeset, sent once");
    let expected: BTreeMap<ScopePath, Seq> = [&root, &sub(&root, "map"), &sub(&root, "flow"), &sub(&root, "fleet")]
        .map(|scope| (scope.clone(), Seq::new(1)))
        .into();
    assert_eq!(applied.seqs, expected);
    for scope in expected.keys() {
        let entry = client.wps().changeset_named(scope, "01JAZ0WHOLE").await.expect("the log").expect("named");
        assert_eq!(entry.seq, Seq::new(1), "{scope}");
    }
}

/// Section 3.4: a changeset one of whose scopes moved past its base is refused whole, naming that scope: nothing lands
/// in the scopes that did not move.
#[tokio::test]
async fn a_moved_scope_refuses_the_whole_changeset() {
    let client = client_of(MockWps::phase_3()).await;
    let root = site(&client, "atomic").await;
    let base = read_site(&client, &root).await;
    let draft = draft_of(&base, &root, "01JAZ0ATOMIC", |builder| {
        builder.declare(&sub(&root, "map"), None, object(json!({ "frame": {} })));
        builder.declare(&sub(&root, "flow"), None, object(json!({ "flow.return_rule": { "min_backlog": 1 } })));
    })
    .await;
    write(&client, &sub(&root, "fleet"), vec![declare(DeclaredId::mint(), json!({ "external_ids": { "src": "x" } }))])
        .await;
    let refused = client.apply(&draft).await.expect_err("the fleet it read moved");
    assert_eq!(refused, WpsError::HeadMoved { moved: vec![(sub(&root, "fleet"), Seq::new(0), Seq::new(1))] });
    assert_eq!(client.wps().head(&sub(&root, "map")), Some(Seq::new(0)), "nothing written");
    assert_eq!(client.wps().head(&sub(&root, "flow")), Some(Seq::new(0)), "nothing written");
}

/// Section 3.9: a draft applied again, after a read base moved, is answered by WPS's replay of the whole first answer.
#[tokio::test]
async fn a_draft_applied_again_is_replayed_whole() {
    let client = client_of(MockWps::phase_3()).await;
    let root = site(&client, "again").await;
    let base = read_site(&client, &root).await;
    let draft = draft_of(&base, &root, "01JAZ0AGAIN", |builder| {
        builder.declare(&sub(&root, "flow"), None, object(json!({ "flow.return_rule": { "min_backlog": 2 } })));
        builder.declare(&sub(&root, "fleet"), None, object(json!({ "external_ids": { "src": "f" } })));
    })
    .await;
    let first = client.apply(&draft).await.expect("applied");
    write(&client, &sub(&root, "map"), vec![declare(DeclaredId::mint(), json!({ "frame": {} }))]).await;
    let again = client.apply(&draft).await.expect("the replay");
    assert_eq!((again.seqs, again.replayed), (first.seqs, true));
    assert_eq!(client.wps().head(&sub(&root, "flow")), Some(Seq::new(1)), "written once");
}

/// Section 3.9: an answer lost once is settled by the resend under the same identifier, which WPS answers with its
/// replay of the first sending; the draft is written once.
#[tokio::test]
async fn a_lost_answer_is_settled_by_the_replay_of_its_resend() {
    let client = client_of(MockWps::phase_3()).await;
    let root = site(&client, "lost").await;
    let base = read_site(&client, &root).await;
    let draft = draft_of(&base, &root, "01JAZ0LOST", |builder| {
        builder.declare(&sub(&root, "map"), None, object(json!({ "frame": {} })));
    })
    .await;
    client.wps().lose_next_answer();
    let applied = client.apply(&draft).await.expect("settled by the replay");
    assert_eq!((applied.seqs.get(&sub(&root, "map")), applied.replayed), (Some(&Seq::new(1)), true));
    assert_eq!(client.wps().head(&sub(&root, "map")), Some(Seq::new(1)), "written once");
}

/// Section 3.9: a write that landed and lost both answers is unknown; once a scope it read has moved, the body its
/// caller kept, sent again, is answered by WPS's replay of the whole first answer, never `head-moved`.
#[tokio::test]
async fn a_lost_answer_after_a_landed_write_recovers_by_replay() {
    let client = client_of(MockWps::phase_3()).await;
    let root = site(&client, "settle").await;
    let base = read_site(&client, &root).await;
    let draft = draft_of(&base, &root, "01JAZ0SETTLE", |builder| {
        builder.declare(&sub(&root, "flow"), None, object(json!({ "flow.return_rule": { "min_backlog": 3 } })));
    })
    .await;
    let kept = changeset_of(&draft).expect("the body the approval keeps");
    client.wps().lose_next_answer();
    client.wps().lose_next_answer();
    let unknown = client.apply(&draft).await.expect_err("both answers lost");
    assert_eq!(unknown, WpsError::OutcomeUnknown { scopes: vec![sub(&root, "flow")] });
    write(&client, &sub(&root, "map"), vec![declare(DeclaredId::mint(), json!({ "frame": {} }))]).await;
    let settled = client.resend(&kept, &draft).await.expect("settled by the replay");
    assert_eq!((settled.seqs.get(&sub(&root, "flow")), settled.replayed), (Some(&Seq::new(1)), true));
    assert_eq!(client.wps().head(&sub(&root, "flow")), Some(Seq::new(1)), "written once");
}

/// Section 3.9: a draft that only creates a scope writes none, so its outcome rests on WPS's replay alone: both
/// answers lost leave the created scope in doubt, and the kept body sent again answers the creation, replayed.
#[tokio::test]
async fn a_create_only_draft_recovers_by_replay() {
    let client = client_of(MockWps::phase_3()).await;
    let root = site(&client, "create").await;
    let base = read_site(&client, &root).await;
    let extra = sub(&root, "extra");
    let draft = draft_of(&base, &root, "01JAZ0CREATE", |builder| {
        builder.create(NewScope { path: extra.clone(), boundary: false });
    })
    .await;
    assert!(draft.written().is_empty(), "it writes no scope");
    let kept = changeset_of(&draft).expect("the body the approval keeps");
    client.wps().lose_next_answer();
    client.wps().lose_next_answer();
    let unknown = client.apply(&draft).await.expect_err("both answers lost");
    assert_eq!(unknown, WpsError::OutcomeUnknown { scopes: vec![extra.clone()] });
    let settled = client.resend(&kept, &draft).await.expect("settled by the replay");
    let created: Vec<ScopePath> = settled.created.iter().map(|scope| scope.scope.clone()).collect();
    assert_eq!((created, settled.replayed), (vec![extra.clone()], true));
    assert_eq!(client.wps().head(&extra), Some(Seq::new(0)), "created once, written never");
}

/// Section 3.9: a kept body is sent again only for the draft that made it: another draft is refused, and nothing is
/// sent.
#[tokio::test]
async fn a_changed_draft_cannot_reuse_its_kept_body() {
    let client = client_of(MockWps::phase_3()).await;
    let root = site(&client, "changed").await;
    let base = read_site(&client, &root).await;
    let draft = draft_of(&base, &root, "01JAZ0CHANGED", |builder| {
        builder.declare(&sub(&root, "map"), None, object(json!({ "frame": {} })));
    })
    .await;
    let kept = changeset_of(&draft).expect("the body the approval keeps");
    let changed = ChangesetDraft { message: "Another edit".to_owned(), ..draft };
    let before = client.wps().requests();
    let refused = client.resend(&kept, &changed).await.expect_err("another draft");
    let unsent =
        matches!(&refused, NotLanded::Unknown(WpsError::Invalid { text }) if text.contains("not the changeset"));
    assert!(unsent, "{refused:?}");
    assert_eq!(client.wps().requests(), before, "nothing was sent");
}

/// Section 3.9: an operation identifier that is empty, or past WPS's 128 characters, is refused before anything is
/// sent: every write names one.
#[tokio::test]
async fn an_empty_or_overlong_identifier_is_refused_before_anything_is_sent() {
    let client = client_of(MockWps::phase_3()).await;
    let root = site(&client, "named").await;
    let base = read_site(&client, &root).await;
    let map = sub(&root, "map");
    for (operation, chars) in [(String::new(), 0), ("o".repeat(OPERATION_CHARS + 1), OPERATION_CHARS + 1)] {
        let draft = draft_of(&base, &root, &operation, |builder| {
            builder.declare(&map, None, object(json!({ "frame": {} })));
        })
        .await;
        let before = client.wps().requests();
        let refused = client.apply(&draft).await.expect_err("refused");
        let named = format!("{chars} characters");
        assert!(matches!(&refused, WpsError::Invalid { text } if text.contains(&named)), "{refused:?}");
        assert_eq!(client.wps().requests(), before, "nothing was sent");
    }
    assert_eq!(client.wps().head(&map), Some(Seq::new(0)), "nothing was written");
}

/// Section 3.8, scopes phase 3's 422s: a changeset initiated on the map naming the flow, its sibling, and one
/// creating a scope it states at a base other than 0, each refused whole and read as its arm; at 0 it applies.
#[tokio::test]
async fn a_sibling_or_a_created_scope_off_zero_is_refused() {
    let client = client_of(MockWps::phase_3()).await;
    let root = site(&client, "shape").await;
    let base = read_site(&client, &root).await;
    let (map, flow) = (sub(&root, "map"), sub(&root, "flow"));
    let id = DeclaredId::mint();
    let sibling = ChangesetDraft {
        initiated_on: map.clone(),
        bases: base.cut.seqs.clone(),
        creates: Vec::new(),
        prologues: BTreeMap::new(),
        author: Author::new("approver").expect("an author"),
        message: "a sibling".to_owned(),
        ops: vec![ScopedOp {
            scope: flow.clone(),
            op: Op::Declare { id, name: None, components: object(json!({ "frame": {} })) },
        }],
        expect_generators: None,
        operation: "01JAZ0SIBLING".to_owned(),
        minted: vec![id],
    };
    let WpsError::Refused { problem } = client.apply(&sibling).await.expect_err("a sibling") else {
        panic!("a refusal")
    };
    assert_eq!((problem.kind, problem.scope), (ProblemKind::OutsideTheInitiatingScope, Some(flow.clone())));
    let extra = sub(&root, "extra");
    let mut off_zero = ChangesetDraft { initiated_on: root.clone(), operation: "01JAZ0OFFZERO".to_owned(), ..sibling };
    off_zero.creates.push(NewScope { path: extra.clone(), boundary: false });
    off_zero.bases.insert(extra.clone(), Seq::new(3));
    off_zero.prologues.insert(extra.clone(), prologue("Extra"));
    let WpsError::Refused { problem } = client.apply(&off_zero).await.expect_err("off 0") else { panic!("a refusal") };
    assert_eq!((problem.kind, problem.scope), (ProblemKind::CreatedBaseNotZero, Some(extra.clone())));
    assert_eq!((client.wps().head(&flow), client.wps().head(&extra)), (Some(Seq::new(0)), None), "nothing written");
    off_zero.bases.insert(extra.clone(), Seq::new(0));
    let landed = client.apply(&off_zero).await.expect("a scope created and written beside the flow");
    assert_eq!((landed.seqs.get(&extra), landed.created.len()), (Some(&Seq::new(1)), 1));
    assert_eq!(client.wps().head(&flow), Some(Seq::new(1)));
}

/// Section 3.4: a read base may name a family of the registry, the generator family an evaluation read, held to the
/// family's head as a scope's is: a publish past it refuses the changeset 409 naming the family, and at the head the
/// changeset applies.
#[tokio::test]
async fn a_registry_family_read_base_is_held_to_its_head() {
    let client = client_of(MockWps::phase_3()).await;
    let root = site(&client, "family").await;
    let base = read_site(&client, &root).await;
    let family = path("registry/generators");
    let read = Seq::new(client.wps().generators().await.expect("the generator family").at);
    let mut draft = draft_of(&base, &root, "01JAZ0FAMILY", |builder| {
        builder.declare(&sub(&root, "map"), None, object(json!({ "frame": {} })));
    })
    .await;
    draft.bases.insert(family.clone(), read);
    client.wps().publish_generator("tiling", "1.1.0", "digest-tiling-2");
    let head = Seq::new(read.get() + 1);
    let refused = client.apply(&draft).await.expect_err("the family moved past the base");
    assert_eq!(refused, WpsError::HeadMoved { moved: vec![(family.clone(), read, head)] });
    draft.bases.insert(family, head);
    let applied = client.apply(&draft).await.expect("at the family's head, under the identifier nothing stored");
    assert_eq!(applied.seqs.get(&sub(&root, "map")), Some(&Seq::new(1)));
}

/// Section 3.8: a stated generator set other than the one WPS would run is refused 409, read as `GeneratorsMoved`
/// with the set that runs; the set the cache holds is the one that runs, so stating it applies.
#[tokio::test]
async fn a_mismatching_generator_set_is_refused() {
    let client = client_of(MockWps::phase_3()).await;
    client.wps().publish("tiling", "0.7.0", json!({ "type": "object" }), Vec::new());
    client.wps().anchor_generator("tiling", "tiling");
    client.wps().publish_generator("unnamed", "1.0.0", "digest-unnamed-1");
    client.registry().revalidate(client.wps()).await.expect("the generator set read again");
    let running = client.registry().snapshot().generators.clone();
    assert_eq!(
        running.0.keys().cloned().collect::<Vec<_>>(),
        vec!["tiling".to_owned()],
        "a generator no definition names never runs"
    );
    let root = site(&client, "gens").await;
    let base = read_site(&client, &root).await;
    let mut stale = running.clone();
    stale.0.insert("tiling".to_owned(), (Version::new(0, 9, 0), "digest-old".to_owned()));
    let build = |builder: &mut DraftBuilder<'_>| {
        builder.declare(&sub(&root, "map"), None, object(json!({ "tiling": { "pitch": [900, 900] } })));
    };
    let mut draft = draft_of(&base, &root, "01JAZ0GENS", build).await;
    draft.expect_generators = Some(stale.clone());
    let refused = client.apply(&draft).await.expect_err("the stated set differs");
    assert_eq!(refused, WpsError::GeneratorsMoved { expected: stale, found: running.clone() });
    assert_eq!(refused.api_problem().type_, "generators-moved");
    draft.expect_generators = Some(running);
    client.apply(&draft).await.expect("the set that runs, stated, applies");
    let mut idle = draft_of(&read_site(&client, &root).await, &root, "01JAZ0IDLE", |builder| {
        builder.declare(&sub(&root, "flow"), None, object(json!({ "flow.return_rule": { "min_backlog": 1 } })));
    })
    .await;
    idle.expect_generators = Some(GeneratorSet::default());
    client.apply(&idle).await.expect("a changeset running no generator states none");
}

/// Spec 04 section 3.7's respelling: `@head` is written bare only at an annotated place, so an external key that merely
/// reads as a reference is stored as written, and the apply that wrote both reports no respelling it was not owed.
#[tokio::test]
async fn head_is_respelled_only_where_annotated() {
    let client = client_of(MockWps::phase_3()).await;
    let root = site(&client, "respell").await;
    let sector = DeclaredId::mint();
    write(&client, &sub(&root, "map"), vec![declare(sector, json!({ "sector": { "members": [] } }))]).await;
    let base = read_site(&client, &root).await;
    let literal = format!("{}@head", DeclaredId::mint());
    let flow = sub(&root, "flow");
    let mut node = None;
    let draft = draft_of(&base, &root, "01JAZ0RESPELL", |builder| {
        let kin = builder.reference(&Reference::Kin(sector.into()));
        let components =
            json!({ "flow.node": { "kind": "storage", "sectors": [kin] }, "external_ids": { "src": literal } });
        node = Some(builder.declare(&flow, None, object(components)));
    })
    .await;
    let applied = client.apply(&draft).await.expect("applied");
    assert!(applied.warnings.is_empty(), "{:?}", applied.warnings);
    let node: EntityId = node.expect("declared").into();
    let held = client.entity(&flow, &node, Seq::new(1)).await.expect("the node");
    assert_eq!(held.components["external_ids"]["src"], json!(literal), "an unannotated string is stored as written");
    assert_eq!(held.components["flow.node"]["sectors"][0], json!(sector.to_string()), "a kin reference is stored bare");
}

/// Spec 04 section 3.7: a respelling WPS reports and does not owe is a warning; what landed stands.
#[tokio::test]
async fn an_unowed_respelling_is_reported_and_the_write_stands() {
    let client = client_of(MockWps::phase_3()).await;
    let root = site(&client, "unowed").await;
    let base = read_site(&client, &root).await;
    let draft = draft_of(&base, &root, "01JAZ0UNOWED", |builder| {
        builder.declare(&sub(&root, "map"), None, object(json!({ "frame": {} })));
    })
    .await;
    client.wps().report_respelling(Respelled {
        entity: DeclaredId::mint().to_string(),
        component: "frame".to_owned(),
        path: "/parent".to_owned(),
        written: "x@head".to_owned(),
        stored: "x".to_owned(),
    });
    let applied = client.apply(&draft).await.expect("the write stands");
    assert_eq!(applied.seqs.get(&sub(&root, "map")), Some(&Seq::new(1)));
    assert!(applied.warnings.iter().any(|warning| warning.contains("did not expect")), "{:?}", applied.warnings);
}
