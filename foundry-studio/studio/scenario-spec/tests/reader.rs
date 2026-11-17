//! Spec 07 section 8, statement 5's last half: the generated files load with dessim's `Csv` kind through the
//! simulator's own reader (`harness::prepare`, then the scenario's resolution and placement as a run builds them),
//! with the fragment's every field stated so none rests on a serde default; and, with the flow compiler's setup in a
//! composition, the preload puts the fill of the slots on the racks, every SKU's start stock among it. The world is
//! dessim's own small relay fixture, `minimal_putaway.json`.

use std::fs;
use std::path::{Path, PathBuf};

use dessim::config::FidelityMode;
use dessim::harness::{RunOverrides as HarnessOverrides, prepare};
use dessim::runner::resolve_scenario_with_racking;
use dessim::scenario::ScenarioConfig;
use dessim::world_spec::{WorldSpec, load};
use dessim::worldmap;
use desunits::SimDuration;
use native_modules as _;
use rustc_hash::FxHashMap;
use scenario_spec::{Generated, RUN_FILES, SiteFacts, generate, site_facts, spec_fragment, stated_defaults};
use serde_json::{Value, json};
use studio_protocol::{FillPercent, OrdersPerHour, RunOverrides, ScenarioContent, Seed, SkuCount};
use tempfile::TempDir;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sim/dessim/tests/fixtures/minimal_putaway.json")
}

/// A small workload: the fixture holds few slots.
fn content() -> ScenarioContent {
    let mut content = stated_defaults();
    let generator = &mut content.workload.generator;
    generator.orders_per_hour = OrdersPerHour(20.0);
    generator.inventory_fill_pct = FillPercent(50.0);
    generator.sku_count = SkuCount(30);
    generator.seed = Seed(4);
    content.run_controls.duration_sim_ms = SimDuration::ms(3_600_000);
    content
}

/// A run's folder as spec 08 section 3.5 lays it out: `spec.json`, the world and the two generated files.
fn folder(spec: &Value, workload: &Generated) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("a scratch folder");
    for (file, bytes) in [
        (RUN_FILES.world, fs::read(fixture()).expect("the fixture reads")),
        (RUN_FILES.items, workload.items_csv()),
        (RUN_FILES.orders, workload.orders_csv()),
    ] {
        let path = dir.path().join(file);
        fs::create_dir_all(path.parent().expect("a file has a folder")).expect("the folder is made");
        fs::write(path, bytes).expect("the file is written");
    }
    let spec_path = dir.path().join("spec.json");
    fs::write(&spec_path, serde_json::to_vec_pretty(spec).expect("JSON")).expect("spec.json is written");
    (dir, spec_path)
}

/// The workload for the fixture's slots; its flow is the composition's, which brings stock in, since the fixture
/// holds no `flow.` entities to say so.
fn workload(content: &ScenarioContent, brings_stock_in: bool) -> Generated {
    let world = worldmap::load(&fixture()).expect("the fixture loads");
    let facts = site_facts(&world);
    assert!(facts.slots.0 > 0 && !facts.brings_stock_in, "the fixture holds inventory slots and no flow");
    let facts = SiteFacts { brings_stock_in, ..facts };
    generate(&content.workload.generator, content.run_controls.duration_sim_ms, facts)
}

#[test]
fn the_files_load_with_dessims_csv_kind_and_every_order_is_read() {
    let content = content();
    let made = workload(&content, false);
    let spec = spec_fragment(&content, &RunOverrides::default(), RUN_FILES);
    let (_dir, spec_path) = folder(&spec, &made);
    prepare(&spec_path, &HarnessOverrides::default()).expect("the simulator's reader prepares the run");
    let loaded = load(&spec_path).expect("the spec loads");
    let ScenarioConfig::Csv(_) = &loaded.scenario else { panic!("the workload block reads as dessim's Csv kind") };
    let data =
        resolve_scenario_with_racking(loaded.world.map, loaded.world.racking, &loaded.scenario, &loaded.settings)
            .expect("the scenario resolves and its stock is placed");
    assert_eq!(data.orders.len() as u64, made.header.orders.0);
    assert_eq!(data.skus.len() as u64, made.header.skus.0);
}

#[test]
fn the_fragment_states_every_field_it_owns_so_none_rests_on_a_serde_default() {
    let content = content();
    let spec = spec_fragment(&content, &RunOverrides::default(), RUN_FILES);
    for pointer in [
        "/world",
        "/scenario/items_csv",
        "/scenario/orders_csv",
        "/scenario/units_per_tote",
        "/settings/sim_duration/secs",
        "/settings/sim_duration/nanos",
        "/settings/rng_seed",
        "/settings/fidelity",
        "/settings/vtm/isolation",
        "/settings/slice_width_ms",
        "/settings/order/release_all_at_start",
        "/settings/tripwires",
    ] {
        assert!(spec.pointer(pointer).is_some(), "the fragment states {pointer}");
    }
    let read: WorldSpec = serde_json::from_value(spec).expect("dessim's spec format reads the fragment");
    assert_eq!(read.settings.sim_duration.as_millis(), 3_600_000);
    assert_eq!(read.settings.rng_seed, 0);
    assert_eq!(read.settings.fidelity, FidelityMode::CollisionDes);
    assert!(!read.settings.order.release_all_at_start);
    assert!(read.settings.wall_timeout_secs.is_none() && !read.settings.no_watchdog, "no wall-clock stop; watchdog on");
    let ScenarioConfig::Csv(csv) = read.scenario else { panic!("the Csv kind") };
    assert_eq!(csv.units_per_tote, content.workload.generator.units_per_stored_tote.0);
}

/// A fill of F on S slots starts F x S stocked totes, every SKU's start stock among them, however much the orders
/// bring in through putaway. Here they bring in more than the racks start with, so the preload fraction is small: it is
/// each SKU's share on the racks, and does not also cut the slots that single-SKU stock may go to. dessim rounds each
/// SKU's share up, by at most a tote.
#[test]
fn with_the_setup_composed_the_fill_of_the_slots_starts_on_the_racks_with_every_skus_start_stock() {
    let mut content = content();
    let generator = &mut content.workload.generator;
    generator.inventory_fill_pct = FillPercent(80.0);
    generator.sku_count = SkuCount(10);
    generator.orders_per_hour = OrdersPerHour(40.0);
    let made = workload(&content, true);
    assert!(made.header.preload_fraction.0 < 0.5, "the orders bring in more than the racks start with");
    let world = worldmap::load(&fixture()).expect("the fixture loads");
    let fill = (content.workload.generator.inventory_fill_pct.0 / 100.0 * site_facts(&world).slots.0 as f64).round();
    let mut spec = spec_fragment(&content, &RunOverrides::default(), RUN_FILES);
    let cencora: Value =
        serde_json::from_str(include_str!("../../../sim/dessim/compositions/cencora.json")).expect("JSON");
    let mut tasks = cencora["tasks"].as_array().expect("a task list").clone();
    for task in &mut tasks {
        if task["type"] == "FillInventory" {
            task["config"] = json!({ "preload_fraction": made.header.preload_fraction.0 });
        }
    }
    spec["composition"] = json!({ "tasks": tasks });
    let (_dir, spec_path) = folder(&spec, &made);
    let loaded = load(&spec_path).expect("the spec loads");
    let data =
        resolve_scenario_with_racking(loaded.world.map, loaded.world.racking, &loaded.scenario, &loaded.settings)
            .expect("the scenario resolves");

    let mut demand: FxHashMap<&str, u64> = FxHashMap::default();
    for line in made.orders.iter().flat_map(|order| &order.lines) {
        *demand.entry(line.sku.as_str()).or_default() += line.quantity.0;
    }
    assert_eq!(data.skus.len(), made.items.len(), "dessim's reader numbers the SKUs in the items file's order");
    for (sku, item) in data.skus.iter().zip(&made.items) {
        let start = item.units.0 - demand.get(item.sku.as_str()).copied().unwrap_or(0);
        let held: u64 =
            data.inventory.iter().filter(|((id, _), _)| *id == sku.id).map(|(_, &qty)| u64::from(qty)).sum();
        assert!(
            (start..=start + 1).contains(&held),
            "{}: {held} units on the racks at simtime 0, {start} to start",
            item.sku
        );
    }
    let stocked = data.tote_placements.iter().filter(|tote| tote.has_tasks).count() as f64;
    let rounding = made.items.len() as f64;
    assert!((fill..=fill + rounding).contains(&stocked), "{stocked} totes stocked at simtime 0 for a fill of {fill}");
}
