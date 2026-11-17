//! Baseline v0.1 tiling, stamp, rack, cohort and carrier-pool outputs compared with Foundry native
//! expansion. Traversal-link topology changes are intentionally outside this cut.

use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use worldgen_host::{Budget, Engine, EntityView, ImportPolicy, Module, Object};
use site_model::canonical::to_canonical_string;
use site_model::pipeline::{apply, expand, graph, layers, merge};
use site_model::{Document, Entity, EntityId, LayerSet};

const FRAME: &str = "f0000000-0000-4000-8000-000000000001";
const GRID: &str = "a0000000-0000-4000-8000-000000000001";
const TEMPLATE: &str = "a0000000-0000-4000-8000-000000000002";
const STAMP: &str = "a0000000-0000-4000-8000-000000000003";

struct Generators {
    tiling: Module,
    stamp: Module,
    rack: Module,
    cohort: Module,
    carrier_pool: Module,
}

impl Generators {
    fn load() -> Self {
        let root = PathBuf::from(env::var_os("FOUNDRY_PARITY_MODULES").expect("module directory"));
        let engine = Engine::new().unwrap();
        let load = |name| Module::load(&engine, &fs::read(root.join(name)).unwrap(), ImportPolicy::TrapOnUse).unwrap();
        let tiling = load("tiling.wasm");
        let stamp = load("stamp.wasm");
        let manifest = tiling.describe(Budget::default()).unwrap();
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(manifest.writes, ["pose", "cell"]);
        assert_eq!(stamp.describe(Budget::default()).unwrap().version, "0.1.0");
        Self { tiling, stamp, rack: load("rack.wasm"), cohort: load("cohort.wasm"), carrier_pool: load("carrier_pool.wasm") }
    }

    fn expand(&self, initial: &BTreeMap<EntityId, Entity>, names: &[&str]) -> BTreeMap<EntityId, Entity> {
        // ordered: deterministic pass order and exact identity-set assertions.
        let mut entities = initial.clone();
        let mut visited = BTreeSet::new();
        for _ in 0..32 {
            let mut pending = vec![];
            for entity in entities.values() {
                for &name in names {
                    if !entity.has(name) || !visited.insert((entity.id.clone(), name)) {
                        continue;
                    }
                    let fields = if name == "stamp" {
                        let target: EntityId =
                            entity.components["stamp"]["template"].as_str().unwrap().parse().unwrap();
                        vec![("template".into(), vec![view(&entities[&target])])]
                    } else {
                        vec![]
                    };
                    let module = match name { "stamp" => &self.stamp, "tiling" => &self.tiling, "rack" => &self.rack, "cohort" => &self.cohort, "carrier_pool" => &self.carrier_pool, _ => unreachable!() };
                    let object = Object { anchor: view(entity), fields };
                    let mut session = module.instantiate(Budget::default()).unwrap();
                    let output = session.generate(name, &object).unwrap();
                    for row in output {
                        let id: EntityId = format!("{}/{}", entity.id, row.key).parse().unwrap();
                        let mut child = Entity::new(id);
                        child.parent = Some(entity.id.clone());
                        child.name = row.name;
                        child.components = serde_json::from_str(&row.components).unwrap();
                        pending.push(child);
                    }
                }
            }
            if pending.is_empty() {
                return entities;
            }
            for entity in pending {
                assert!(entities.insert(entity.id.clone(), entity).is_none(), "duplicate generator identity");
            }
        }
        panic!("generator passes did not settle");
    }

    fn compare(&self, set: &LayerSet, label: &str) -> (usize, usize) {
        let mut merged = merge::merge(set).unwrap();
        // These tests compare generation, before authored output overrides.
        merged.overrides.clear();
        merged.suppressions.clear();
        let expanded = expand::expand(&merged).unwrap();
        let expected = apply::apply(&merged, &expanded).unwrap().entities;
        let actual = self.expand(&merged.entities, &["stamp", "tiling", "rack", "cohort", "carrier_pool"]);
        let selected = |entity: &&Entity| entity.parent.is_some();
        let expected: BTreeMap<_, _> = expected.values().filter(selected).map(|e| (e.id.clone(), e)).collect();
        let actual: BTreeMap<_, _> = actual.values().filter(selected).map(|e| (e.id.clone(), e)).collect();
        assert_eq!(actual.keys().collect::<Vec<_>>(), expected.keys().collect::<Vec<_>>(), "{label}: identities");
        for (id, wanted) in &expected {
            let got = actual[id];
            assert_eq!(got.name, wanted.name, "{label}: {id}: name");
            assert_eq!(
                to_canonical_string(&json!(got.components)),
                to_canonical_string(&json!(wanted.components)),
                "{label}: {id}: components"
            );
        }
        let robots = actual.values().filter(|e| e.has("robot")).count();
        let carriers = actual.values().filter(|e| e.has("carrier")).count();
        if label == "fleet_shapes" { assert_eq!((robots, carriers), (8, 6)); }
        if label == "stamped_fleets" { assert_eq!((robots, carriers), (4, 6)); }
        let cells = actual.values().filter(|e| e.has("cell")).count();
        let links = actual.values().filter(|e| e.has("traversal_link")).count();
        eprintln!("{label}: {cells} cells, {links} traversal links, {} generated entities identical", actual.len());
        (cells, links)
    }
}

fn view(entity: &Entity) -> EntityView {
    EntityView {
        id: entity.id.to_string(),
        name: entity.name.clone(),
        components: json!(entity.components).to_string(),
    }
}

fn set(mut entities: Vec<Value>) -> LayerSet {
    entities.push(json!({"id":FRAME,"components":{"frame":{"dims":2,"units":"mm","unit_mm":1}}}));
    let raw = json!({"proto_version":"0.7.0","doc":{"id":"d0000000-0000-4000-8000-000000000001"},
                     "primary_frame":FRAME,"entities":entities});
    LayerSet { scenario: Document::from_value(Path::new("generator-parity"), &raw).unwrap(), layers: vec![] }
}

#[test]
fn baseline_tiling_preserves_odd_pitches_and_exclusions() {
    let generators = Generators::load();
    let input = set(vec![json!({"id":GRID,"components":{
        "region":{"frame":FRAME,"origin":[-301,17],"size":[1206,2004]},
        "tiling":{"pitch":[402,501],"exclusions":[{"origin":[402,501],"size":[402,1002]}],
                  "derives":{"cell":{"traversable_by":["htm"]},"custom":{"$comment":"not generated","value":17}}}
    }})]);
    assert_eq!(generators.compare(&input, "odd pitch with exclusion"), (10, 0));
}

#[test]
fn archived_rack_shapes_match_native_expansion() {
    compare_fixture("rack_shapes");
}

#[test]
fn archived_stamp_and_tiling_match_native_expansion() {
    compare_fixture("stamp_and_tiling");
}

#[test]
#[ignore = "Known v0.1/native quarter-turn size mismatch; fwd bJpVt6Au3gUIWjV0ApPZr3SBB0lXmw6Iung2OKSrzAg"]
fn archived_turned_cell_matches_native_expansion() {
    compare_fixture("turned_cell");
}

#[test]
fn archived_template_placeholders_match_native_expansion() {
    compare_fixture("template_placeholders");
}

fn compare_fixture(name: &str) {
    let root = PathBuf::from(env::var_os("FOUNDRY_PARITY_ROOT").expect("Foundry inputs"));
    let input = layers::resolve(&root.join(format!("data/seed/application-tests/generators/{name}.world.json"))).unwrap();
    Generators::load().compare(&input, name);
}

#[test]
#[ignore = "Known native geometry issue: oriented cell footprints are not composed; fwd bJpVt6Au3gUIWjV0ApPZr3SBB0lXmw6Iung2OKSrzAg"]
fn quarter_turn_rotates_the_physical_cell_footprint() {
    let input = set(vec![
        json!({"id":TEMPLATE,"components":{"template":{"entities":[
            {"local":"stand","components":{"pose":{"position":[0,0]},"cell":{"size":[3000,1000]}}},
            {"local":"tile","components":{"region":{"origin":[0,1000],"size":[3000,1000]},
             "tiling":{"pitch":[1500,1000],"derives":{"cell":{"size":[1500,500]}}}}}
        ]}}}),
        json!({"id":STAMP,"components":{"pose":{"frame":FRAME,"position":[0,0],"heading_deg":90},"stamp":{"template":TEMPLATE}}}),
    ]);
    let merged = merge::merge(&input).unwrap();
    let expansion = expand::expand(&merged).unwrap();
    let applied = apply::apply(&merged, &expansion).unwrap();
    let geometry = graph::cell_geometry(&applied.entities, &expansion.frames).unwrap();
    let footprint = |suffix: &str| {
        let id: EntityId = format!("{STAMP}/instance:0/local:{suffix}").parse().unwrap();
        geometry[&id].footprint.map(|step| step.0)
    };
    assert_eq!([footprint("stand"),footprint("tile/cell:0,0")], [[1000,3000],[500,1500]]);
}

#[test]
fn archived_fleet_shapes_match_native_expansion() {
    compare_fixture("fleet_shapes");
}

#[test]
fn nested_stamp_fleets_match_native_expansion() {
    compare_fixture("stamped_fleets");
}

#[test]
fn baseline_manifests_match_owned_input_lenses() {
    let generators = Generators::load();
    let root = PathBuf::from(env::var_os("FOUNDRY_PARITY_ROOT").unwrap()).join("data/seed/application");
    for (name, lens, module) in [
        ("tiling", "cell_grid", &generators.tiling), ("stamp", "stamped", &generators.stamp),
        ("rack", "racked", &generators.rack), ("cohort", "cohorted", &generators.cohort),
        ("carrier_pool", "pooled", &generators.carrier_pool),
    ] {
        let definition: Value = serde_json::from_str(&fs::read_to_string(root.join(format!("lenses/{lens}.json"))).unwrap()).unwrap();
        let manifest = module.describe(Budget::default()).unwrap();
        assert_eq!(manifest.name, name);
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(definition["lens"], lens);
        assert_eq!(definition["version"], manifest.version);
        assert_eq!(manifest.lens, fs::read_to_string(root.join(definition["text"]["path"].as_str().unwrap())).unwrap());
    }
}

// The original fleet comparisons ran cohort and carrier_pool independently of
// site geometry. Preserve that boundary and their exact census counts here.
fn compare_site_fleets(relative: &str, counts: (usize, usize)) {
    let root = PathBuf::from(env::var_os("FOUNDRY_PARITY_ROOT").unwrap());
    let set = layers::resolve(&root.join(relative)).unwrap();
    let mut merged = merge::merge(&set).unwrap();
    merged.overrides.clear();
    merged.suppressions.clear();
    let expected = apply::apply(&merged, &expand::expand(&merged).unwrap()).unwrap().entities;
    let actual = Generators::load().expand(&merged.entities, &["cohort", "carrier_pool"]);
    for (component, count) in [("robot", counts.0), ("carrier", counts.1)] {
        // ordered: compare every derived identity and its canonical payload.
        let rows = |entities: &BTreeMap<EntityId, Entity>| entities.values()
            .filter(|e| e.parent.is_some() && e.has(component))
            .map(|e| (e.id.clone(), (e.name.clone(), to_canonical_string(&json!(e.components)))))
            .collect::<BTreeMap<_,_>>();
        let expected_rows = rows(&expected);
        let actual_rows = rows(&actual);
        assert_eq!(actual_rows.len(), count, "{relative}: {component} census");
        assert_eq!(actual_rows, expected_rows, "{relative}: every {component} identity/name/component");
    }
}

#[test]
fn every_dallas_robot_and_carrier_matches_native_expansion() {
    compare_site_fleets("data/seed/application-tests/generators/sites/walmart-dallas/scenario.world.json", (15, 75));
}

#[test]
fn every_dfw6_robot_and_carrier_matches_native_expansion() {
    compare_site_fleets("data/seed/application-tests/generators/sites/walmart-dfw6/walmart_dfw6.world.json", (278, 8510));
}
