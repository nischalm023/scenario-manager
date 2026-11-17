//! Spec 04 section 8's statement 17, the client's closure check of the flagged export (rulings D40 and D185), over the
//! mock's export: the export holds the site's unit whole and every entity its references reach, generated entities
//! beside declared ones, pins stripped and keys sorted; the client refuses a reference to an entity the export does
//! not hold, a pin at another changeset than the one the export holds its scope at or naming no changeset, a
//! reference at head across units, and a component the registry does not define; and a scope it holds below its head
//! makes the export inexact. The mock's own emulation of WPS is tested beside it (`mock::flagged_export`), and `close`
//! on files written by hand beside `close`.

mod support;

use serde_json::{Map, Value, json};
use studio_protocol::{BranchName, DeclaredId, EntityId, Pin, ScopePath, Seq};
use support::{catalog, client, declare, object, path, site, write};
use wps_client::wire::ExportFlags;
use wps_client::wps::Wps;
use wps_client::{Client, CutExport, Exactness, MockWps, Op, Problem, ProblemKind, WorldStore, WpsError};

async fn exported(client: &Client<MockWps>, root: &ScopePath) -> Result<CutExport, WpsError> {
    let cut = client.resolve_cut(root, &Pin::Branch(BranchName::main())).await.expect("a cut");
    client.export(&cut).await
}

fn problems(result: Result<CutExport, WpsError>) -> Vec<Problem> {
    match result {
        Err(WpsError::ExportNotClosed { problems }) => problems,
        other => panic!("expected ExportNotClosed, got {other:?}"),
    }
}

fn file(export: &CutExport) -> Value {
    serde_json::from_slice(&export.bytes).expect("JSON")
}

/// The entity the file lists as `id`.
fn listed<'a>(file: &'a Value, id: &str) -> Option<&'a Value> {
    file["entities"].as_array().expect("entities").iter().find(|entry| entry["id"] == id)
}

/// A site whose map holds one frame, at `name`.
async fn framed_site(client: &Client<MockWps>, name: &str) -> (ScopePath, DeclaredId) {
    let root = site(client, name).await;
    let frame = DeclaredId::mint();
    write(client, &path(&format!("{root}/map")), vec![declare(frame, json!({ "frame": { "unit_mm": 1 } }))]).await;
    (root, frame)
}

/// Statement 17: the export holds the site's scopes at the cut and the robot class a cohort pins at its pin, lists
/// generated entities beside declared ones under the scope that holds each, strips the pin, states its scope and its
/// three flags, and writes no prologue; and it says which scope holds each declared entity, as the Map panel reads it.
#[tokio::test]
async fn the_export_holds_the_site_and_what_it_pins() {
    let client = client().await;
    let (root, frame) = framed_site(&client, "held").await;
    let item = catalog(&client, "held").await;
    let class = DeclaredId::mint();
    write(&client, &item, vec![declare(class, json!({ "robot_class": {} }))]).await;
    let fleet = path(&format!("{root}/fleet"));
    let cohort = DeclaredId::mint();
    write(&client, &fleet, vec![declare(cohort, json!({ "cohort": { "robot_class": format!("{class}@1") } }))]).await;
    let map = path(&format!("{root}/map"));
    let cell: EntityId = format!("{frame}/cell:0,0").parse().expect("a generated identity");
    client.wps().plant(&map, cell.clone(), object(json!({ "cell": {}, "pose": { "frame": frame.to_string() } })));

    let cut = client.resolve_cut(&root, &Pin::Branch(BranchName::main())).await.expect("a cut");
    let export = client.export(&cut).await.expect("a closed export");
    let mut held = cut.seqs.clone();
    held.insert(item.clone(), Seq::new(1));
    assert_eq!(export.scopes, held, "the site at its cut and the pinned item at its pin");
    assert_eq!(export.exact, Exactness::Exact);
    let holders = [(frame, map.clone()), (cohort, fleet.clone()), (class, item.clone())].into_iter().collect();
    assert_eq!(export.holders, holders, "each declared entity's scope, the generated cell none of its own");

    let written = file(&export);
    assert_eq!(written["scope"], json!(root.to_string()));
    for flag in ["nested", "derived", "referenced"] {
        assert_eq!(written[flag], json!(true), "`{flag}` stated");
    }
    for absent in ["doc", "proto_version", "primary_frame"] {
        assert!(written.get(absent).is_none(), "no `{absent}`");
    }
    let generated = listed(&written, &cell.to_string()).expect("the generated cell is listed");
    assert_eq!(generated["scope"], json!(map.to_string()));
    let cohort = listed(&written, &cohort.to_string()).expect("the cohort is listed");
    assert_eq!(cohort.pointer("/components/cohort/robot_class"), Some(&json!(class.to_string())), "pin stripped");
    assert_eq!(listed(&written, &class.to_string()).expect("the pinned class")["scope"], json!(item.to_string()));
    let keys: Vec<&String> = generated.as_object().expect("an object").keys().collect();
    assert_eq!(keys, ["components", "id", "scope"], "keys sorted, no `name` for an entity without one");
}

/// Statement 17: a reference to an entity the export does not hold is named with its referrer, field and target,
/// whether it names a declared identity or a generated one; a reference to a generated entity the export holds
/// passes.
#[tokio::test]
async fn a_reference_out_of_the_export_is_refused() {
    let client = client().await;
    let (root, frame) = framed_site(&client, "out").await;
    let map = path(&format!("{root}/map"));
    let planted = format!("{frame}/cell:0,0");
    client.wps().plant(&map, planted.parse().expect("an identity"), Map::new());
    let (orphan, underived, held) = (DeclaredId::mint(), DeclaredId::mint(), DeclaredId::mint());
    let nowhere = DeclaredId::mint();
    write(
        &client,
        &map,
        vec![
            declare(orphan, json!({ "frame": { "parent": nowhere.to_string() } })),
            declare(underived, json!({ "frame": { "parent": format!("{frame}/cell:9,9") } })),
            declare(held, json!({ "frame": { "parent": planted } })),
        ],
    )
    .await;
    let found = problems(exported(&client, &root).await);
    assert_eq!(found.len(), 2, "the held generated entity resolves: {found:#?}");
    for (referrer, target) in [(orphan, nowhere.to_string()), (underived, format!("{frame}/cell:9,9"))] {
        let problem = found.iter().find(|problem| problem.entity == Some(EntityId::from(referrer))).expect("named");
        assert_eq!(problem.kind, ProblemKind::Dangling);
        assert_eq!((problem.component.as_deref(), problem.path.as_deref()), (Some("frame"), Some("/parent")));
        assert!(problem.text.contains(&target), "the target is named: {}", problem.text);
    }
}

/// Spec 08 section 3.5's exactness, the export's own: a site read at its heads, an exact cut, pins a catalog item at a
/// changeset below the item's head, so the export holds the item below its head and is inexact, naming the item.
#[tokio::test]
async fn a_held_scope_read_below_its_head_makes_the_export_inexact() {
    let client = client().await;
    let (root, _) = framed_site(&client, "below").await;
    let (item, class) = class_item(&client, "below", 2).await;
    let cohort = json!({ "cohort": { "robot_class": format!("{class}@1") } });
    write(&client, &path(&format!("{root}/fleet")), vec![declare(DeclaredId::mint(), cohort)]).await;
    let cut = client.resolve_cut(&root, &Pin::Branch(BranchName::main())).await.expect("a cut");
    assert_eq!(cut.exact, Exactness::Exact, "the site's own scopes stand at their heads");
    let export = client.export(&cut).await.expect("the export");
    assert_eq!(export.scopes.get(&item), Some(&Seq::new(1)), "the item at the pin, below its head");
    let Exactness::ComponentsOnly { reason } = &export.exact else { panic!("the export is not exact") };
    assert!(reason.contains(item.as_str()), "{reason}");
}

/// A catalog item at `name` declaring one robot class at changeset 1, patched up to changeset `head`.
async fn class_item(client: &Client<MockWps>, name: &str, head: u64) -> (ScopePath, DeclaredId) {
    let item = catalog(client, name).await;
    let class = DeclaredId::mint();
    write(client, &item, vec![declare(class, json!({ "robot_class": {} }))]).await;
    for version in 2..=head {
        let patch = Op::Patch { id: class, name: None, components: object(json!({ "robot_class": { "v": version } })) };
        write(client, &item, vec![patch]).await;
    }
    (item, class)
}

/// A template whose one embedded entity is a cohort naming `reference`: a place the annotations do not reach, so WPS
/// neither indexes the reference nor follows its pin until complete annotation traversal lands.
fn template_naming(reference: &str) -> Value {
    let cohort = json!({ "local": "cohort", "components": { "cohort": { "robot_class": reference } } });
    json!({ "template": { "params": {}, "entities": [cohort] } })
}

/// The scopes WPS's own file holds entities from for `root` at its heads, before the client reads it.
async fn held_by_wps(client: &Client<MockWps>, root: &ScopePath) -> Value {
    let cut = client.resolve_cut(root, &Pin::Branch(BranchName::main())).await.expect("a cut");
    let raw = client.wps().export_flagged(root, ExportFlags::ALL, &cut.seqs).await.expect("WPS writes the file");
    serde_json::from_slice::<Value>(&raw).expect("JSON")["scopes"].clone()
}

/// The mock follows only the references WPS's index holds: a catalog item reached only through a pin nested in a
/// template's embedded entities is not held, so the client finds the reference out of the file and refuses it.
#[tokio::test]
async fn a_scope_pinned_only_inside_a_template_is_not_held_and_the_export_is_refused() {
    let client = client().await;
    let (root, _) = framed_site(&client, "nested-only").await;
    let (pinned, class) = class_item(&client, "nested-only-class", 1).await;
    let holder = catalog(&client, "nested-only-template").await;
    let template = DeclaredId::mint();
    write(&client, &holder, vec![declare(template, template_naming(&format!("{class}@1")))]).await;
    let stamp = json!({ "stamp": { "template": format!("{template}@1") } });
    write(&client, &path(&format!("{root}/map")), vec![declare(DeclaredId::mint(), stamp)]).await;

    let held = held_by_wps(&client, &root).await;
    let template_held = json!({ "changeset": 1, "scope": holder, "whole": false });
    assert!(held.as_array().expect("a list").contains(&template_held), "{held}");
    assert!(!held.to_string().contains(pinned.as_str()), "the nested pin is not followed: {held}");
    let found = problems(exported(&client, &root).await);
    let problem = found.iter().find(|problem| problem.entity == Some(template.into())).expect("the template");
    assert_eq!(problem.kind, ProblemKind::Dangling, "{found:#?}");
    let field = (problem.component.as_deref(), problem.path.as_deref());
    assert_eq!(field, (Some("template"), Some("/entities/0/components/cohort/robot_class")));
    assert!(problem.text.contains(&class.to_string()), "{}", problem.text);
}

/// Ruling D185: a pin brings its target and none of the rest of its scope. A pin nested in a template, which WPS's
/// index cannot see, naming another entity of that scope at the same changeset, names an entity the file does not
/// hold, though the file holds its scope in part at that changeset; so the client refuses it.
#[tokio::test]
async fn a_nested_pin_beside_a_pinned_target_names_nothing_the_file_holds() {
    let client = client().await;
    let (root, _) = framed_site(&client, "beside").await;
    let item = catalog(&client, "beside").await;
    let (class, beside) = (DeclaredId::mint(), DeclaredId::mint());
    let classes = vec![declare(class, json!({ "robot_class": {} })), declare(beside, json!({ "robot_class": {} }))];
    write(&client, &item, classes).await;
    let cohort = json!({ "cohort": { "robot_class": format!("{class}@1") } });
    write(&client, &path(&format!("{root}/fleet")), vec![declare(DeclaredId::mint(), cohort)]).await;
    let template = DeclaredId::mint();
    write(&client, &path(&format!("{root}/map")), vec![declare(template, template_naming(&format!("{beside}@1")))])
        .await;

    let held = held_by_wps(&client, &root).await;
    assert!(held.as_array().expect("a list").contains(&json!({ "changeset": 1, "scope": item, "whole": false })));
    let found = problems(exported(&client, &root).await);
    let [problem] = found.as_slice() else { panic!("one problem: {found:#?}") };
    assert_eq!((problem.kind, problem.entity.clone()), (ProblemKind::Dangling, Some(template.into())));
    assert!(problem.text.contains(&beside.to_string()), "{}", problem.text);
}

/// A pin WPS's index cannot see, nested in a template, names an earlier changeset of a scope the file holds at a
/// later one. Stripping it would read the later version, so the client refuses it naming both changesets; a nested
/// pin at the changeset held is stripped like any other.
#[tokio::test]
async fn a_nested_pin_is_held_to_the_changeset_the_export_holds() {
    let client = client().await;
    let (root, _) = framed_site(&client, "nested-pin").await;
    let (pinned, class) = class_item(&client, "nested-pin", 3).await;
    let cohort = json!({ "cohort": { "robot_class": format!("{class}@3") } });
    write(&client, &path(&format!("{root}/fleet")), vec![declare(DeclaredId::mint(), cohort)]).await;
    let map = path(&format!("{root}/map"));
    let (agreeing, earlier) = (DeclaredId::mint(), DeclaredId::mint());
    write(&client, &map, vec![declare(agreeing, template_naming(&format!("{class}@3")))]).await;
    let written = file(&exported(&client, &root).await.expect("a pin at the changeset held closes"));
    let nested = "/components/template/entities/0/components/cohort/robot_class";
    let agreed = listed(&written, &agreeing.to_string()).expect("the template");
    assert_eq!(agreed.pointer(nested), Some(&json!(class.to_string())), "its pin stripped");

    write(&client, &map, vec![declare(earlier, template_naming(&format!("{class}@2")))]).await;
    let held = held_by_wps(&client, &root).await;
    let class_held = json!({ "changeset": 3, "scope": pinned, "whole": false });
    assert!(held.as_array().expect("a list").contains(&class_held), "{held}");
    let found = problems(exported(&client, &root).await);
    assert_eq!(found.len(), 1, "{found:#?}");
    let problem = &found[0];
    assert_eq!((problem.kind, problem.entity.clone()), (ProblemKind::PinnedTwice, Some(earlier.into())));
    assert_eq!(problem.path.as_deref(), Some("/entities/0/components/cohort/robot_class"));
    for named in [pinned.as_str(), "changeset 2", "changeset 3"] {
        assert!(problem.text.contains(named), "{named} in {}", problem.text);
    }
}

/// The pin rule allows a reference at head, bare or `@head`, only within the referrer's unit, and WPS refuses one
/// across units only where the annotations reach. One nested in a template, naming an entity the export holds through
/// another pin, would read that pin's changeset in silence, so the client refuses it, naming the scope; one at head
/// within the unit passes.
#[tokio::test]
async fn a_reference_at_head_across_units_is_refused() {
    let client = client().await;
    let (root, frame) = framed_site(&client, "head-across").await;
    let (item, class) = class_item(&client, "head-across", 1).await;
    let cohort = json!({ "cohort": { "robot_class": format!("{class}@1") } });
    write(&client, &path(&format!("{root}/fleet")), vec![declare(DeclaredId::mint(), cohort)]).await;
    let (bare, at_head, within) = (DeclaredId::mint(), DeclaredId::mint(), DeclaredId::mint());
    let ops = vec![
        declare(bare, template_naming(&class.to_string())),
        declare(at_head, template_naming(&format!("{class}@head"))),
        declare(within, template_naming(&frame.to_string())),
    ];
    write(&client, &path(&format!("{root}/map")), ops).await;
    let found = problems(exported(&client, &root).await);
    assert_eq!(found.len(), 2, "the reference within the unit passes: {found:#?}");
    for template in [bare, at_head] {
        let problem = found.iter().find(|problem| problem.entity == Some(template.into())).expect("each is named");
        assert_eq!(problem.kind, ProblemKind::HeadAcrossUnits, "{}", problem.text);
        assert!(problem.text.contains(item.as_str()), "{}", problem.text);
    }
}

/// A template whose one embedded entity is a frame whose parent is a derivation address of `generator`, written in
/// its two parts, `{ generator, key }`: a place the annotations do not reach.
fn template_parented_in_two_parts(generator: DeclaredId, key: &str) -> Value {
    let parent = json!({ "generator": generator.to_string(), "key": key });
    let frame = json!({ "local": "frame", "components": { "frame": { "parent": parent } } });
    json!({ "template": { "params": {}, "entities": [frame] } })
}

/// A derivation address written in its two parts carries no pin, so it is at head, and WPS refuses one across units
/// (`two_part_across_units`) only where the annotations reach. One nested in a template, naming another unit's derived
/// entity that the export holds through a pin, would read that pin's changeset in silence, so the client refuses it,
/// naming the scope; one naming a derived entity of the referrer's unit passes.
#[tokio::test]
async fn a_derivation_address_in_two_parts_across_units_is_refused() {
    let client = client().await;
    let (root, frame) = framed_site(&client, "two-part-across").await;
    let map = path(&format!("{root}/map"));
    let item = catalog(&client, "two-part-across").await;
    let tiling = DeclaredId::mint();
    write(&client, &item, vec![declare(tiling, json!({ "frame": {} }))]).await;
    let (across, within) = (format!("{tiling}/cell:0,0"), format!("{frame}/cell:0,0"));
    client.wps().plant(&item, across.parse().expect("an identity"), object(json!({ "cell": {} })));
    client.wps().plant(&map, within.parse().expect("an identity"), object(json!({ "cell": {} })));
    let (crossing, staying) = (DeclaredId::mint(), DeclaredId::mint());
    let ops = vec![
        declare(DeclaredId::mint(), json!({ "frame": { "parent": format!("{across}@1") } })),
        declare(crossing, template_parented_in_two_parts(tiling, "cell:0,0")),
        declare(staying, template_parented_in_two_parts(frame, "cell:0,0")),
    ];
    write(&client, &map, ops).await;
    let found = problems(exported(&client, &root).await);
    let [problem] = found.as_slice() else { panic!("the address within the unit passes: {found:#?}") };
    assert_eq!((problem.kind, problem.entity.clone()), (ProblemKind::HeadAcrossUnits, Some(crossing.into())));
    assert!(problem.text.contains(item.as_str()), "{}", problem.text);
}

/// A pin naming a tag or a branch names no changeset to hold the file to, and one that does not read names nothing:
/// each is refused where it stands rather than stripped to the bare identity the file holds.
#[tokio::test]
async fn a_pin_naming_no_changeset_is_refused_not_stripped() {
    let client = client().await;
    let (root, _) = framed_site(&client, "no-changeset").await;
    let (_, class) = class_item(&client, "no-changeset", 1).await;
    let cohort = json!({ "cohort": { "robot_class": format!("{class}@1") } });
    write(&client, &path(&format!("{root}/fleet")), vec![declare(DeclaredId::mint(), cohort)]).await;
    let cases = [
        (format!("{class}@tag:v1"), ProblemKind::TagNotHeld),
        (format!("{class}@main"), ProblemKind::BadReference),
        (format!("{class}@1@2"), ProblemKind::BadReference),
    ];
    let mut templates = Vec::new();
    for (reference, kind) in &cases {
        let template = DeclaredId::mint();
        write(&client, &path(&format!("{root}/map")), vec![declare(template, template_naming(reference))]).await;
        templates.push((template, reference, *kind));
    }
    let found = problems(exported(&client, &root).await);
    assert_eq!(found.len(), cases.len(), "{found:#?}");
    for (template, reference, kind) in templates {
        let problem = found.iter().find(|problem| problem.entity == Some(template.into())).expect("each is named");
        assert_eq!(problem.kind, kind, "{reference}: {}", problem.text);
        assert!(problem.text.contains(reference.as_str()), "{reference}: {}", problem.text);
    }
}

/// A `$` key names no component: a note beside an entity's components or a template internal's, and a placeholder
/// standing for a templated tiling's derived map, as in Walmart DFW6, leave the export closed rather than refused as
/// components the registry does not define.
#[tokio::test]
async fn a_dollar_key_is_no_component_the_registry_must_define() {
    let client = client().await;
    client.wps().publish("tiling", "0.7.0", json!({ "type": "object" }), Vec::new());
    let (root, _) = framed_site(&client, "dollar-keys").await;
    let tiling = json!({ "pitch": [850, 850], "derives": { "$param": "derives" } });
    let internal = json!({ "local": "floor", "components": { "$comment": "open floor", "tiling": tiling } });
    let template = json!({ "$comment": "a templated floor", "template": { "params": {}, "entities": [internal] } });
    write(&client, &path(&format!("{root}/map")), vec![declare(DeclaredId::mint(), template)]).await;
    exported(&client, &root).await.expect("no `$` key is read as a component");
}

/// A parameter standing for a reference inside a template is no reference (SPEC §2), its stamp's to replace, so the
/// export stays closed; the same parameter at the top of a component names nothing, and is refused.
#[tokio::test]
async fn a_parameter_for_a_reference_is_none_inside_a_template_only() {
    let client = client().await;
    let (root, _) = framed_site(&client, "parameter").await;
    let parameter = json!({ "$param": "class", "$comment": "the stamp's robot class" });
    let internal = json!({ "local": "fleet", "components": { "cohort": { "robot_class": parameter } } });
    let template = json!({ "template": { "params": { "class": {} }, "entities": [internal] } });
    write(&client, &path(&format!("{root}/map")), vec![declare(DeclaredId::mint(), template)]).await;
    exported(&client, &root).await.expect("a parameter inside a template is no reference");

    let cohort = DeclaredId::mint();
    write(
        &client,
        &path(&format!("{root}/fleet")),
        vec![declare(cohort, json!({ "cohort": { "robot_class": parameter } }))],
    )
    .await;
    let found = problems(exported(&client, &root).await);
    assert!(
        found.iter().any(|problem| problem.kind == ProblemKind::BadReference
            && problem.entity == Some(cohort.into())
            && problem.path.as_deref() == Some("/robot_class")),
        "{found:#?}"
    );
}

/// Section 3.3 with section 3.10: a component the export carries and the registry does not define has no annotations
/// to walk, so the export fails closed rather than vouch for references nobody read.
#[tokio::test]
async fn an_undefined_component_fails_the_closure_check_closed() {
    let client = client().await;
    let (root, _) = framed_site(&client, "undefined").await;
    write(
        &client,
        &path(&format!("{root}/map")),
        vec![declare(DeclaredId::mint(), json!({ "nobody.defines": { "target": DeclaredId::mint().to_string() } }))],
    )
    .await;
    let found = problems(exported(&client, &root).await);
    assert!(
        found
            .iter()
            .any(|problem| problem.kind == ProblemKind::Unregistered && problem.text.contains("nobody.defines")),
        "{found:#?}"
    );
}
