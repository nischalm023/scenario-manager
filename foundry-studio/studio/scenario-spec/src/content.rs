//! Within `scenario-spec`, `content` is what a scenario version may hold (spec 07 section 3.2): the stated defaults
//! a new version starts from, so no default of dessim's decides a run, and the checks every save runs over the whole
//! merged content, the ranges and the rules that join fields, each finding naming its field by JSON pointer. A
//! finding says what the field takes; the API's field catalogue supplies the field's label.

use desunits::{SimDuration, SimTime};
use studio_protocol::{
    FillPercent, FloorTraffic, GeneratorSpec, LinesPerOrder, OrderTrays, OrdersPerHour, Release, ReportWindow,
    RunControls, RunOverrides, ScenarioContent, Seed, SkuCount, UnitsPerTote, Workload,
};

/// The milliseconds of simtime in an hour.
pub const HOUR_MS: u64 = 3_600_000;
/// The metrics slice every run records (spec 07 section 3.2, fixed for the proof of concept); a report window's ends
/// fall on its multiples.
pub const SLICE_MS: u64 = 10_000;
/// The largest seed JSON holds exactly.
pub const SEED_LIMIT: u64 = (1 << 53) - 1;

/// One field a check refuses: its pointer, and what it takes, worded to follow the field's label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub pointer: String,
    pub rule: String,
}

/// Section 3.2's stated defaults: 300 orders an hour, 3 lines an order, 400 SKUs, 80 % fill, 1 unit a stored tote,
/// every order at the start, generator seed 0, no order trays; 4 h of simtime, simulation seed 0, reserved cells,
/// the report window from 0 to the end.
pub fn stated_defaults() -> ScenarioContent {
    ScenarioContent {
        workload: Workload {
            generator: GeneratorSpec {
                orders_per_hour: OrdersPerHour(300.0),
                lines_per_order: LinesPerOrder(3.0),
                sku_count: SkuCount(400),
                inventory_fill_pct: FillPercent(80.0),
                units_per_stored_tote: UnitsPerTote(1),
                release: Release::AllAtStart,
                wave_every_sim_ms: None,
                seed: Seed(0),
            },
            order_trays: OrderTrays(0),
        },
        run_controls: RunControls {
            duration_sim_ms: SimDuration::ms(4 * HOUR_MS),
            seed: Seed(0),
            floor_traffic: FloorTraffic::Reserved,
            report_window: ReportWindow {
                from_sim_ms: SimTime::ms(0),
                to_sim_ms: None,
                before_name: "warm-up".to_owned(),
            },
        },
    }
}

/// Every check of section 3.2 over `content`, in pointer order; empty when it may be saved.
pub fn validate(content: &ScenarioContent) -> Vec<Violation> {
    let mut found = validate_generator(&content.workload.generator, "/workload/generator");
    let controls = &content.run_controls;
    duration(&mut found, "/run_controls/duration_sim_ms", controls.duration_sim_ms);
    seed(&mut found, "/run_controls/seed".to_owned(), controls.seed);
    found.extend(window("/run_controls/report_window", &controls.report_window, controls.duration_sim_ms));
    found
}

/// The generator spec's checks, its pointers under `under`: the version's `/workload/generator` or the endpoint
/// body's `/generator`.
pub fn validate_generator(generator: &GeneratorSpec, under: &str) -> Vec<Violation> {
    let mut found = Vec::new();
    let at = |field: &str| format!("{under}/{field}");
    within(&mut found, at("orders_per_hour"), generator.orders_per_hour.0, 1.0, 10_000.0);
    within(&mut found, at("lines_per_order"), generator.lines_per_order.0, 1.0, 50.0);
    within(&mut found, at("sku_count"), f64::from(generator.sku_count.0), 1.0, 100_000.0);
    within(&mut found, at("inventory_fill_pct"), generator.inventory_fill_pct.0, 0.0, 100.0);
    within(&mut found, at("units_per_stored_tote"), f64::from(generator.units_per_stored_tote.0), 1.0, 10_000.0);
    match (generator.release, generator.wave_every_sim_ms) {
        (Release::Waves, None) => found.push(Violation {
            pointer: at("wave_every_sim_ms"),
            rule: "is needed with waves: the simtime between two waves".to_owned(),
        }),
        (_, Some(every)) => within(&mut found, at("wave_every_sim_ms"), every.as_ms() as f64, 60_000.0, 3_600_000.0),
        (_, None) => {}
    }
    seed(&mut found, at("seed"), generator.seed);
    found
}

/// The checks of an experiment's run overrides against the content they override.
pub fn validate_overrides(content: &ScenarioContent, overrides: &RunOverrides) -> Vec<Violation> {
    let mut found = Vec::new();
    if let Some(stated) = overrides.duration_sim_ms {
        duration(&mut found, "/run_overrides/duration_sim_ms", stated);
    }
    let (duration, window_now) = effective(content, overrides);
    if overrides.duration_sim_ms.is_some() || overrides.report_window.is_some() {
        let pointer = if overrides.report_window.is_some() {
            "/run_overrides/report_window"
        } else {
            "/run_controls/report_window"
        };
        found.extend(window(pointer, &window_now, duration));
    }
    found
}

/// The duration and report window a run takes: the version's, unless the experiment overrides them.
pub fn effective(content: &ScenarioContent, overrides: &RunOverrides) -> (SimDuration, ReportWindow) {
    let duration = overrides.duration_sim_ms.unwrap_or(content.run_controls.duration_sim_ms);
    let window = overrides.report_window.clone().unwrap_or_else(|| content.run_controls.report_window.clone());
    (duration, window)
}

fn within(found: &mut Vec<Violation>, pointer: String, value: f64, least: f64, most: f64) {
    if !(least..=most).contains(&value) {
        found.push(Violation {
            pointer,
            rule: format!("takes {} to {}; this is {}", words(least), words(most), words(value)),
        });
    }
}

fn seed(found: &mut Vec<Violation>, pointer: String, seed: Seed) {
    if seed.0 > SEED_LIMIT {
        found.push(Violation { pointer, rule: format!("takes a whole number 0 to {}", words(SEED_LIMIT as f64)) });
    }
}

/// A run's duration, one minute to seven days of simtime: a version's, an override's and the endpoint body's.
pub(crate) fn duration(found: &mut Vec<Violation>, pointer: &str, duration: SimDuration) {
    let ms = duration.as_ms() as f64;
    if !(60_000.0..=604_800_000.0).contains(&ms) {
        let rule = format!("takes one minute to seven days of simtime; this is {} ms", words(ms));
        found.push(Violation { pointer: pointer.to_owned(), rule });
    }
}

/// A report window inside the duration, its ends on metrics-slice ends, its name at most 40 characters.
fn window(pointer: &str, window: &ReportWindow, duration: SimDuration) -> Vec<Violation> {
    let mut found = Vec::new();
    let (from, end) = (window.from_sim_ms.as_ms(), duration.as_ms());
    let slice = |ms: u64| ms.is_multiple_of(SLICE_MS);
    if from > end || !slice(from) {
        let rule = format!("starts on a 10 s slice end inside the run's {} ms; this is {from} ms", words(end as f64));
        found.push(Violation { pointer: format!("{pointer}/from_sim_ms"), rule });
    }
    if let Some(to) = window.to_sim_ms.map(SimTime::as_ms)
        && (to <= from || to > end || !slice(to))
    {
        let rule = format!("ends on a 10 s slice end after the start and inside the run; this is {to} ms");
        found.push(Violation { pointer: format!("{pointer}/to_sim_ms"), rule });
    }
    if window.before_name.chars().count() > 40 {
        found.push(Violation {
            pointer: format!("{pointer}/before_name"),
            rule: "takes at most 40 characters".to_owned(),
        });
    }
    found
}

/// A number as the checks word it: whole numbers grouped by thousands, others as written.
fn words(value: f64) -> String {
    if value.fract() != 0.0 || !value.is_finite() || value.abs() >= 1e15 {
        return value.to_string();
    }
    let digits = (value.abs() as u64).to_string();
    let mut grouped = String::new();
    for (n, digit) in digits.chars().enumerate() {
        if n > 0 && (digits.len() - n).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    if value < 0.0 { format!("-{grouped}") } else { grouped }
}
