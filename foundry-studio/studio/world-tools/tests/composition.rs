use site_model::World;
use worldlens::Library;
use worldlens::Matcher;
use worldlens::Budget;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command};
use worldadapt_host::Manifest;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn scenario() -> PathBuf {
    root().join("data/seed/application-tests/lenses/fixtures/walmart-dallas/scenario.world.json")
}
fn manifest(derived: bool) -> Manifest {
    Manifest {
        name: "test".into(),
        version: "0.1.0".into(),
        format: "test".into(),
        projects: true,
        ingests: true,
        reads_derived: derived,
    }
}
fn entity_values(world: &World) -> Value {
    json!(world
        .entities
        .iter()
        .map(|(id, e)| (id.to_string(), json!({"name":e.name, "components":e.components, "parent":e.parent})))
        .collect::<BTreeMap<_, _>>())
}

#[test]
fn native_commands_preserve_expansion_and_flattened_runtime_meaning() {
    let scratch = tempfile::tempdir().unwrap();
    let (set, original) = site_model::load(&scenario()).unwrap();
    assert!(!original.overrides.is_empty());
    assert!(original.suppressions.iter().any(|s| s.excluded_by.is_none()));
    let expanded = scratch.path().join("expanded.json");
    let out = Command::new(env!("CARGO_BIN_EXE_foundry-world"))
        .args(["expand".as_ref(), scenario().as_os_str(), "--out".as_ref(), expanded.as_os_str()])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        fs::read_to_string(&expanded).unwrap(),
        site_model::expanded::to_canonical_string(&site_model::expanded::emit(&set, &original))
    );
    let flat = scratch.path().join("flattened.json");
    let out = Command::new(env!("CARGO_BIN_EXE_foundry-world"))
        .args(["flatten".as_ref(), scenario().as_os_str(), "--out".as_ref(), flat.as_os_str()])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let value: Value = serde_json::from_slice(&fs::read(&flat).unwrap()).unwrap();
    assert_eq!(value, foundry_world_tools::flatten::merged_document(&set, &original));
    assert_eq!(value["doc"]["id"], set.scenario.meta.id.to_string());
    assert_eq!(
        value["entities"].as_array().unwrap().len(),
        original.declared.len()
            + original.overrides.len()
            + original.suppressions.iter().filter(|s| s.excluded_by.is_none()).count()
    );
    let (_, loaded) = site_model::load(&flat).unwrap();
    assert_eq!(
        entity_values(&loaded),
        entity_values(&original),
        "all identities, labels, components and generating parents survive flattening"
    );
    assert_eq!(
        site_model::expanded::emit(&set, &loaded)["entities"].as_array().unwrap().len(),
        value["entities"].as_array().unwrap().len()
    );
    // Edges contain the compiled runtime topology, independent of file composition.
    assert_eq!(format!("{:?}", loaded.edges), format!("{:?}", original.edges));
}

#[test]
fn application_site_conversion_matches_explicit_generic_site_rows() {
    let scratch = tempfile::tempdir().unwrap();
    for derived in [false, true] {
        let manifest = manifest(derived);
        let native = foundry_world_tools::adapter::site(&scenario(), &manifest).unwrap();
        let explicit = json!({"scope":native.scope, "entities":native.entities.iter().map(|e|json!({"id":e.id,"name":e.name,"scope":e.scope,"derived":e.derived,"components":e.components})).collect::<Vec<_>>()});
        let path = scratch.path().join("site.json");
        fs::write(&path, explicit.to_string()).unwrap();
        assert_eq!(
            worldadapt_cli::read_site(&path, &manifest).unwrap(),
            native,
            "the generic host receives exactly the application helper's materialized entities"
        );
        assert_eq!(native.entities.iter().any(|e| e.derived), derived);
        assert!(native.entities.iter().any(|e| e.scope != native.scope), "catalog entities retain their owning scope");
    }
}

#[test]
fn preserved_application_lenses_are_publishable_and_fit_the_server_read_budget() {
    let (_, world) = site_model::load(&scenario()).unwrap();
    let text = fs::read_to_string(root().join("data/seed/application-tests/lenses/legacy-wps/world.lens")).unwrap();
    let shipped = Library::parse(&text).unwrap();
    assert_eq!(shipped.lenses().count(), 7, "the application library ships seven lenses");
    let matcher = Matcher::new(&world.entities);
    let limit = world_graph_server::storage::LENS_READ_BUDGET;
    let mut budget = Budget { steps: limit.steps / 100, bytes: limit.bytes / 100 };
    for lens in shipped.lenses() {
        let cut = Library::parse(&world_graph_cli::lenses::lens_text(&text, lens.name()).unwrap()).unwrap();
        let declared: Vec<&str> = cut.lenses().map(|l| l.name()).collect();
        assert_eq!(declared, vec![lens.name()], "one text registers one lens");
        let sliced = cut.lens(lens.name()).expect("the lens it was cut for");
        for pattern in sliced.patterns_used() {
            assert!(
                cut.get(pattern).is_some(),
                "`{}` runs `pattern {pattern}` and the cut left it behind",
                lens.name()
            );
        }
        matcher.project_within(sliced, &mut budget).unwrap();
    }
}
