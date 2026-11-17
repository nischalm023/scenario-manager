//! Spec 04 section 3.13's development seed, `seed::run`, against the mock: an entry below an earlier entry's scope is
//! written, the populated ancestor kept with only its flag checked; an entry to write is refused, before anything is
//! written, beside a held scope it can reference (kin to its own, or of a document its `depends_on` names) that holds
//! no changeset seeded from the manifest's text of its document; and a held scope no entry to write can reference, as
//! every scope of a manifest with nothing to write, is skipped without being asked about.

mod support;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use studio_protocol::{DeclaredId, ScopePath, Seq};
use support::{client, path};
use wps_client::seed::{self, SeedError, SeedReport};
use wps_client::{Client, DocId, MockWps};

/// A fresh folder for one test's seed manifest and documents, and the scopes `names` gives below `template/site/`,
/// each name's first slug a template root made fresh by a tag only this call uses: `["t/map", "u/map"]` are the maps
/// of two templates no other test writes.
fn seed_folder_and_scopes<const N: usize>(names: [&str; N]) -> (PathBuf, [ScopePath; N]) {
    let tag = &DeclaredId::mint().to_string()[..8];
    let folder = env::temp_dir().join(format!("studio-seed-unit-{tag}"));
    fs::create_dir_all(&folder).expect("a folder");
    let fresh = |name: &str| match name.split_once('/') {
        Some((root, below)) => path(&format!("template/site/{root}{tag}/{below}")),
        None => path(&format!("template/site/{name}{tag}")),
    };
    (folder, names.map(fresh))
}

/// A world document of `entities` under a fresh identity, depending on the documents `depends_on` names.
fn world(entities: Vec<Value>, depends_on: &[&str]) -> String {
    let doc = json!({ "id": DocId::mint().as_str(), "kind": "world", "depends_on": depends_on });
    json!({ "proto_version": "0.7.0", "doc": doc, "entities": entities }).to_string()
}

/// A frame, a fresh entity.
fn frame() -> Value {
    json!({ "id": DeclaredId::mint().to_string(), "components": { "frame": {} } })
}

/// A world document of `count` frames, so that no two calls give the same text.
fn frames(count: usize) -> String {
    world((0..count).map(|_| frame()).collect(), &[])
}

/// The identity of the document `text` holds.
fn identity(text: &str) -> String {
    let document: Value = serde_json::from_str(text).expect("a document");
    document["doc"]["id"].as_str().expect("an identity").to_owned()
}

/// Seeds from a manifest in `folder` listing `entries`, each a scope and its document's file, in order.
async fn seed_from(
    client: &Client<MockWps>,
    folder: &Path,
    entries: &[(&ScopePath, &str)],
) -> Result<SeedReport, SeedError> {
    let entries: Vec<Value> =
        entries.iter().map(|(scope, document)| json!({ "scope": scope.to_string(), "document": document })).collect();
    let manifest = folder.join("manifest.json");
    fs::write(&manifest, json!({ "v": 1, "vocabulary": [], "entries": entries }).to_string()).expect("written");
    seed::run(client, &manifest, None, "test").await
}

/// Section 3.13: a manifest whose second entry is the first entry's `map` seeds both, the populated root kept with only
/// its flag checked.
#[tokio::test]
async fn a_seed_entry_below_an_earlier_one_is_written() {
    let client = client().await;
    let (folder, [root, map]) = seed_folder_and_scopes(["t", "t/map"]);
    let description = json!({ "template.description": { "title": "T", "text": "t" } });
    let card = json!({ "id": DeclaredId::mint().to_string(), "components": description });
    fs::write(folder.join("root.json"), world(vec![card], &[])).expect("written");
    fs::write(folder.join("map.json"), frames(1)).expect("written");
    let report = seed_from(&client, &folder, &[(&root, "root.json"), (&map, "map.json")]).await.expect("seeded");
    let _ = fs::remove_dir_all(&folder);
    let written: Vec<ScopePath> = report.written.into_iter().map(|(scope, _)| scope).collect();
    assert_eq!(written, vec![root, map]);
}

/// Section 3.13: a manifest that gains a `flow` entry beside a `map` seeded from an earlier text of the map's document
/// writes nothing and makes no scope, naming the map; beside the map's own text it writes the flow.
#[tokio::test]
async fn a_seed_writes_nothing_beside_a_scope_seeded_from_another_text_of_its_document() {
    let client = client().await;
    let (folder, [map, flow]) = seed_folder_and_scopes(["t/map", "t/flow"]);
    let (earlier, current) = (frames(1), frames(2));
    fs::write(folder.join("flow.json"), frames(1)).expect("written");
    fs::write(folder.join("map.json"), &earlier).expect("written");
    seed_from(&client, &folder, &[(&map, "map.json")]).await.expect("the earlier map seeded");
    fs::write(folder.join("map.json"), &current).expect("written");
    let both = [(&map, "map.json"), (&flow, "flow.json")];
    let refused = seed_from(&client, &folder, &both).await;
    let Err(SeedError::UnmatchedHeldScope { held, unwritten, .. }) = &refused else { panic!("{refused:?}") };
    assert_eq!((held, unwritten), (&map, &flow));
    let tree = client.tree().await.expect("the scope tree");
    assert_eq!((tree.head(&map), tree.head(&flow)), (Some(Seq::new(1)), None), "nothing written, no scope made");

    fs::write(folder.join("map.json"), &earlier).expect("written");
    let report = seed_from(&client, &folder, &both).await.expect("seeded beside its map");
    let _ = fs::remove_dir_all(&folder);
    assert_eq!((report.written, report.skipped), (vec![(flow, Seq::new(1))], vec![map]));
}

/// Section 3.13: the seed checks, before it writes any entry, every held scope an entry to write can reference, so
/// where a manifest's first entry is still to write and its later entry's scope was seeded from another text of its
/// document, it makes not even the first entry's scope.
#[tokio::test]
async fn a_seed_checks_a_later_held_entry_before_it_writes_an_earlier_one() {
    let client = client().await;
    let (folder, [fleet, map]) = seed_folder_and_scopes(["t/fleet", "t/map"]);
    fs::write(folder.join("map.json"), frames(1)).expect("written");
    seed_from(&client, &folder, &[(&map, "map.json")]).await.expect("the earlier map seeded");
    fs::write(folder.join("map.json"), frames(2)).expect("written");
    fs::write(folder.join("fleet.json"), frames(1)).expect("written");
    let refused = seed_from(&client, &folder, &[(&fleet, "fleet.json"), (&map, "map.json")]).await;
    let _ = fs::remove_dir_all(&folder);
    let Err(SeedError::UnmatchedHeldScope { held, unwritten, .. }) = &refused else { panic!("{refused:?}") };
    assert_eq!((held, unwritten), (&map, &fleet));
    let tree = client.tree().await.expect("the scope tree");
    assert_eq!((tree.head(&fleet), tree.head(&map)), (None, Some(Seq::new(1))), "nothing written, no scope made");
}

/// Section 3.13: the seed checks every held scope an entry to write can reference, not only the first, so where a
/// held entry seeded from this manifest's text of its document is listed before one seeded from another text, the
/// refusal names the second.
#[tokio::test]
async fn a_seed_names_a_held_entry_from_another_text_after_one_that_matches() {
    let client = client().await;
    let (folder, [map, fleet, flow]) = seed_folder_and_scopes(["t/map", "t/fleet", "t/flow"]);
    for document in ["map.json", "fleet.json", "flow.json"] {
        fs::write(folder.join(document), frames(1)).expect("written");
    }
    seed_from(&client, &folder, &[(&map, "map.json"), (&fleet, "fleet.json")]).await.expect("map and fleet seeded");
    fs::write(folder.join("fleet.json"), frames(2)).expect("written");
    let all = [(&map, "map.json"), (&fleet, "fleet.json"), (&flow, "flow.json")];
    let refused = seed_from(&client, &folder, &all).await;
    let _ = fs::remove_dir_all(&folder);
    let Err(SeedError::UnmatchedHeldScope { held, unwritten, .. }) = &refused else { panic!("{refused:?}") };
    assert_eq!((held, unwritten), (&fleet, &flow));
    assert_eq!(client.tree().await.expect("the scope tree").head(&flow), None, "no flow scope made");
}

/// Section 3.13: the seed trusts an entry's `depends_on` to name every document of another unit it pins into, so a new
/// template naming none, beside another template's map seeded from an earlier text, is written, and that map is left
/// as it stands.
#[tokio::test]
async fn a_seed_writes_an_entry_beside_a_scope_from_another_text_that_it_cannot_reference() {
    let client = client().await;
    let (folder, [stale, new]) = seed_folder_and_scopes(["t/map", "u/map"]);
    fs::write(folder.join("stale.json"), frames(1)).expect("written");
    seed_from(&client, &folder, &[(&stale, "stale.json")]).await.expect("the earlier map seeded");
    fs::write(folder.join("stale.json"), frames(2)).expect("written");
    fs::write(folder.join("new.json"), frames(1)).expect("written");
    let report = seed_from(&client, &folder, &[(&stale, "stale.json"), (&new, "new.json")]).await;
    let _ = fs::remove_dir_all(&folder);
    let report = report.expect("seeded beside the stale map");
    assert_eq!((report.written, report.skipped), (vec![(new, Seq::new(1))], vec![stale.clone()]));
    assert_eq!(client.wps().head(&stale), Some(Seq::new(1)), "the stale map left as it stands");
}

/// Section 3.13: a new template whose document depends on another template's map, seeded from an earlier text of its
/// document, can pin it, so the seed writes nothing and makes no scope, naming that map.
#[tokio::test]
async fn a_seed_writes_nothing_beside_a_scope_from_another_text_that_a_new_entry_depends_on() {
    let client = client().await;
    let (folder, [stale, new]) = seed_folder_and_scopes(["t/map", "u/map"]);
    fs::write(folder.join("stale.json"), frames(1)).expect("written");
    seed_from(&client, &folder, &[(&stale, "stale.json")]).await.expect("the earlier map seeded");
    let current = frames(2);
    fs::write(folder.join("stale.json"), &current).expect("written");
    fs::write(folder.join("new.json"), world(vec![frame()], &[&identity(&current)])).expect("written");
    let refused = seed_from(&client, &folder, &[(&stale, "stale.json"), (&new, "new.json")]).await;
    let _ = fs::remove_dir_all(&folder);
    let Err(SeedError::UnmatchedHeldScope { held, unwritten, .. }) = &refused else { panic!("{refused:?}") };
    assert_eq!((held, unwritten), (&stale, &new));
    assert_eq!(client.wps().head(&new), None, "no scope made");
}

/// Section 3.13: a manifest whose every scope is held, one of them seeded from an earlier text of its document, is
/// skipped whole, as before the check: the seed reads the scope tree once, writes nothing and asks no scope for a
/// changeset by its operation identifier.
#[tokio::test]
async fn a_seed_skips_a_manifest_whose_every_scope_is_held_though_one_text_changed() {
    let client = client().await;
    let (folder, [map, flow]) = seed_folder_and_scopes(["t/map", "t/flow"]);
    fs::write(folder.join("map.json"), frames(1)).expect("written");
    fs::write(folder.join("flow.json"), frames(1)).expect("written");
    let both = [(&map, "map.json"), (&flow, "flow.json")];
    seed_from(&client, &folder, &both).await.expect("both seeded");
    fs::write(folder.join("map.json"), frames(2)).expect("written");
    let before = client.wps().requests();
    let report = seed_from(&client, &folder, &both).await;
    let requests = client.wps().requests() - before;
    let _ = fs::remove_dir_all(&folder);
    let skipped = SeedReport { skipped: vec![map.clone(), flow.clone()], ..SeedReport::default() };
    assert_eq!(report.expect("skipped whole"), skipped);
    let heads = (client.wps().head(&map), client.wps().head(&flow));
    assert_eq!(heads, (Some(Seq::new(1)), Some(Seq::new(1))), "every head as it was");
    assert_eq!(requests, 1, "one read of the scope tree, and no scope asked for a changeset by operation");
}
