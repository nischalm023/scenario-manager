//! Spec 07 section 3.2's checks over a version's content: the stated defaults pass; each range and each rule that
//! joins fields refuses, naming the field by its pointer.

use desunits::{SimDuration, SimTime};
use scenario_spec::{Violation, stated_defaults, validate, validate_overrides};
use studio_protocol::{OrdersPerHour, Release, ReportWindow, RunOverrides, Seed};

fn pointers(found: Vec<Violation>) -> Vec<String> {
    found.into_iter().map(|violation| violation.pointer).collect()
}

#[test]
fn the_stated_defaults_pass_every_check() {
    assert_eq!(validate(&stated_defaults()), []);
}

#[test]
fn ranges_and_joining_rules_refuse_naming_the_field() {
    let mut content = stated_defaults();
    content.workload.generator.orders_per_hour = OrdersPerHour(0.0);
    content.workload.generator.release = Release::Waves;
    content.run_controls.seed = Seed(1 << 53);
    content.run_controls.report_window = ReportWindow {
        from_sim_ms: SimTime::ms(15_000),
        to_sim_ms: Some(SimTime::ms(10_000)),
        before_name: "x".repeat(41),
    };
    assert_eq!(
        pointers(validate(&content)),
        [
            "/workload/generator/orders_per_hour",
            "/workload/generator/wave_every_sim_ms",
            "/run_controls/seed",
            "/run_controls/report_window/from_sim_ms",
            "/run_controls/report_window/to_sim_ms",
            "/run_controls/report_window/before_name",
        ]
    );
    let mut kept = stated_defaults();
    kept.workload.generator.wave_every_sim_ms = Some(SimDuration::ms(600_000));
    assert_eq!(validate(&kept), [], "another release keeps an interval, which only waves read");
}

#[test]
fn an_overridden_duration_keeps_the_report_window_inside_it() {
    let mut content = stated_defaults();
    content.run_controls.report_window.from_sim_ms = SimTime::ms(1_800_000);
    let shorter = RunOverrides { duration_sim_ms: Some(SimDuration::ms(900_000)), report_window: None };
    assert_eq!(pointers(validate_overrides(&content, &shorter)), ["/run_controls/report_window/from_sim_ms"]);
    let too_short = RunOverrides { duration_sim_ms: Some(SimDuration::ms(1_000)), report_window: None };
    assert!(
        pointers(validate_overrides(&stated_defaults(), &too_short))
            .contains(&"/run_overrides/duration_sim_ms".to_owned())
    );
}
