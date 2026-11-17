//! Within `scenarios`, `fields` is spec 07 section 3.2's field catalogue: one static table naming each scenario field
//! once, its pointer, label, unit, clock and, for a closed value, the words of each value, so the panel, the version
//! diff, the tool summaries and the approval card's rows give a field one name everywhere. It also says which fields
//! a sweep may vary (section 3.6: the generator's fields and the floor traffic), and which pointers an edit may set.
//! A closed field offers only the values this build runs (section 5.1), so a sweep varies no field with fewer than
//! two of them, nor a field read only with a value the build does not run.

use std::sync::LazyLock;

use scenario_spec::{RELEASE, this_build_accepts};
use studio_protocol::{Clock, FieldCatalogue, FieldEntry, FieldOption};

/// The pointers of the scenario row an edit may also set; they open no version.
pub const ROW_FIELDS: [&str; 2] = ["/name", "/description"];

const HOUR: f64 = 3_600_000.0;
const MINUTE: f64 = 60_000.0;

struct Row {
    pointer: &'static str,
    label: &'static str,
    short_label: &'static str,
    unit: &'static str,
    per_unit: f64,
    clock: Option<Clock>,
    sweepable: bool,
    options: &'static [(&'static str, &'static str)],
    hint: &'static str,
}

/// The setting of a runner image's `accepts` that names which values of a closed field it runs; a closed field not
/// listed runs every value.
const SETTINGS: [(&str, &str); 1] = [("/workload/generator/release", RELEASE)];
/// A field a run reads only with one value of a setting: the wave interval, only with waves.
const READ_WITH: [(&str, (&str, &str)); 1] = [("/workload/generator/wave_every_sim_ms", (RELEASE, "waves"))];

const RELEASES: &[(&str, &str)] = &[
    ("all_at_start", "All at the start: every order is there at 0 s of simtime"),
    ("even", "Even through each hour: evenly spaced at the hourly rate"),
    ("random", "Random arrivals at the hourly rate: gaps drawn from the generator's seed"),
    ("waves", "Waves: the orders of each interval together at its start"),
];

const TRAFFIC: &[(&str, &str)] = &[
    ("reserved", "Reserved cells: robots book the cells they cross"),
    ("ghost", "Ghost moves: robots pass through each other"),
];

const ROWS: &[Row] = &[
    Row {
        pointer: "/workload/generator/orders_per_hour",
        label: "Orders per hour",
        short_label: "rate",
        unit: "orders an hour",
        per_unit: 1.0,
        clock: None,
        sweepable: true,
        options: &[],
        hint: "The arrival rate, 1 to 10,000; the default is 300.",
    },
    Row {
        pointer: "/workload/generator/lines_per_order",
        label: "Lines per order, on average",
        short_label: "lines",
        unit: "lines",
        per_unit: 1.0,
        clock: None,
        sweepable: true,
        options: &[],
        hint: "1 to 50; the default is 3.",
    },
    Row {
        pointer: "/workload/generator/sku_count",
        label: "SKUs",
        short_label: "SKUs",
        unit: "SKUs",
        per_unit: 1.0,
        clock: None,
        sweepable: true,
        options: &[],
        hint: "1 to 100,000; the default is 400.",
    },
    Row {
        pointer: "/workload/generator/inventory_fill_pct",
        label: "Inventory fill",
        short_label: "fill",
        unit: "% of the inventory tote slots",
        per_unit: 1.0,
        clock: None,
        sweepable: true,
        options: &[],
        hint: "Stocked at 0 s of simtime, 0 to 100; the default is 80.",
    },
    Row {
        pointer: "/workload/generator/units_per_stored_tote",
        label: "Units per stored tote",
        short_label: "units a tote",
        unit: "units",
        per_unit: 1.0,
        clock: None,
        sweepable: true,
        options: &[],
        hint: "What one stored tote holds when the stock is placed, 1 to 10,000; the default is 1.",
    },
    Row {
        pointer: "/workload/generator/release",
        label: "Release",
        short_label: "release",
        unit: "",
        per_unit: 1.0,
        clock: None,
        sweepable: true,
        options: RELEASES,
        hint: "When the orders arrive in simtime; the default is all at the start.",
    },
    Row {
        pointer: "/workload/generator/wave_every_sim_ms",
        label: "Wave interval",
        short_label: "waves every",
        unit: "minutes of simtime",
        per_unit: MINUTE,
        clock: Some(Clock::Sim),
        sweepable: true,
        options: &[],
        hint: "Waves only: 1 to 60 minutes of simtime between two waves.",
    },
    Row {
        pointer: "/workload/generator/seed",
        label: "Generator seed",
        short_label: "seed",
        unit: "",
        per_unit: 1.0,
        clock: None,
        sweepable: true,
        options: &[],
        hint: "The same values and seed always make the same orders and stock; the default is 0.",
    },
    Row {
        pointer: "/workload/order_trays",
        label: "Order trays",
        short_label: "trays",
        unit: "trays",
        per_unit: 1.0,
        clock: None,
        sweepable: false,
        options: &[],
        hint: "FillInventory's order trays; 0 where the flow stores none.",
    },
    Row {
        pointer: "/run_controls/duration_sim_ms",
        label: "Duration",
        short_label: "duration",
        unit: "hours of simtime",
        per_unit: HOUR,
        clock: Some(Clock::Sim),
        sweepable: false,
        options: &[],
        hint: "One minute to seven days of simtime; the generator makes orders for as long. The default is 4 hours.",
    },
    Row {
        pointer: "/run_controls/seed",
        label: "Simulation seed",
        short_label: "simulation seed",
        unit: "",
        per_unit: 1.0,
        clock: None,
        sweepable: false,
        options: &[],
        hint: "Each repeat runs with the next one.",
    },
    Row {
        pointer: "/run_controls/floor_traffic",
        label: "Floor traffic",
        short_label: "traffic",
        unit: "",
        per_unit: 1.0,
        clock: None,
        sweepable: true,
        options: TRAFFIC,
        hint: "How robots share the floor; the default is reserved cells.",
    },
    Row {
        pointer: "/run_controls/report_window/from_sim_ms",
        label: "Report window start",
        short_label: "report from",
        unit: "hours of simtime",
        per_unit: HOUR,
        clock: Some(Clock::Sim),
        sweepable: false,
        options: &[],
        hint: "What a run's summary covers starts here, on a 10 s slice end.",
    },
    Row {
        pointer: "/run_controls/report_window/to_sim_ms",
        label: "Report window end",
        short_label: "report to",
        unit: "hours of simtime",
        per_unit: HOUR,
        clock: Some(Clock::Sim),
        sweepable: false,
        options: &[],
        hint: "Empty for the end of the run.",
    },
    Row {
        pointer: "/run_controls/report_window/before_name",
        label: "Name of the excluded start",
        short_label: "before",
        unit: "",
        per_unit: 1.0,
        clock: None,
        sweepable: false,
        options: &[],
        hint: "What the start the summary leaves out is called, such as warm-up.",
    },
];

static CATALOGUE: LazyLock<FieldCatalogue> = LazyLock::new(|| {
    let accepts = this_build_accepts();
    let runs =
        |setting: &str, value: &str| accepts.get(setting).is_some_and(|values| values.iter().any(|v| v == value));
    let items = ROWS
        .iter()
        .map(|row| {
            let setting = SETTINGS.iter().find(|(pointer, _)| *pointer == row.pointer).map(|(_, setting)| *setting);
            let read_with = READ_WITH.iter().find(|(pointer, _)| *pointer == row.pointer).map(|(_, needs)| *needs);
            let options: Vec<FieldOption> = row
                .options
                .iter()
                .map(|(value, words)| FieldOption {
                    value: (*value).to_owned(),
                    words: (*words).to_owned(),
                    accepted: setting.is_none_or(|setting| runs(setting, value)),
                })
                .collect();
            let choices = options.iter().filter(|option| option.accepted).count();
            FieldEntry {
                pointer: row.pointer.to_owned(),
                label: row.label.to_owned(),
                short_label: row.short_label.to_owned(),
                unit: row.unit.to_owned(),
                per_unit: row.per_unit,
                clock: row.clock,
                sweepable: row.sweepable
                    && (options.is_empty() || choices >= 2)
                    && read_with.is_none_or(|(setting, value)| runs(setting, value)),
                options,
                hint: row.hint.to_owned(),
            }
        })
        .collect();
    FieldCatalogue { items }
});

/// The catalogue, every field in pointer order of the content.
pub fn catalogue() -> &'static FieldCatalogue {
    &CATALOGUE
}

pub fn entry(pointer: &str) -> Option<&'static FieldEntry> {
    CATALOGUE.items.iter().find(|entry| entry.pointer == pointer)
}

/// The words for the fields of a request that are not a scenario's content.
const REQUEST_FIELDS: [(&str, &str); 7] = [
    ("/name", "Name"),
    ("/description", "Description"),
    ("/set", "The edit"),
    ("/revision", "The revision"),
    ("/repeats", "Repeats"),
    ("/varies/values", "The sweep's values"),
    ("/run_overrides/duration_sim_ms", "The check run's duration"),
];

/// A field's label, or its pointer for one neither the catalogue nor a request holds.
pub fn label(pointer: &str) -> String {
    if let Some((_, words)) = REQUEST_FIELDS.iter().find(|(field, _)| *field == pointer) {
        return (*words).to_owned();
    }
    entry(pointer).map_or_else(|| pointer.to_owned(), |entry| entry.label.clone())
}

/// Whether an edit may set `pointer` in a version's content: a field of the catalogue or a group holding fields.
pub fn settable(pointer: &str) -> bool {
    !pointer.is_empty()
        && CATALOGUE
            .items
            .iter()
            .any(|entry| entry.pointer == pointer || entry.pointer.starts_with(&format!("{pointer}/")))
}
