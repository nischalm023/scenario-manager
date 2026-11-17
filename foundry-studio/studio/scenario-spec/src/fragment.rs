//! Within `scenario-spec`, `fragment` is spec 07 section 3.2's map from a scenario version to what a run starts with
//! (`spec_fragment`, the run's spec field by field) and what the flow compiler takes in run mode (`setup` and
//! `order_kinds`, spec 06 section 3.2). The fragment states every field a run reads from the scenario, and the
//! settings this proof of concept fixes, so none rests on a serde default of dessim's. `Compiled::spec` adds the
//! configured flow's composition and settings to this fragment before readiness or a run reads it.

use dessim::composition::flowgraph::Container;
use dessim::model::ExitPath;
use serde::Serialize;

pub use dessim::composition::flowgraph::OrderKind;
use serde_json::{Value, json};
use studio_protocol::{FloorTraffic, OrderTrays, PreloadFraction, RunOverrides, ScenarioContent};

use crate::content::{SLICE_MS, effective};
use crate::generator::Generated;

/// Where a run's folder holds the world and the two generated files, each relative to `spec.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpecFiles<'a> {
    pub world: &'a str,
    pub items: &'a str,
    pub orders: &'a str,
}

/// The run folder's layout spec 08 section 3.5 names.
pub const RUN_FILES: SpecFiles<'static> =
    SpecFiles { world: "world/site.world.json", items: "data/items.csv", orders: "data/orders.csv" };

/// The run's `spec.json`: the world, the workload block of dessim's `Csv` kind naming the generated files, and the
/// settings from the run controls with the experiment's overrides and the fixed ones (no wall-clock timeout and the
/// watchdog on, by leaving `wall_timeout_secs` and `no_watchdog` out, as spec 08 section 3.5 asks).
pub fn spec_fragment(content: &ScenarioContent, overrides: &RunOverrides, files: SpecFiles<'_>) -> Value {
    let (duration, _) = effective(content, overrides);
    let ms = duration.as_ms();
    let fidelity = match content.run_controls.floor_traffic {
        FloorTraffic::Reserved => "CollisionDes",
        FloorTraffic::Ghost => "Ghost",
    };
    json!({
        "world": files.world,
        "scenario": {
            "items_csv": files.items,
            "orders_csv": files.orders,
            "units_per_tote": content.workload.generator.units_per_stored_tote.0,
        },
        "settings": {
            "sim_duration": { "secs": ms / 1000, "nanos": (ms % 1000) * 1_000_000 },
            "rng_seed": content.run_controls.seed.0,
            "fidelity": fidelity,
            "vtm": { "isolation": "off" },
            "slice_width_ms": SLICE_MS,
            "order": { "release_all_at_start": false },
            "tripwires": false,
        },
    })
}

/// FillInventory's setup for spec 06's run mode: the generator's preload fraction and the version's order trays.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Setup {
    pub stock_at_start: PreloadFraction,
    pub order_trays: OrderTrays,
}

pub fn setup(content: &ScenarioContent, workload: &Generated) -> Setup {
    Setup { stock_at_start: workload.header.preload_fraction, order_trays: content.workload.order_trays }
}

/// Every order the `Csv` kind reads is a tote order leaving by the dock (dessim's `OrderSpec::unstated`), so a
/// generated workload holds that one kind.
pub fn order_kinds() -> Vec<OrderKind> {
    vec![OrderKind { container: Container::Tote, exit_path: ExitPath::Dock, routing: None }]
}
