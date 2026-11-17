//! The mock's flagged export, held to what WPS's does under ruling D185: the requested scopes whole and a boundary
//! below them left out; `referenced` bringing each entity a reference reaches, a pin's at its changeset and one at
//! head's where its referrer's reading places it, followed until nothing new turns up, and none of the rest of its
//! scope; derived rows, and references to and from them, only under `derived`; records laid over the rows a
//! generation left; a stand-in generation's rows given back the pins their input held; and WPS's refusals of a scope
//! the export would read at two changesets and of an `at` that does not fit the scopes the request reaches. Each file
//! is read as the mock writes it, before the client closes it.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};
use studio_protocol::{DeclaredId, EntityId, ScopePath, Seq};

use crate::draft::Op;
use crate::error::{ProblemKind, WpsError};
use crate::mock::{MockWps, flagged_upload_refusal};
use crate::wire::{ExportFlags, RefAnnotation, WireChangeset};
use crate::wps::Wps;

fn path(text: &str) -> ScopePath {
    text.parse().expect("a scope path")
}

fn object(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("an object")
}

/// A mock whose registry reads references where WPS's demo seed annotates them: a frame's parent, a cohort's robot
/// class and a stamp's template, none in a template's embedded entities.
fn mock() -> MockWps {
    let mock = MockWps::phase_3();
    let refs = |path: &str| vec![RefAnnotation { path: path.to_owned(), expects: None }];
    let open = json!({ "type": "object" });
    mock.publish("frame", "0.7.0", open.clone(), refs("/parent"));
    mock.publish("cohort", "0.7.0", open.clone(), refs("/robot_class"));
    mock.publish("stamp", "0.7.0", open.clone(), refs("/template"));
    for unannotated in ["cell", "robot_class", "template"] {
        mock.publish(unannotated, "0.7.0", open.clone(), Vec::new());
    }
    mock
}

/// A site `org/site` with its `map` and `fleet` and an annex of its own unit below it, and two catalog items of their
/// own units, `lib/class` and `lib/template`.
async fn site_and_catalog(mock: &MockWps) {
    let made = [
        ("org", false),
        ("org/site", true),
        ("org/site/map", false),
        ("org/site/fleet", false),
        ("org/site/annex", true),
        ("lib", false),
        ("lib/class", true),
        ("lib/template", true),
    ];
    for (scope, boundary) in made {
        mock.create_scope(&path(scope), boundary).await.expect("a scope");
    }
}

/// Write `ops` into `scope` at its head.
async fn write(mock: &MockWps, scope: &str, ops: Vec<Op>) {
    let scope = path(scope);
    let head = mock.head(&scope).expect("the scope");
    let changeset = WireChangeset::of_one_scope(scope, head, "a test", "written by hand", ops);
    mock.post_changeset(&changeset).await.expect("the write lands");
}

fn declare(id: DeclaredId, components: Value) -> Op {
    Op::Declare { id, name: None, components: object(components) }
}

/// A frame whose parent is `parent`, as written.
fn frame_under(parent: &str) -> Value {
    json!({ "frame": { "parent": parent } })
}

/// A robot class in `lib/class`, declared at changeset 1 and patched up to changeset `head`.
async fn class(mock: &MockWps, head: u64) -> DeclaredId {
    let class = DeclaredId::mint();
    write(mock, "lib/class", vec![declare(class, json!({ "robot_class": {} }))]).await;
    for version in 2..=head {
        let patch = Op::Patch { id: class, name: None, components: object(json!({ "robot_class": { "v": version } })) };
        write(mock, "lib/class", vec![patch]).await;
    }
    class
}

/// A template whose one embedded entity is a cohort naming `reference`, where no annotation reaches.
fn template_naming(reference: &str) -> Value {
    let cohort = json!({ "local": "cohort", "components": { "cohort": { "robot_class": reference } } });
    json!({ "template": { "params": {}, "entities": [cohort] } })
}

/// The mock's flagged export of `org/site` with `flags` at its heads, as written.
async fn exported_with(mock: &MockWps, flags: ExportFlags) -> Result<Value, WpsError> {
    let bytes = mock.export_flagged(&path("org/site"), flags, &BTreeMap::new()).await?;
    Ok(serde_json::from_slice(&bytes).expect("JSON"))
}

/// The mock's flagged export of `org/site` with all three flags at its heads, as written.
async fn exported(mock: &MockWps) -> Result<Value, WpsError> {
    exported_with(mock, ExportFlags::ALL).await
}

/// The row the file lists as `id`.
fn listed(file: &Value, id: impl ToString) -> Option<&Value> {
    let id = id.to_string();
    file["entities"].as_array().expect("entities").iter().find(|entry| entry["id"] == id)
}

/// The scopes the file holds entities from, as its `scopes` lists them: each with its changeset and whether whole.
fn held(file: &Value) -> Vec<(String, u64, bool)> {
    let scopes = file["scopes"].as_array().expect("a list");
    let one = |entry: &Value| {
        let scope = entry["scope"].as_str().expect("a scope").to_owned();
        (scope, entry["changeset"].as_u64().expect("a changeset"), entry["whole"].as_bool().expect("whole or not"))
    };
    scopes.iter().map(one).collect()
}

/// The one problem an export refused with.
fn refused(result: Result<Value, WpsError>) -> (ProblemKind, Option<ScopePath>) {
    match result {
        Err(WpsError::ExportNotClosed { problems }) if problems.len() == 1 => {
            (problems[0].kind, problems[0].scope.clone())
        }
        other => panic!("one refusal, not {other:?}"),
    }
}

/// With `nested`, the named scope and every scope nested in it come whole, an entity no one references among them,
/// and a boundary below the site, with what it holds, does not; without it, only the named scope comes whole, and a
/// reference at head into the rest of its unit brings its target alone.
#[tokio::test]
async fn the_requested_scopes_come_whole_and_a_boundary_below_them_does_not() {
    let mock = mock();
    site_and_catalog(&mock).await;
    let (root, unreferenced, annexed) = (DeclaredId::mint(), DeclaredId::mint(), DeclaredId::mint());
    write(&mock, "org/site/map", vec![declare(root, json!({ "frame": {} }))]).await;
    write(&mock, "org/site/map", vec![declare(unreferenced, json!({ "frame": {} }))]).await;
    write(&mock, "org/site/annex", vec![declare(annexed, json!({ "frame": {} }))]).await;
    let site = DeclaredId::mint();
    write(&mock, "org/site", vec![declare(site, frame_under(&root.to_string()))]).await;

    let file = exported(&mock).await.expect("the export");
    let whole = |scope: &str, changeset: u64| (scope.to_owned(), changeset, true);
    assert_eq!(held(&file), [whole("org/site", 1), whole("org/site/fleet", 0), whole("org/site/map", 2)]);
    assert!(listed(&file, unreferenced).is_some(), "an entity no one references comes with its scope");
    assert!(listed(&file, annexed).is_none(), "a boundary below the site is no scope nested in it");
    let flags = (&file["nested"], &file["derived"], &file["referenced"], &file["scope"]);
    assert_eq!(flags, (&json!(true), &json!(true), &json!(true), &json!("org/site")));

    let unnested = exported_with(&mock, ExportFlags { nested: false, ..ExportFlags::ALL }).await.expect("the export");
    assert_eq!(held(&unnested), [whole("org/site", 1), ("org/site/map".to_owned(), 2, false)]);
    assert!(listed(&unnested, root).is_some() && listed(&unnested, unreferenced).is_none(), "the target alone");
    assert_eq!(unnested["nested"], json!(false));
}

/// Ruling D185: a pin brings its target at the pinned changeset and none of the rest of its scope, which the file
/// lists as held in part.
#[tokio::test]
async fn a_pin_brings_its_target_and_none_of_the_rest_of_its_scope() {
    let mock = mock();
    site_and_catalog(&mock).await;
    let (class, other) = (DeclaredId::mint(), DeclaredId::mint());
    let classes = vec![declare(class, json!({ "robot_class": {} })), declare(other, json!({ "robot_class": {} }))];
    write(&mock, "lib/class", classes).await;
    let cohort = json!({ "cohort": { "robot_class": format!("{class}@1") } });
    write(&mock, "org/site/fleet", vec![declare(DeclaredId::mint(), cohort)]).await;

    let file = exported(&mock).await.expect("the export");
    assert!(held(&file).contains(&("lib/class".to_owned(), 1, false)), "{:?}", held(&file));
    assert_eq!(listed(&file, class).expect("the pinned class")["scope"], json!("lib/class"));
    assert!(listed(&file, other).is_none(), "the rest of the class's scope stays out");
}

/// `referenced` follows what it brings, pins and references at head alike, until nothing new turns up; and a pinned
/// target's references at head read the scopes of its unit as the pinned changeset left them, by commit place, not
/// at their heads.
#[tokio::test]
async fn references_are_followed_from_what_they_bring_as_each_pin_left_its_unit() {
    let mock = mock();
    site_and_catalog(&mock).await;
    for (scope, boundary) in [("lib/kit", true), ("lib/kit/b", false)] {
        mock.create_scope(&path(scope), boundary).await.expect("a scope");
    }
    let (frame, part, kit) = (DeclaredId::mint(), DeclaredId::mint(), DeclaredId::mint());
    write(&mock, "lib/class", vec![declare(frame, json!({ "frame": {} }))]).await;
    write(&mock, "lib/kit/b", vec![declare(part, frame_under(&format!("{frame}@1")))]).await;
    write(&mock, "lib/kit", vec![declare(kit, frame_under(&part.to_string()))]).await;
    let moved = Op::Patch { id: part, name: None, components: object(json!({ "frame": { "parent": null } })) };
    write(&mock, "lib/kit/b", vec![moved]).await;
    write(&mock, "org/site/map", vec![declare(DeclaredId::mint(), frame_under(&format!("{kit}@1")))]).await;

    let file = exported(&mock).await.expect("the export");
    let scopes = held(&file);
    for scope in ["lib/class", "lib/kit", "lib/kit/b"] {
        assert!(scopes.contains(&(scope.to_owned(), 1, false)), "{scope} at changeset 1: {scopes:?}");
    }
    let brought = listed(&file, part).expect("the kit's part, at head where the pin read its unit");
    assert_eq!(
        brought.pointer("/components/frame/parent"),
        Some(&json!(format!("{frame}@1"))),
        "as changeset 1 left it"
    );
    assert!(listed(&file, frame).is_some(), "the part's own pin, followed in turn");
}

/// Derived rows, and the references made to and from them, come only under `derived`: without it the planted cells
/// stay out, a pin to one brings nothing, and a pin a planted row holds is never followed.
#[tokio::test]
async fn derived_rows_and_references_to_them_come_only_under_derived() {
    let mock = mock();
    site_and_catalog(&mock).await;
    let class = class(&mock, 1).await;
    let tiling = DeclaredId::mint();
    write(&mock, "lib/class", vec![declare(tiling, json!({ "frame": {} }))]).await;
    let derived: EntityId = format!("{tiling}/cell:0,0").parse().expect("a derived identity");
    mock.plant(&path("lib/class"), derived.clone(), object(json!({ "cell": {} })));
    let map = DeclaredId::mint();
    write(&mock, "org/site/map", vec![declare(map, frame_under(&format!("{derived}@2")))]).await;
    let planted: EntityId = format!("{map}/cell:0,0").parse().expect("a derived identity");
    let pinning = object(json!({ "cohort": { "robot_class": format!("{class}@2") } }));
    mock.plant(&path("org/site/map"), planted.clone(), pinning);

    let file = exported(&mock).await.expect("the export");
    assert!(listed(&file, &derived).is_some() && listed(&file, &planted).is_some(), "both derived rows");
    assert!(listed(&file, class).is_some(), "the planted row's pin, followed");

    let declared = exported_with(&mock, ExportFlags { derived: false, ..ExportFlags::ALL }).await.expect("the export");
    assert_eq!(declared["derived"], json!(false));
    assert!(listed(&declared, &derived).is_none() && listed(&declared, &planted).is_none(), "no derived row");
    assert!(listed(&declared, class).is_none(), "nor what only a derived row pins");
    assert!(held(&declared).iter().all(|(scope, _, _)| scope.starts_with("org/site")), "{:?}", held(&declared));
}

/// An override lies over the row it names that a generation left, and a suppressed one is gone; WPS's index holds an
/// override's references as it holds the row's, so under `derived` what only an override pins comes too.
#[tokio::test]
async fn records_are_laid_over_what_a_generation_left() {
    let mock = mock();
    site_and_catalog(&mock).await;
    let class = class(&mock, 2).await;
    let frame = DeclaredId::mint();
    write(&mock, "org/site/map", vec![declare(frame, json!({ "frame": { "unit_mm": 1 } }))]).await;
    let [kept, gone]: [EntityId; 2] = ["cell:0,0", "cell:0,1"].map(|key| format!("{frame}/{key}").parse().unwrap());
    for cell in [&kept, &gone] {
        mock.plant(&path("org/site/map"), cell.clone(), object(json!({ "cell": { "blocked": false } })));
    }
    let overlaid = json!({ "cell": { "blocked": true }, "cohort": { "robot_class": format!("{class}@1") } });
    let records = vec![
        Op::Override { target: kept.clone(), components: object(overlaid) },
        Op::Suppress { target: gone.clone() },
    ];
    write(&mock, "org/site/map", records).await;
    let file = exported(&mock).await.expect("the export");
    let overridden = listed(&file, &kept).expect("the overridden cell");
    assert_eq!(overridden.pointer("/components/cell/blocked"), Some(&json!(true)));
    assert_eq!(overridden["scope"], json!("org/site/map"));
    assert!(listed(&file, &gone).is_none(), "the suppressed cell is absent");
    assert!(held(&file).contains(&("lib/class".to_owned(), 1, false)), "the override's pin: {:?}", held(&file));
}

/// WPS's refusal of a scope its site reaches at two changesets: `reached_twice`, read as `PinnedTwice` naming the
/// scope.
#[tokio::test]
async fn a_scope_reached_at_two_changesets_is_refused() {
    let mock = mock();
    site_and_catalog(&mock).await;
    let class = class(&mock, 2).await;
    let pinned =
        |seq: u64| declare(DeclaredId::mint(), json!({ "cohort": { "robot_class": format!("{class}@{seq}") } }));
    write(&mock, "org/site/fleet", vec![pinned(1), pinned(2)]).await;
    assert_eq!(refused(exported(&mock).await), (ProblemKind::PinnedTwice, Some(path("lib/class"))));
}

/// WPS's refusals of an `at` that does not fit the scopes the export reads, the requested ones and with `referenced`
/// every scope of their units, each read as the cut's pin left unresolved, naming the scope: one leaving a scope of
/// the site's unit out, one naming a scope the export does not read, one past a scope's head, and without
/// `referenced`, one naming a scope of the unit the request does not ask for.
#[tokio::test]
async fn an_at_that_does_not_fit_the_scopes_read_is_refused_naming_the_scope() {
    let mock = mock();
    site_and_catalog(&mock).await;
    let at = |named: &[(&str, u64)]| -> BTreeMap<ScopePath, Seq> {
        named.iter().map(|(scope, seq)| (path(scope), Seq::new(*seq))).collect()
    };
    let unit = at(&[("org/site", 0), ("org/site/fleet", 0), ("org/site/map", 0)]);
    let left_out = at(&[("org/site", 0), ("org/site/fleet", 0)]);
    let unread = at(&[("lib/class", 0), ("org/site", 0), ("org/site/fleet", 0), ("org/site/map", 0)]);
    let beyond = at(&[("org/site", 0), ("org/site/fleet", 3), ("org/site/map", 0)]);
    let unreferenced = ExportFlags { nested: false, referenced: false, ..ExportFlags::ALL };
    let cases = [
        (ExportFlags::ALL, left_out, "org/site/map"),
        (ExportFlags::ALL, unread, "lib/class"),
        (ExportFlags::ALL, beyond, "org/site/fleet"),
        (unreferenced, unit, "org/site/fleet"),
    ];
    for (flags, at, named) in cases {
        match mock.export_flagged(&path("org/site"), flags, &at).await {
            Err(WpsError::PinUnresolved { scope, .. }) => assert_eq!(scope, path(named)),
            other => panic!("{flags:?} {at:?}: {other:?}"),
        }
    }
    let alone = at(&[("org/site", 0)]);
    mock.export_flagged(&path("org/site"), unreferenced, &alone).await.expect("the requested scope alone is read");
}

/// A pin never overrides where the request reads a scope: exported without `nested`, the site's map is no requested
/// scope but one the export reads at its head, so a pinned catalog entity's pin of the map at an earlier changeset is
/// refused as `reached_twice`, though the file would hold nothing else of the map.
#[tokio::test]
async fn a_pin_never_overrides_where_the_request_reads_a_scope() {
    let mock = mock();
    site_and_catalog(&mock).await;
    let (map_frame, kit) = (DeclaredId::mint(), DeclaredId::mint());
    write(&mock, "org/site/map", vec![declare(map_frame, json!({ "frame": {} }))]).await;
    write(&mock, "lib/class", vec![declare(kit, frame_under(&format!("{map_frame}@1")))]).await;
    write(&mock, "org/site/map", vec![declare(DeclaredId::mint(), json!({ "frame": {} }))]).await;
    write(&mock, "org/site", vec![declare(DeclaredId::mint(), frame_under(&format!("{kit}@1")))]).await;
    let unnested = ExportFlags { nested: false, ..ExportFlags::ALL };
    assert_eq!(refused(exported_with(&mock, unnested).await), (ProblemKind::PinnedTwice, Some(path("org/site/map"))));
}

/// A generation that stamps each template a `stamp` names once: one row, `<stamp>/instance:0`, carrying the
/// components of the template's first embedded entity as its input holds them, pins stripped.
fn stamp_once(read: Vec<Value>) -> Result<Vec<Value>, String> {
    let mut rows = Vec::new();
    for entry in &read {
        let Some(named) = entry.pointer("/components/stamp/template").and_then(Value::as_str) else { continue };
        let template = read.iter().find(|one| one["id"] == named).ok_or(format!("{named} is not in the input"))?;
        let components = template.pointer("/components/template/entities/0/components").cloned().unwrap_or_default();
        let id = format!("{}/instance:0", entry["id"].as_str().unwrap_or_default());
        rows.push(json!({ "id": id, "components": components }));
    }
    Ok(rows)
}

/// A stand-in generation reads the template a stamp pins, which `referenced` brought; its row, copying the template's
/// nested pin, holds that pin where the index reads it, as WPS's rows do, so the class it pins comes at that pin; and
/// a row naming a scope its input pinned at two changesets is refused, as WPS refuses the write that would leave it.
#[tokio::test]
async fn a_generated_row_brings_the_pin_it_copies() {
    let mock = mock();
    mock.set_generation(Box::new(stamp_once));
    site_and_catalog(&mock).await;
    let class = class(&mock, 2).await;
    let template = DeclaredId::mint();
    write(&mock, "lib/template", vec![declare(template, template_naming(&format!("{class}@1")))]).await;
    let stamp = DeclaredId::mint();
    write(&mock, "org/site/map", vec![declare(stamp, json!({ "stamp": { "template": format!("{template}@1") } }))])
        .await;

    let file = exported(&mock).await.expect("the copied pin is followed");
    assert!(held(&file).contains(&("lib/class".to_owned(), 1, false)), "{:?}", held(&file));
    let row = listed(&file, format!("{stamp}/instance:0")).expect("the generated row");
    assert_eq!(row["scope"], json!("org/site/map"));
    assert_eq!(row.pointer("/components/cohort/robot_class"), Some(&json!(format!("{class}@1"))), "pinned again");

    let cohort = json!({ "cohort": { "robot_class": format!("{class}@2") } });
    write(&mock, "org/site/fleet", vec![declare(DeclaredId::mint(), cohort)]).await;
    assert_eq!(refused(exported(&mock).await), (ProblemKind::PinnedTwice, Some(path("lib/class"))));
}

/// A stand-in generation's row that no file could hold is refused naming the row, as a failed generation is, rather
/// than filled in: one without an identity, one whose generator no entity it read declares, one without components.
#[tokio::test]
async fn a_generated_row_that_names_no_entity_is_refused() {
    let unheld = DeclaredId::mint();
    let rows = [
        (json!({ "components": {} }), "without an `id`".to_owned()),
        (json!({ "id": format!("{unheld}/instance:0"), "components": {} }), unheld.to_string()),
        (json!({ "id": "STAMP/instance:0" }), "without its components".to_owned()),
    ];
    for (row, named) in rows {
        let mock = mock();
        site_and_catalog(&mock).await;
        let stamp = DeclaredId::mint();
        write(&mock, "org/site/map", vec![declare(stamp, json!({ "frame": {} }))]).await;
        let row = row.to_string().replace("STAMP", &stamp.to_string());
        let made: Value = serde_json::from_str(&row).expect("a row");
        mock.set_generation(Box::new(move |_| Ok(vec![made.clone()])));
        match exported(&mock).await {
            Err(WpsError::Refused { problem }) => {
                assert_eq!(problem.kind, ProblemKind::GenerationRefused, "{row}");
                assert!(problem.text.contains(&named), "{named} in {}", problem.text);
            }
            other => panic!("{row}: {other:?}"),
        }
    }
}

/// WPS's import refuses a flagged export before it reads the scope or the base, naming the keys it holds that only
/// such a file writes: a scope it holds in part would read as deletions. So an upload to a scope at a base it never
/// reached, or to no scope, is refused as one too.
#[tokio::test]
async fn an_import_of_a_flagged_export_is_refused() {
    let mock = mock();
    site_and_catalog(&mock).await;
    let file = exported(&mock).await.expect("the export");
    let refusal = flagged_upload_refusal(&file).expect("the upload is refused");
    assert_eq!(refusal.details["keys"], json!(["derived", "nested", "referenced", "scope", "scopes"]));
    for (scope, base) in [("org/site", 0), ("org/site", 9), ("org/nowhere", 0)] {
        match mock.import(&path(scope), Seq::new(base), &file).await {
            Err(WpsError::Refused { problem }) => {
                assert_eq!(problem.kind, ProblemKind::DocumentFlaggedExport, "{scope} at {base}");
            }
            other => panic!("a flagged export imported into {scope} at {base}: {other:?}"),
        }
    }
}
