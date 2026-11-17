//! Spec 04 section 8's statement 4: `check` reports each problem it owns with its entity, component and path, before
//! anything is sent, against a site of the sub-scope layout in the in-memory mock; WPS's resolve refusals become
//! located problems, and pins and deletions are judged over the reference graph the draft leaves.

mod support;

use std::collections::BTreeMap;

use serde_json::{Value, json};
use studio_protocol::{DeclaredId, EntityId, ScopePath, Seq};
use support::{catalog, client, client_of, declare, object, path, read_site, site, write};
use wps_client::draft::{ChangesetDraft, NewScope, Op, ScopedOp};
use wps_client::{
    Author, Client, DocId, DraftBuilder, MockWps, Problem, ProblemKind, Prologue, UnitSnapshot, WorldStore,
};

/// The site `customer/c/site/chk`: a frame and a sector in its map, a node naming the sector in its flow, a cohort
/// pinning the catalog's robot class at changeset 1 in its fleet; the catalog item at changeset 2; one spent id.
struct Fixture {
    client: Client<MockWps>,
    root: ScopePath,
    frame: DeclaredId,
    sector: DeclaredId,
    class: DeclaredId,
    spent: DeclaredId,
}

fn map(root: &ScopePath) -> ScopePath {
    path(&format!("{root}/map"))
}
fn flow(root: &ScopePath) -> ScopePath {
    path(&format!("{root}/flow"))
}
fn fleet(root: &ScopePath) -> ScopePath {
    path(&format!("{root}/fleet"))
}

async fn fixture_of(mock: MockWps) -> Fixture {
    let client = client_of(mock).await;
    let root = site(&client, "chk").await;
    let item = catalog(&client, "a").await;
    let (frame, sector, class, spent, node, cohort) = (
        DeclaredId::mint(),
        DeclaredId::mint(),
        DeclaredId::mint(),
        DeclaredId::mint(),
        DeclaredId::mint(),
        DeclaredId::mint(),
    );
    write(&client, &item, vec![declare(class, json!({ "robot_class": { "speed": 1 } }))]).await;
    write(
        &client,
        &item,
        vec![Op::Patch {
            id: class,
            name: None,
            components: support::object(json!({ "robot_class": { "speed": 2 } })),
        }],
    )
    .await;
    write(
        &client,
        &map(&root),
        vec![
            Op::Declare {
                id: frame,
                name: Some("floor".to_owned()),
                components: object(json!({ "frame": {}, "external_ids": { "src": "f" } })),
            },
            declare(sector, json!({ "sector": { "members": [frame.to_string()] } })),
            declare(spent, json!({ "frame": {} })),
        ],
    )
    .await;
    write(&client, &map(&root), vec![Op::Delete { id: spent }]).await;
    write(
        &client,
        &flow(&root),
        vec![declare(node, json!({ "flow.node": { "kind": "storage", "sectors": [format!("{sector}@head")] } }))],
    )
    .await;
    write(&client, &fleet(&root), vec![declare(cohort, json!({ "cohort": { "robot_class": format!("{class}@1") } }))])
        .await;
    Fixture { client, root, frame, sector, class, spent }
}

async fn fixture() -> Fixture {
    fixture_of(MockWps::phase_3()).await
}

fn draft(base: &UnitSnapshot, initiated_on: &ScopePath, ops: Vec<ScopedOp>) -> ChangesetDraft {
    ChangesetDraft {
        initiated_on: initiated_on.clone(),
        bases: base.cut.seqs.clone(),
        creates: Vec::new(),
        prologues: BTreeMap::new(),
        author: Author::new("tester").expect("an author"),
        message: "a test draft".to_owned(),
        ops,
        expect_generators: None,
        operation: "op-check".to_owned(),
        minted: Vec::new(),
    }
}

fn scoped(scope: &ScopePath, op: Op) -> ScopedOp {
    ScopedOp { scope: scope.clone(), op }
}

/// The one problem of `kind`, or a failure listing what was found.
fn found(problems: &[Problem], kind: ProblemKind) -> Problem {
    problems
        .iter()
        .find(|problem| problem.kind == kind)
        .cloned()
        .unwrap_or_else(|| panic!("no {kind} among {problems:#?}"))
}

fn at(problem: &Problem, entity: DeclaredId, component: &str, field: &str) {
    assert_eq!(problem.entity, Some(EntityId::from(entity)), "{problem:?}");
    assert_eq!(problem.component.as_deref(), Some(component), "{problem:?}");
    assert_eq!(problem.path.as_deref(), Some(field), "{problem:?}");
}

async fn check(fixture: &Fixture, draft: &ChangesetDraft, base: &UnitSnapshot) -> Vec<Problem> {
    fixture.client.check(draft, base).await.expect("check answers")
}

/// Statement 4: an operation outside the initiating scope's subtree, a missing base, two writes of one entity.
#[tokio::test]
async fn the_draft_shape_is_checked() {
    let fixture = fixture().await;
    let base = read_site(&fixture.client, &fixture.root).await;
    let id = DeclaredId::mint();
    let mut outside =
        draft(&base, &flow(&fixture.root), vec![scoped(&map(&fixture.root), declare(id, json!({ "frame": {} })))]);
    outside.minted.push(id);
    let problem = found(&check(&fixture, &outside, &base).await, ProblemKind::OutsideSubtree);
    assert_eq!((problem.scope, problem.entity), (Some(map(&fixture.root)), Some(id.into())));

    let mut no_base = draft(&base, &fixture.root, vec![]);
    no_base.bases.remove(&fleet(&fixture.root));
    assert_eq!(
        found(&check(&fixture, &no_base, &base).await, ProblemKind::MissingBase).scope,
        Some(fleet(&fixture.root))
    );

    let patch = |value: Value| Op::Patch { id: fixture.frame, name: None, components: object(value) };
    let twice = draft(
        &base,
        &fixture.root,
        vec![
            scoped(&map(&fixture.root), patch(json!({ "frame": { "dims": 2 } }))),
            scoped(&map(&fixture.root), patch(json!({ "external_ids": { "src": "g" } }))),
        ],
    );
    assert_eq!(
        found(&check(&fixture, &twice, &base).await, ProblemKind::DuplicateWrite).entity,
        Some(fixture.frame.into())
    );
}

/// Statement 4: a spent id, declared again by a draft that did not mint it.
#[tokio::test]
async fn a_spent_id_is_refused() {
    let fixture = fixture().await;
    let base = read_site(&fixture.client, &fixture.root).await;
    let again =
        draft(&base, &fixture.root, vec![scoped(&map(&fixture.root), declare(fixture.spent, json!({ "frame": {} })))]);
    assert_eq!(found(&check(&fixture, &again, &base).await, ProblemKind::SpentId).entity, Some(fixture.spent.into()));
}

/// Statement 4: a dangling reference, one in `stitch.pairs/*/*`, and one inside `template.entities`, which only the
/// walker's own list reaches.
#[tokio::test]
async fn references_that_name_nothing_are_dangling() {
    let fixture = fixture().await;
    let base = read_site(&fixture.client, &fixture.root).await;
    let nowhere = DeclaredId::mint();
    let (node, stitch, template) = (DeclaredId::mint(), DeclaredId::mint(), DeclaredId::mint());
    let mut drafted = draft(
        &base,
        &fixture.root,
        vec![
            scoped(
                &flow(&fixture.root),
                declare(node, json!({ "flow.node": { "kind": "storage", "sectors": [format!("{nowhere}@head")] } })),
            ),
            scoped(
                &map(&fixture.root),
                declare(
                    stitch,
                    json!({ "stitch": { "a": fixture.frame.to_string(), "b": fixture.frame.to_string(), "pairs": [[fixture.frame.to_string(), nowhere.to_string()]] } }),
                ),
            ),
            scoped(
                &map(&fixture.root),
                declare(
                    template,
                    json!({ "template": { "entities": [
            { "local": "a", "components": { "frame": { "parent": { "local_ref": "b" } } } }
        ] } }),
                ),
            ),
        ],
    );
    drafted.minted.extend([node, stitch, template]);
    let problems = check(&fixture, &drafted, &base).await;
    let dangling: Vec<&Problem> = problems.iter().filter(|problem| problem.kind == ProblemKind::Dangling).collect();
    let field = |entity: DeclaredId| {
        dangling
            .iter()
            .find(|problem| problem.entity == Some(entity.into()))
            .map(|problem| (problem.component.clone(), problem.path.clone()))
    };
    assert_eq!(field(node), Some((Some("flow.node".to_owned()), Some("/sectors/0".to_owned()))), "{problems:#?}");
    assert_eq!(field(stitch), Some((Some("stitch".to_owned()), Some("/pairs/0/1".to_owned()))), "{problems:#?}");
    let inside = Some((Some("template".to_owned()), Some("/entities/0/components/frame/parent".to_owned())));
    assert_eq!(field(template), inside, "{problems:#?}");
}

/// Statement 4: the pin rule mirrored: `LateralNeedsVersion`, `PinnedInsideBoundary`, `PinInTwoParts` and
/// `PinBeyondHead`, each at its field.
#[tokio::test]
async fn the_pin_rule_is_mirrored() {
    let fixture = fixture().await;
    let base = read_site(&fixture.client, &fixture.root).await;
    let ids: Vec<DeclaredId> = (0..4).map(|_| DeclaredId::mint()).collect();
    let mut drafted = draft(
        &base,
        &fixture.root,
        vec![
            scoped(
                &fleet(&fixture.root),
                declare(ids[0], json!({ "cohort": { "robot_class": fixture.class.to_string() } })),
            ),
            scoped(
                &flow(&fixture.root),
                declare(
                    ids[1],
                    json!({ "flow.node": { "kind": "storage", "sectors": [format!("{}@1", fixture.sector)] } }),
                ),
            ),
            scoped(
                &map(&fixture.root),
                declare(
                    ids[2],
                    json!({ "sector": { "members": [{ "generator": format!("{}@1", fixture.frame), "key": "cell:0,0" }] } }),
                ),
            ),
            scoped(
                &fleet(&fixture.root),
                declare(ids[3], json!({ "cohort": { "robot_class": format!("{}@9", fixture.class) } })),
            ),
        ],
    );
    drafted.minted.extend(ids.iter().copied());
    let problems = check(&fixture, &drafted, &base).await;
    at(&found(&problems, ProblemKind::LateralNeedsVersion), ids[0], "cohort", "/robot_class");
    at(&found(&problems, ProblemKind::PinnedInsideBoundary), ids[1], "flow.node", "/sectors/0");
    at(&found(&problems, ProblemKind::PinInTwoParts), ids[2], "sector", "/members/0");
    at(&found(&problems, ProblemKind::PinBeyondHead), ids[3], "cohort", "/robot_class");
}

/// Statement 4: `PinnedTwice` over the references the changeset leaves standing: a new cohort pinning the catalog
/// at 2 beside the standing one pinning it at 1.
#[tokio::test]
async fn two_pins_of_one_scope_are_refused() {
    let fixture = fixture().await;
    let base = read_site(&fixture.client, &fixture.root).await;
    let id = DeclaredId::mint();
    let mut drafted = draft(
        &base,
        &fixture.root,
        vec![scoped(
            &fleet(&fixture.root),
            declare(id, json!({ "cohort": { "robot_class": format!("{}@2", fixture.class) } })),
        )],
    );
    drafted.minted.push(id);
    assert_eq!(
        found(&check(&fixture, &drafted, &base).await, ProblemKind::PinnedTwice).scope,
        Some(fleet(&fixture.root))
    );
}

/// Statement 4: U+0000, a schema the payload does not satisfy, and a body over 64 MiB.
#[tokio::test]
async fn payloads_are_checked_as_wps_would() {
    let fixture = fixture().await;
    let base = read_site(&fixture.client, &fixture.root).await;
    let (nul, invalid, large) = (DeclaredId::mint(), DeclaredId::mint(), DeclaredId::mint());
    let mut drafted = draft(
        &base,
        &fixture.root,
        vec![
            scoped(&fixture.root, declare(nul, json!({ "template.description": { "title": "a\u{0}b", "text": "t" } }))),
            scoped(&fixture.root, declare(invalid, json!({ "template.description": { "title": "no text" } }))),
            scoped(
                &flow(&fixture.root),
                declare(large, json!({ "flow.return_rule": { "min_backlog": 1, "pad": "x".repeat(65 << 20) } })),
            ),
        ],
    );
    drafted.minted.extend([nul, invalid, large]);
    let problems = check(&fixture, &drafted, &base).await;
    at(&found(&problems, ProblemKind::Nul), nul, "template.description", "");
    at(&found(&problems, ProblemKind::SchemaInvalid), invalid, "template.description", "");
    // The limit is the whole changeset's, named by the scope it is initiated on (wp:server/DESIGN.md section 8).
    assert_eq!(found(&problems, ProblemKind::TooLarge).scope, Some(fixture.root.clone()));
}

/// Statement 4: an overwrite that leaves a component out is a `Replace`, WPS's exact replace, which removes it, so the
/// check finds nothing to refuse.
#[tokio::test]
async fn an_overwrite_is_an_exact_replace() {
    let fixture = fixture().await;
    let base = read_site(&fixture.client, &fixture.root).await;
    let author = Author::new("tester").expect("an author");
    let mut builder = DraftBuilder::new(&base, fixture.root.clone(), author, "overwrite", "op-overwrite");
    builder
        .overwrite(fixture.frame, Some("floor".to_owned()), object(json!({ "frame": { "dims": 2 } })))
        .expect("a kept entity");
    let drafted = builder.build();
    assert!(matches!(drafted.ops[0].op, Op::Replace { .. }));
    let problems = check(&fixture, &drafted, &base).await;
    assert!(problems.is_empty(), "{problems:#?}");
}

/// Statement 4: a created root without a prologue, and a `primary_frame` naming nothing.
#[tokio::test]
async fn a_new_root_states_its_prologue() {
    let client = client().await;
    let root = site(&client, "fresh").await;
    let base = read_site(&client, &root).await;
    let card = DeclaredId::mint();
    let mut drafted = draft(
        &base,
        &root,
        vec![scoped(&root, declare(card, json!({ "template.description": { "title": "t", "text": "x" } })))],
    );
    drafted.minted.push(card);
    let problems = client.check(&drafted, &base).await.expect("check answers");
    assert_eq!(found(&problems, ProblemKind::MissingPrologue).scope, Some(root.clone()));

    let nothing: EntityId = DeclaredId::mint().into();
    let prologue = Prologue {
        doc: DocId::mint(),
        kind: Some("world".to_owned()),
        name: Some("Fresh".to_owned()),
        primary_frame: Some(nothing.clone()),
        depends_on: Vec::new(),
    };
    drafted.prologues.insert(root.clone(), prologue);
    let problems = client.check(&drafted, &base).await.expect("check answers");
    assert!(problems.iter().all(|problem| problem.kind != ProblemKind::MissingPrologue), "{problems:#?}");
    assert_eq!(found(&problems, ProblemKind::Dangling).entity, Some(nothing));
}

fn site_prologue(frame: Option<EntityId>) -> Prologue {
    Prologue {
        doc: DocId::mint(),
        kind: Some("world".to_owned()),
        name: Some("Checked".to_owned()),
        primary_frame: frame,
        depends_on: Vec::new(),
    }
}

/// Statement 4 and section 3.3's closure: a site's root prologue names its primary frame, and one the draft deletes
/// leaves it dangling.
#[tokio::test]
async fn a_site_prologue_names_a_frame_the_draft_keeps() {
    let fixture = fixture().await;
    let base = read_site(&fixture.client, &fixture.root).await;
    let mut frameless = draft(&base, &fixture.root, vec![]);
    frameless.prologues.insert(fixture.root.clone(), site_prologue(None));
    let problems = check(&fixture, &frameless, &base).await;
    assert_eq!(found(&problems, ProblemKind::MissingPrologue).scope, Some(fixture.root.clone()));

    let mut deleting = draft(&base, &fixture.root, vec![scoped(&map(&fixture.root), Op::Delete { id: fixture.frame })]);
    deleting.prologues.insert(fixture.root.clone(), site_prologue(Some(fixture.frame.into())));
    let problems = check(&fixture, &deleting, &base).await;
    let dangling = problems
        .iter()
        .find(|problem| problem.kind == ProblemKind::Dangling && problem.scope == Some(fixture.root.clone()));
    assert_eq!(dangling.and_then(|problem| problem.entity.clone()), Some(fixture.frame.into()), "{problems:#?}");
}

/// Section 3.7, step 2: what WPS's resolution refuses reaches the draft as a located problem: a kin reference to an
/// entity deleted at head, a pin naming a changeset where its target was not live; and a referrer the draft creates is
/// resolved from a scope that exists, its owner found.
#[tokio::test]
async fn resolve_refusals_are_located_problems() {
    let fixture = fixture().await;
    let item = path("catalog/agent/b");
    fixture.client.ensure_scope(&item, true).await.expect("a second catalog item");
    let gone = DeclaredId::mint();
    write(&fixture.client, &item, vec![declare(gone, json!({ "robot_class": {} }))]).await;
    write(&fixture.client, &item, vec![Op::Delete { id: gone }]).await;
    let base = read_site(&fixture.client, &fixture.root).await;
    let extra = path(&format!("{}/extra", fixture.root));
    let ids: Vec<DeclaredId> = (0..4).map(|_| DeclaredId::mint()).collect();
    let mut drafted = draft(
        &base,
        &fixture.root,
        vec![
            scoped(
                &flow(&fixture.root),
                declare(
                    ids[0],
                    json!({ "flow.node": { "kind": "storage", "sectors": [format!("{}@head", fixture.spent)] } }),
                ),
            ),
            scoped(&fleet(&fixture.root), declare(ids[1], json!({ "cohort": { "robot_class": format!("{gone}@2") } }))),
            scoped(&extra, declare(ids[2], json!({ "cohort": { "robot_class": format!("{}@1", fixture.class) } }))),
            scoped(
                &extra,
                declare(ids[3], json!({ "cohort": { "robot_class": format!("{}@1", DeclaredId::mint()) } })),
            ),
        ],
    );
    drafted.minted.extend(ids.iter().copied());
    drafted.creates.push(NewScope { path: extra.clone(), boundary: false });
    drafted.bases.insert(extra.clone(), Seq::new(0));
    let problems = check(&fixture, &drafted, &base).await;
    let of = |entity: DeclaredId| -> Vec<&Problem> {
        problems.iter().filter(|problem| problem.entity == Some(entity.into())).collect()
    };
    let dangling = of(ids[0]);
    assert!(
        dangling
            .iter()
            .any(|problem| problem.kind == ProblemKind::Dangling && problem.path.as_deref() == Some("/sectors/0")),
        "{problems:#?}"
    );
    assert!(of(ids[1]).iter().any(|problem| problem.kind == ProblemKind::PinnedDangling), "{problems:#?}");
    assert!(of(ids[2]).is_empty(), "the owner of a reference from a created scope is found: {problems:#?}");
    assert!(of(ids[3]).iter().any(|problem| problem.kind == ProblemKind::Dangling), "{problems:#?}");
}

/// Section 3.7, step 5 over records: a live override the draft leaves standing still pins the catalog, so a new cohort
/// pinning it elsewhere is `PinnedTwice`; once the draft writes a generator input of the override's anchor, WPS
/// archives the override and only the new pin stands.
#[tokio::test]
async fn a_standing_override_pins_until_it_is_archived() {
    let fixture = fixture_of(MockWps::phase_3()).await;
    fixture.client.wps().publish("carrier_pool", "0.7.0", json!({ "type": "object" }), Vec::new());
    fixture.client.wps().anchor_generator("carrier_pool", "carrier_pool");
    let pool = DeclaredId::mint();
    write(&fixture.client, &fleet(&fixture.root), vec![declare(pool, json!({ "external_ids": { "src": "pool" } }))])
        .await;
    let target: EntityId = format!("{pool}/robot:0").parse().expect("a derived address");
    let pin_one = object(json!({ "cohort": { "robot_class": format!("{}@1", fixture.class) } }));
    write(&fixture.client, &fleet(&fixture.root), vec![Op::Override { target, components: pin_one }]).await;
    let base = read_site(&fixture.client, &fixture.root).await;
    let cohort = base.scopes[&fleet(&fixture.root)]
        .entities
        .iter()
        .find(|entity| entity.components.contains_key("cohort"))
        .expect("the fixture's cohort")
        .id;
    let fresh = DeclaredId::mint();
    let ops = |more: Vec<ScopedOp>| {
        let mut ops = vec![
            scoped(&fleet(&fixture.root), Op::Delete { id: cohort }),
            scoped(
                &fleet(&fixture.root),
                declare(fresh, json!({ "cohort": { "robot_class": format!("{}@2", fixture.class) } })),
            ),
        ];
        ops.extend(more);
        ops
    };
    let mut standing = draft(&base, &fixture.root, ops(Vec::new()));
    standing.minted.push(fresh);
    assert_eq!(
        found(&check(&fixture, &standing, &base).await, ProblemKind::PinnedTwice).scope,
        Some(fleet(&fixture.root))
    );

    let input = Op::Patch { id: pool, name: None, components: object(json!({ "carrier_pool": {} })) };
    let mut archiving = draft(&base, &fixture.root, ops(vec![scoped(&fleet(&fixture.root), input)]));
    archiving.minted.push(fresh);
    let problems = check(&fixture, &archiving, &base).await;
    assert!(problems.iter().all(|problem| problem.kind != ProblemKind::PinnedTwice), "{problems:#?}");
}

/// Section 3.7, step 6 over the graph the draft leaves: a referrer whose unrelated component the draft patches still
/// names the deleted sector, one whose referring component the draft rewrites does not, and every page of referrers is
/// read.
#[tokio::test]
async fn a_delete_is_checked_against_what_the_draft_leaves() {
    let fixture = fixture().await;
    fixture.client.wps().set_referrer_page(1);
    let second = DeclaredId::mint();
    write(
        &fixture.client,
        &flow(&fixture.root),
        vec![declare(
            second,
            json!({ "flow.node": { "kind": "storage", "sectors": [format!("{}@head", fixture.sector)] } }),
        )],
    )
    .await;
    let base = read_site(&fixture.client, &fixture.root).await;
    let node = base.scopes[&flow(&fixture.root)]
        .entities
        .iter()
        .find(|entity| entity.id != second)
        .expect("the fixture's node")
        .id;
    let delete = scoped(&map(&fixture.root), Op::Delete { id: fixture.sector });
    let unrelated = Op::Patch { id: node, name: None, components: object(json!({ "external_ids": { "src": "n" } })) };
    let drafted = draft(&base, &fixture.root, vec![delete.clone(), scoped(&flow(&fixture.root), unrelated)]);
    let problems = check(&fixture, &drafted, &base).await;
    let left: Vec<&Problem> = problems.iter().filter(|problem| problem.kind == ProblemKind::LeftDangling).collect();
    assert_eq!(left.len(), 2, "both referrers, across two pages, still name the sector: {problems:#?}");

    let rewrite = |id: DeclaredId| Op::Patch {
        id,
        name: None,
        components: object(json!({ "flow.node": { "kind": "storage", "sectors": [] } })),
    };
    let drafted = draft(
        &base,
        &fixture.root,
        vec![delete, scoped(&flow(&fixture.root), rewrite(node)), scoped(&flow(&fixture.root), rewrite(second))],
    );
    let problems = check(&fixture, &drafted, &base).await;
    assert!(problems.iter().all(|problem| problem.kind != ProblemKind::LeftDangling), "{problems:#?}");
}
