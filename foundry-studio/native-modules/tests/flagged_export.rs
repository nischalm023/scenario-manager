//! A run built from a site's flagged export, the whole site as WPS generated it with every entity it references
//! (rulings D175 and D185).
//!
//! The export here is the small EasyLoop fixture as the native pipeline derives it, written out the way WPS writes a
//! site with all three flags: every entity once, no prologue and no records. No authored document reads that way,
//! since it states no `proto_version` and no `doc`, so a load that succeeds took the flagged export's path. The
//! harness's load of a spec naming the file, the run, its recording and the recording's reload of the file it carries
//! all read it so, and they build the same map, and run the same run, as the authored fixture.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use dessim::harness::{RecordingTarget, RunOverrides, prepare};
use dessim::metrics::profile::PerfProfile;
use dessim::metrics::worldview::read_worldview;
use dessim::runner::RunnerHooks;
use dessim::world_spec;
use dessim::worldmap::{WorldMap, load, load_from_bytes};
use native_modules as _;
use serde_json::{Value, json};
use site_model::flagged_export;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

/// The EasyLoop fixture's spec beside the fixture's flagged export, both written into `dir`; and the export's bytes.
fn flagged_spec(dir: &Path) -> (PathBuf, Vec<u8>) {
    let (_, authored) = site_model::load(&fixture("minimal_easyloop.json")).expect("the authored fixture loads");
    let file = flagged_export::written_out(&authored, "customer/c/site/easyloop");
    let bytes = serde_json::to_vec(&file).expect("an export encodes");
    fs::write(dir.join("world.json"), &bytes).expect("the export is written");
    let mut spec: Value =
        serde_json::from_slice(&fs::read(fixture("minimal_easyloop_spec.json")).expect("the spec reads")).unwrap();
    spec["world"] = json!("world.json");
    let path = dir.join("spec.json");
    fs::write(&path, serde_json::to_vec_pretty(&spec).expect("a spec encodes")).expect("the spec is written");
    (path, bytes)
}

/// Two maps built from one site: the same entities, arcs and cells, and the same dessim cells, relays, stations and
/// slots made of them.
fn same_map(built: &WorldMap, authored: &WorldMap) {
    assert_eq!(built.world.entities.keys().collect::<Vec<_>>(), authored.world.entities.keys().collect::<Vec<_>>());
    assert_eq!(built.world.edges, authored.world.edges);
    assert_eq!(built.world.frames.primary(), authored.world.frames.primary());
    assert_eq!(built.index.cell_entities, authored.index.cell_entities);
    assert_eq!(built.index.relay_shelves, authored.index.relay_shelves);
    assert_eq!(built.index.station_entities, authored.index.station_entities);
    assert_eq!(built.index.storage_slots, authored.index.storage_slots);
    assert_eq!(built.cell_sizes_mm, authored.cell_sizes_mm);
    assert_eq!(built.cell_headings_rad, authored.cell_headings_rad);
    assert_eq!(built.map.fleet_config().htms, authored.map.fleet_config().htms);
}

/// A run's harness reads the world its spec names through `world_spec::load`, and a flagged export builds the map
/// the authored fixture does.
#[test]
fn a_spec_naming_a_flagged_export_loads_its_world() {
    let dir = tempfile::tempdir().expect("a folder");
    let (spec, _) = flagged_spec(dir.path());
    let loaded = world_spec::load(&spec).expect("the spec's flagged export loads");
    let authored = load(&fixture("minimal_easyloop.json")).expect("the authored fixture loads");
    same_map(&loaded.world, &authored);
    assert!(loaded.world.world.overrides.is_empty() && loaded.world.world.suppressions.is_empty());
}

/// A recording carries its world byte for byte, and `load_from_bytes` reads a flagged export from those bytes.
#[test]
fn a_flagged_export_loads_from_its_bytes() {
    let dir = tempfile::tempdir().expect("a folder");
    let (_, bytes) = flagged_spec(dir.path());
    let built = load_from_bytes(&bytes).expect("the flagged export loads from its bytes");
    same_map(&built, &load(&fixture("minimal_easyloop.json")).expect("the authored fixture loads"));
}

/// An authored document that also carries a flagged export's `scope` is read neither way, by `load` or by
/// `load_from_bytes`: each refuses it naming both sets of keys, where taking `scope` alone as the mark would skip the
/// native reading.
#[test]
fn an_authored_document_marked_flagged_is_refused() {
    let mut authored: Value =
        serde_json::from_slice(&fs::read(fixture("minimal_easyloop.json")).expect("the fixture reads")).unwrap();
    authored["scope"] = json!("customer/c/site/easyloop");
    let bytes = serde_json::to_vec(&authored).expect("encodes");
    let dir = tempfile::tempdir().expect("a folder");
    let path = dir.path().join("marked.json");
    fs::write(&path, &bytes).expect("written");
    for refused in [load(&path).map(|_| ()), load_from_bytes(&bytes).map(|_| ())] {
        let text = refused.expect_err("a file marked both ways is refused").to_string();
        assert!(text.contains("`scope`") && text.contains("`proto_version`"), "{text}");
    }
}

/// A flagged export without all three of its flags builds no map, by `load` or by `load_from_bytes`: each refuses it
/// naming the flag it leaves unset, as a site exported without `referenced` may name an entity it does not hold.
#[test]
fn a_flagged_export_without_all_three_flags_is_refused() {
    let dir = tempfile::tempdir().expect("a folder");
    let (_, bytes) = flagged_spec(dir.path());
    let mut unset: Value = serde_json::from_slice(&bytes).expect("the export reads");
    unset["referenced"] = json!(false);
    let bytes = serde_json::to_vec(&unset).expect("encodes");
    let path = dir.path().join("unset.json");
    fs::write(&path, &bytes).expect("written");
    for refused in [load(&path).map(|_| ()), load_from_bytes(&bytes).map(|_| ())] {
        let text = refused.expect_err("a flagged export with `referenced` unset is refused").to_string();
        assert!(text.contains("`referenced`") && text.contains("?nested=1&derived=1&referenced=1"), "{text}");
    }
}

/// A short run from the flagged export is the authored fixture's run, picks and events alike; its recording names
/// the frame that states `unit_mm` as the one its positions are measured in, reads back, and reloads the world it
/// carries.
#[test]
fn a_run_from_a_flagged_export_is_recorded_and_read_back() {
    let dir = tempfile::tempdir().expect("a folder");
    let (spec, bytes) = flagged_spec(dir.path());
    let overrides =
        RunOverrides { sim_duration: Some(Duration::from_secs(300)), num_threads: Some(1), ..RunOverrides::default() };
    let path = dir.path().join("run.worldview");
    let target = RecordingTarget { path: path.clone(), run: String::new(), attempt: 0, tap: None };
    let (profile, _) = prepare(&spec, &overrides)
        .expect("the run prepares")
        .execute(RunnerHooks::default(), Some(target))
        .expect("the recording is written");
    let (authored, _) = prepare(&fixture("minimal_easyloop_spec.json"), &overrides)
        .expect("prepares")
        .execute(RunnerHooks::default(), None)
        .expect("nothing is recorded");
    let counted = |profile: &PerfProfile| {
        let result = &profile.sim_result;
        (result.total_picks, result.total_orders_completed, result.completed_order_picks, result.event_count)
    };
    assert_eq!(counted(&profile), counted(&authored), "the same run");
    assert!(counted(&profile).0 > 0, "a run that picks: {:?}", counted(&profile));

    let recording = read_worldview(&path).expect("the recording reads back");
    let primary = load_from_bytes(&bytes).expect("loads").world.frames.primary().map(ToString::to_string);
    assert_eq!(Some(recording.profile.ids.frame.clone()), primary, "the frame that states unit_mm");
    assert_eq!(recording.profile.documents.world, bytes, "the recording carries the export byte for byte");
    let reloaded = load_from_bytes(&recording.profile.documents.world).expect("the recording's world reloads");
    same_map(&reloaded, &load(&fixture("minimal_easyloop.json")).expect("the authored fixture loads"));
}
